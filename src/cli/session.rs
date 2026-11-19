//! Native storage for standalone `aitd` sessions and project backups.
//!
//! Each invocation identifies its parent shell through the process start time
//! and boot ID, then holds an exclusive lock while it reads or updates that
//! shell's persisted authoring project.

use crate::EditorProject;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

const BACKUP_CAP_BYTES: u64 = 1_000_000_000;
const BACKUP_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionRecord {
    loaded_path: Option<PathBuf>,
    working: Option<String>,
    source_bytes: Option<String>,
    saved_project_bytes: Option<String>,
}

/// A lock-protected working project shared by `aitd` commands from one shell.
pub struct Session {
    _lock: File,
    dir: PathBuf,
    project: Option<EditorProject>,
    loaded_path: Option<PathBuf>,
    source_bytes: Option<Vec<u8>>,
    saved_project_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    pub loaded: bool,
    pub path: Option<PathBuf>,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SaveResult {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SaveError {
    pub code: &'static str,
    pub message: String,
}

impl From<String> for SaveError {
    fn from(message: String) -> Self {
        Self {
            code: "SAVE_FAILED",
            message,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub created_at: u64,
    pub original_path: PathBuf,
    pub backup_path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupMetadata {
    created_at: u64,
    original_path: PathBuf,
    size: u64,
}

impl Session {
    /// Open and lock this parent shell's persistent temporary session.
    pub fn open() -> Result<Self, String> {
        let uid = storage_user_key();
        let parent = parent_process_id()?;
        let (start, boot) = process_identity(parent)?;
        let root = std::env::temp_dir().join(format!("aitd-sessions-{uid}"));
        create_private_dir(&root)?;
        let dir = root.join(format!("{parent}-{start}-{boot}"));
        create_private_dir(&dir)?;
        let lock = lock_file(&dir.join("session.lock"))?;
        let record_path = dir.join("session.json");
        let (project, loaded_path, source_bytes, saved_project_bytes) = if record_path.exists() {
            let record: SessionRecord = read_json(&record_path)?;
            if let Some(bytes) = decode_session_bytes(record.working, "working project")? {
                let project = EditorProject::load_project(bytes)
                    .map_err(|e| format!("Cannot restore session project: {e}"))?;
                (
                    Some(project),
                    record.loaded_path,
                    decode_session_bytes(record.source_bytes, "source baseline")?,
                    decode_session_bytes(record.saved_project_bytes, "save baseline")?,
                )
            } else {
                (None, record.loaded_path, None, None)
            }
        } else {
            (None, None, None, None)
        };
        Ok(Self {
            _lock: lock,
            dir,
            project,
            loaded_path,
            source_bytes,
            saved_project_bytes,
        })
    }

    /// Return the loaded project, if any.
    pub fn project(&self) -> Result<&EditorProject, String> {
        self.project
            .as_ref()
            .ok_or_else(|| "No project is loaded".to_owned())
    }

    /// Mutate the project atomically: errors and failed persistence discard the candidate.
    pub fn update<T>(
        &mut self,
        edit: impl FnOnce(&mut EditorProject) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut candidate = self.project()?.clone();
        let output = edit(&mut candidate)?;
        let bytes = candidate
            .save_project()
            .map_err(|e| format!("Cannot serialize project: {e}"))?;
        self.write_record(Some(bytes))?;
        self.project = Some(candidate);
        Ok(output)
    }

    /// Persist the current project after edits made through `project_mut`.
    /// Prefer `update` where possible because it rolls back failed writes.
    pub fn persist(&mut self) -> Result<(), String> {
        let bytes = self
            .project()?
            .save_project()
            .map_err(|e| format!("Cannot serialize project: {e}"))?;
        self.write_record(Some(bytes))
    }

    /// Access the in-memory project for CLI commands that perform one edit batch.
    pub fn project_mut(&mut self) -> Result<&mut EditorProject, String> {
        self.project
            .as_mut()
            .ok_or_else(|| "No project is loaded".to_owned())
    }

    /// Load a project from disk. Dirty projects require explicit discard.
    pub fn load(&mut self, path: &Path, discard: bool) -> Result<(), String> {
        self.require_discard(discard)?;
        let path = absolute_path(path)?;
        let bytes =
            fs::read(&path).map_err(|e| format!("Cannot read project {}: {e}", path.display()))?;
        let project = EditorProject::load_project(bytes.clone())?;
        project.mark_saved();
        let working = project.save_project().map_err(|e| e.to_string())?;
        self.write_record_values(
            Some(working.clone()),
            Some(bytes.clone()),
            Some(working.clone()),
            Some(path.clone()),
        )?;
        self.project = Some(project);
        self.loaded_path = Some(path);
        self.source_bytes = Some(bytes);
        self.saved_project_bytes = Some(working);
        Ok(())
    }

    /// Start a new empty project. Dirty projects require explicit discard.
    pub fn new_project(&mut self, discard: bool) -> Result<(), String> {
        self.require_discard(discard)?;
        let project = EditorProject::default();
        let bytes = project
            .save_project()
            .map_err(|e| format!("Cannot serialize project: {e}"))?;
        self.write_record_values(Some(bytes.clone()), None, Some(bytes.clone()), None)?;
        self.project = Some(project);
        self.loaded_path = None;
        self.source_bytes = None;
        self.saved_project_bytes = Some(bytes);
        Ok(())
    }

    /// Close the current project. Dirty projects require explicit discard.
    pub fn close(&mut self, discard: bool) -> Result<(), String> {
        self.require_discard(discard)?;
        self.write_record_values(None, None, None, None)?;
        self.project = None;
        self.loaded_path = None;
        self.source_bytes = None;
        self.saved_project_bytes = None;
        Ok(())
    }

    /// Describe the current session state.
    pub fn status(&self) -> SessionStatus {
        SessionStatus {
            loaded: self.project.is_some(),
            path: self.loaded_path.clone(),
            dirty: self.is_dirty(),
        }
    }

    /// Save the authoring project atomically, optionally validating before replacement.
    /// The validation callback should return an error only for blocking diagnostics.
    pub fn save(
        &mut self,
        path: &Path,
        force: bool,
        skip_validation: bool,
        validate: impl FnOnce(&EditorProject) -> Result<(), String>,
    ) -> Result<SaveResult, SaveError> {
        // Serialize compare, backup, and replacement across independent shell sessions.
        let _save_lock = save_lock()?;
        let project = self.project()?;
        if !skip_validation {
            validate(project).map_err(|message| SaveError {
                code: "VALIDATION_FAILED",
                message,
            })?;
        }
        let bytes = project
            .save_project()
            .map_err(|e| format!("Cannot serialize project: {e}"))?;
        let target = absolute_path(path)?;
        let same_loaded = self
            .loaded_path
            .as_ref()
            .is_some_and(|loaded| loaded == &target);
        let existed = target.exists();
        let old_bytes = if existed {
            Some(
                fs::read(&target)
                    .map_err(|e| format!("Cannot read existing project for backup: {e}"))?,
            )
        } else {
            None
        };
        if same_loaded {
            if self.source_bytes.as_deref() != old_bytes.as_deref() {
                return Err(SaveError {
                    code: "SOURCE_CHANGED",
                    message: format!("Loaded project changed outside aitd: {}", target.display()),
                });
            }
        }
        if existed && !same_loaded && !force {
            return Err(SaveError::from(format!(
                "{} already exists; pass --force to overwrite it",
                target.display()
            )));
        }
        let backup = old_bytes
            .as_ref()
            .map(|old| create_backup(&target, old))
            .transpose()?;
        if let Some(expected) = old_bytes.as_ref() {
            let current = fs::read(&target)
                .map_err(|e| format!("Cannot recheck project before save: {e}"))?;
            if &current != expected {
                return Err(SaveError {
                    code: "SOURCE_CHANGED",
                    message: format!(
                        "Project changed while aitd was preparing the save: {}",
                        target.display()
                    ),
                });
            }
        } else if target.exists() {
            return Err(SaveError::from(format!(
                "Destination appeared while aitd was preparing the save: {}",
                target.display()
            )));
        }
        atomic_write(&target, &bytes)?;
        if let Err(error) = self.write_record_values(
            Some(bytes.clone()),
            Some(bytes.clone()),
            Some(bytes.clone()),
            Some(target.clone()),
        ) {
            let rollback = match old_bytes.as_ref() {
                Some(old) => atomic_write(&target, old),
                None => fs::remove_file(&target)
                    .map_err(|e| format!("Cannot remove failed save {}: {e}", target.display())),
            };
            return Err(SaveError::from(match rollback {
                Ok(()) => error,
                Err(rollback_error) => {
                    format!("{error}; additionally could not restore destination: {rollback_error}")
                }
            }));
        }
        self.project
            .as_ref()
            .expect("project checked above")
            .mark_saved();
        self.loaded_path = Some(target.clone());
        self.source_bytes = Some(bytes.clone());
        self.saved_project_bytes = Some(bytes);
        Ok(SaveResult {
            path: target,
            backup,
        })
    }

    /// List retained backups, optionally filtering by original project path.
    pub fn backups_list(project: Option<&Path>) -> Result<Vec<BackupInfo>, String> {
        list_backups(project)
    }

    fn require_discard(&self, discard: bool) -> Result<(), String> {
        if self.is_dirty() && !discard {
            Err(
                "The loaded project has unsaved changes; pass --discard to replace or close it"
                    .into(),
            )
        } else {
            Ok(())
        }
    }

    fn is_dirty(&self) -> bool {
        let Some(project) = self.project.as_ref() else {
            return false;
        };
        let Ok(current) = project.save_project() else {
            return true;
        };
        self.saved_project_bytes.as_deref() != Some(current.as_slice())
    }

    fn write_record(&self, working: Option<Vec<u8>>) -> Result<(), String> {
        self.write_record_values(
            working,
            self.source_bytes.clone(),
            self.saved_project_bytes.clone(),
            self.loaded_path.clone(),
        )
    }

    fn write_record_values(
        &self,
        working: Option<Vec<u8>>,
        source_bytes: Option<Vec<u8>>,
        saved_project_bytes: Option<Vec<u8>>,
        loaded_path: Option<PathBuf>,
    ) -> Result<(), String> {
        let record = SessionRecord {
            loaded_path,
            working: working.map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
            source_bytes: source_bytes
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
            saved_project_bytes: saved_project_bytes
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        };
        let bytes =
            serde_json::to_vec(&record).map_err(|e| format!("Cannot encode session state: {e}"))?;
        atomic_write(&self.dir.join("session.json"), &bytes)
    }
}

fn process_identity(pid: u32) -> Result<(u64, String), String> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|e| format!("Cannot identify calling shell process: {e}"))?;
        let tail = stat
            .rsplit_once(") ")
            .map(|(_, tail)| tail)
            .ok_or_else(|| "Cannot parse calling shell process identity".to_owned())?;
        let start = tail
            .split_whitespace()
            .nth(19)
            .ok_or_else(|| "Cannot read calling shell process start time".to_owned())?
            .parse()
            .map_err(|_| "Invalid calling shell process start time".to_owned())?;
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|e| format!("Cannot read system boot identity: {e}"))?
            .trim()
            .to_owned();
        Ok((start, boot))
    }
    #[cfg(target_os = "macos")]
    {
        let start = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "lstart="])
            .output()
            .map_err(|e| format!("Cannot identify calling shell process: {e}"))?;
        if !start.status.success() {
            return Err("Calling shell process no longer exists".into());
        }
        let boot = std::process::Command::new("sysctl")
            .args(["-n", "kern.boottime"])
            .output()
            .map_err(|e| format!("Cannot read system boot identity: {e}"))?;
        if !boot.status.success() {
            return Err("Cannot read system boot identity".into());
        }
        Ok((
            stable_hash(String::from_utf8_lossy(&start.stdout).trim().as_bytes()),
            String::from_utf8_lossy(&boot.stdout).trim().to_owned(),
        ))
    }
    #[cfg(target_os = "windows")]
    {
        windows_process_identity(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = pid;
        Err("Unsupported process identity platform".into())
    }
}

fn parent_process_id() -> Result<u32, String> {
    #[cfg(target_os = "linux")]
    {
        Ok(unsafe { libc::getppid() as u32 })
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        Ok(unsafe { libc::getppid() as u32 })
    }
    #[cfg(target_os = "windows")]
    {
        windows_parent_pid()
    }
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct WinFileTime {
    low: u32,
    high: u32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct WinProcessEntry {
    size: u32,
    usage: u32,
    process_id: u32,
    heap_id: usize,
    module_id: u32,
    threads: u32,
    parent_id: u32,
    priority: i32,
    flags: u32,
    exe: [u16; 260],
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut std::ffi::c_void;
    fn Process32FirstW(snapshot: *mut std::ffi::c_void, entry: *mut WinProcessEntry) -> i32;
    fn Process32NextW(snapshot: *mut std::ffi::c_void, entry: *mut WinProcessEntry) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> *mut std::ffi::c_void;
    fn GetProcessTimes(
        handle: *mut std::ffi::c_void,
        created: *mut WinFileTime,
        exited: *mut WinFileTime,
        kernel: *mut WinFileTime,
        user: *mut WinFileTime,
    ) -> i32;
    fn GetCurrentProcessId() -> u32;
}

#[cfg(target_os = "windows")]
fn windows_parent_pid() -> Result<u32, String> {
    let current = unsafe { GetCurrentProcessId() };
    let snapshot = unsafe { CreateToolhelp32Snapshot(0x2, 0) };
    if snapshot as isize == -1 {
        return Err("Cannot inspect Windows process list".into());
    }
    let mut entry = WinProcessEntry {
        size: std::mem::size_of::<WinProcessEntry>() as u32,
        usage: 0,
        process_id: 0,
        heap_id: 0,
        module_id: 0,
        threads: 0,
        parent_id: 0,
        priority: 0,
        flags: 0,
        exe: [0; 260],
    };
    let mut found = None;
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while ok {
        if entry.process_id == current {
            found = Some(entry.parent_id);
            break;
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    found.ok_or_else(|| "Cannot identify calling shell process".into())
}

#[cfg(target_os = "windows")]
fn windows_process_identity(pid: u32) -> Result<(u64, String), String> {
    let process = unsafe { OpenProcess(0x1000, 0, pid) };
    if process.is_null() {
        return Err("Cannot identify calling shell process start time".into());
    }
    let mut created = WinFileTime { low: 0, high: 0 };
    let mut exited = WinFileTime { low: 0, high: 0 };
    let mut kernel = WinFileTime { low: 0, high: 0 };
    let mut user = WinFileTime { low: 0, high: 0 };
    let ok =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } != 0;
    unsafe {
        CloseHandle(process);
    }
    if !ok {
        return Err("Cannot read calling shell process start time".into());
    }
    let start = ((created.high as u64) << 32) | created.low as u64;
    // Process creation FILETIME is globally unique across system boots.
    Ok((start, "windows".into()))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}
#[cfg(unix)]
fn storage_user_key() -> String {
    current_uid().to_string()
}

#[cfg(windows)]
fn storage_user_key() -> String {
    // Windows normally provides per-user TEMP, but retain isolation when users
    // configure a shared temporary directory too.
    let identity = ["USERPROFILE", "USERDOMAIN", "USERNAME"].map(|key| std::env::var_os(key));
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(target_os = "macos")]
fn stable_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "Session storage path is not a private directory: {}",
                    path.display()
                ));
            }
            #[cfg(unix)]
            {
                if metadata.uid() != current_uid() {
                    return Err(format!(
                        "Session storage directory has the wrong owner: {}",
                        path.display()
                    ));
                }
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(format!(
                        "Session storage directory is accessible by other users: {}",
                        path.display()
                    ));
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(path) {
                Ok(()) => create_private_dir(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    create_private_dir(path)
                }
                Err(error) => Err(format!("Cannot create {}: {error}", path.display())),
            }
        }
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
    }
}

fn lock_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    set_private_file_mode(&mut options);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options
        .open(path)
        .map_err(|e| format!("Cannot open lock {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Cannot inspect lock {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "Lock path is not a regular file: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    if metadata.uid() != current_uid() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "Lock file is not private to this user: {}",
            path.display()
        ));
    }
    file.lock()
        .map_err(|e| format!("Cannot lock this aitd session: {e}"))?;
    Ok(file)
}

#[cfg(unix)]
fn set_private_file_mode(options: &mut OpenOptions) {
    options.mode(0o600);
}
#[cfg(not(unix))]
fn set_private_file_mode(_options: &mut OpenOptions) {}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Cannot read current directory: {e}"))?
            .join(path)
    };
    // Canonicalize only the parent so new destination files are supported.
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| "Project path must name a file".to_owned())?;
    if path.exists() {
        return fs::canonicalize(&path).map_err(|e| format!("Cannot resolve project path: {e}"));
    }
    Ok(fs::canonicalize(parent)
        .map_err(|e| format!("Cannot resolve project directory: {e}"))?
        .join(name))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| "Output path must name a file".to_owned())?
        .to_string_lossy();
    let mut temp = None;
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".{name}.aitd-{}-{attempt}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        set_private_file_mode(&mut options);
        match options.open(&candidate) {
            Ok(file) => {
                temp = Some((candidate, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot stage {}: {e}", path.display())),
        }
    }
    let (temp_path, mut file) =
        temp.ok_or_else(|| "Cannot allocate temporary output file".to_owned())?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|e| format!("Cannot write staged file: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Cannot flush staged file: {e}"))?;
        fs::rename(&temp_path, path)
            .map_err(|e| format!("Cannot replace {}: {e}", path.display()))?;
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Cannot parse {}: {e}", path.display()))
}

fn decode_session_bytes(value: Option<String>, label: &str) -> Result<Option<Vec<u8>>, String> {
    value
        .map(|encoded| {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| format!("Cannot decode session {label}: {e}"))
        })
        .transpose()
}

fn backup_root() -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join(format!("aitd-backups-{}", storage_user_key()));
    create_private_dir(&root)?;
    Ok(root)
}

fn backup_lock(root: &Path) -> Result<File, String> {
    lock_file(&root.join("backups.lock"))
}

fn save_lock() -> Result<File, String> {
    let root = backup_root()?;
    lock_file(&root.join("saves.lock"))
}

fn backup_entries(root: &Path) -> Result<Vec<(BackupInfo, PathBuf)>, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| format!("Cannot list backups: {e}"))? {
        let entry = entry.map_err(|e| format!("Cannot read backup directory: {e}"))?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json")
            || path.file_name().is_some_and(|n| n == "backups.lock")
        {
            continue;
        }
        let metadata: BackupMetadata = read_json(&path)?;
        let backup_path = path.with_extension("aitp");
        let size = fs::metadata(&backup_path)
            .map_err(|e| format!("Missing backup {}: {e}", backup_path.display()))?
            .len();
        entries.push((
            BackupInfo {
                created_at: metadata.created_at,
                original_path: metadata.original_path,
                backup_path,
                size,
            },
            path,
        ));
    }
    entries.sort_by(|(left, _), (right, _)| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.backup_path.cmp(&right.backup_path))
    });
    Ok(entries)
}

fn remove_backup(info: &BackupInfo, metadata: &Path) -> Result<(), String> {
    fs::remove_file(&info.backup_path)
        .map_err(|e| format!("Cannot remove backup {}: {e}", info.backup_path.display()))?;
    fs::remove_file(metadata)
        .map_err(|e| format!("Cannot remove backup metadata {}: {e}", metadata.display()))
}

fn create_backup(original: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let root = backup_root()?;
    let _lock = backup_lock(&root)?;
    create_backup_locked(
        &root,
        original,
        bytes,
        BACKUP_CAP_BYTES,
        BACKUP_TTL_SECS,
        now_secs(),
    )
}

fn create_backup_locked(
    root: &Path,
    original: &Path,
    bytes: &[u8],
    cap_bytes: u64,
    ttl_secs: u64,
    now: u64,
) -> Result<PathBuf, String> {
    clean_orphaned_backups(root)?;
    let mut entries = backup_entries(root)?;
    for (info, metadata) in entries.clone() {
        if now.saturating_sub(info.created_at) >= ttl_secs {
            remove_backup(&info, &metadata)?;
        }
    }
    entries = backup_entries(root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = format!("{stamp}-{}", std::process::id());
    let backup_path = root.join(format!("{stem}.aitp"));
    let metadata_path = root.join(format!("{stem}.json"));
    let metadata = BackupMetadata {
        created_at: now,
        original_path: original.to_path_buf(),
        size: bytes.len() as u64,
    };
    let metadata_bytes =
        serde_json::to_vec(&metadata).map_err(|e| format!("Cannot encode backup metadata: {e}"))?;
    let added = bytes.len() as u64 + metadata_bytes.len() as u64;
    if added > cap_bytes {
        return Err("Project is too large to back up within the 1 GB backup limit".into());
    }
    let mut used = entries.iter().try_fold(0u64, |total, (info, meta)| {
        Ok::<_, String>(
            total.saturating_add(info.size).saturating_add(
                fs::metadata(meta)
                    .map_err(|e| format!("Cannot inspect backup metadata: {e}"))?
                    .len(),
            ),
        )
    })?;
    while used + added > cap_bytes {
        let Some((oldest, metadata)) = entries.first().cloned() else {
            return Err("Cannot make room in the backup store".into());
        };
        let metadata_size = fs::metadata(&metadata)
            .map_err(|e| format!("Cannot inspect backup metadata: {e}"))?
            .len();
        remove_backup(&oldest, &metadata)?;
        used = used
            .saturating_sub(oldest.size)
            .saturating_sub(metadata_size);
        entries.remove(0);
    }
    atomic_write(&backup_path, bytes)?;
    if let Err(e) = atomic_write(&metadata_path, &metadata_bytes) {
        let _ = fs::remove_file(&backup_path);
        return Err(e);
    }
    Ok(backup_path)
}

fn clean_orphaned_backups(root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|e| format!("Cannot inspect backup store: {e}"))? {
        let entry = entry.map_err(|e| format!("Cannot inspect backup store entry: {e}"))?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.extension().is_some_and(|ext| ext == "tmp") && name.contains(".aitd-") {
            fs::remove_file(&path)
                .map_err(|e| format!("Cannot clean staged backup {}: {e}", path.display()))?;
        } else if path.extension().is_some_and(|ext| ext == "aitp")
            && !path.with_extension("json").exists()
        {
            fs::remove_file(&path)
                .map_err(|e| format!("Cannot clean orphan backup {}: {e}", path.display()))?;
        } else if path.extension().is_some_and(|ext| ext == "json")
            && !path.with_extension("aitp").exists()
        {
            fs::remove_file(&path)
                .map_err(|e| format!("Cannot clean orphan metadata {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

fn list_backups(project: Option<&Path>) -> Result<Vec<BackupInfo>, String> {
    let root = backup_root()?;
    let _lock = backup_lock(&root)?;
    let filter = project.map(absolute_path).transpose()?;
    Ok(backup_entries(&root)?
        .into_iter()
        .map(|(info, _)| info)
        .filter(|info| {
            filter
                .as_ref()
                .is_none_or(|path| path == &info.original_path)
        })
        .collect())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
