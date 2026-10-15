//! Authenticated loopback bridge. Project reads and edits run on the editor thread.
use crate::{
    ai::{automation_service, ProjectRevision},
    EditorProject,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread,
    time::Duration,
};
const REQUEST_CAPACITY: usize = 32;
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
const REQUEST_PENDING: u8 = 0;
const REQUEST_CLAIMED: u8 = 1;
const REQUEST_EXPIRED: u8 = 2;
pub use super::protocol::{
    AutomationAction, AutomationClientError, AutomationRequest, AutomationResponse, WireError,
};
type WireResponse = AutomationResponse;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub instance_id: String,
    pub address: String,
    pub token: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct WireRequest {
    token: String,
    #[serde(flatten)]
    request: AutomationRequest,
}
struct Incoming {
    request: AutomationRequest,
    reply: SyncSender<WireResponse>,
    state: Arc<AtomicU8>,
}
impl Incoming {
    fn claim(&self) -> bool {
        self.state
            .compare_exchange(
                REQUEST_PENDING,
                REQUEST_CLAIMED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}
/// UI-thread owner of the authenticated command queue.
pub struct EditorAutomationBridge {
    editor_instance_id: String,
    project_instance_id: String,
    observed_revision: Option<ProjectRevision>,
    current_project_path: Option<PathBuf>,
    original_source_path: Option<PathBuf>,
    display_name: String,
    observed_dirty: Option<bool>,
    runtime_path: PathBuf,
    incoming: Receiver<Incoming>,
    listener_stop: Arc<AtomicBool>,
    status: String,
    last_error: Option<String>,
}
impl EditorAutomationBridge {
    pub fn start(ctx: eframe::egui::Context) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let editor_instance_id = uuid::Uuid::new_v4().to_string();
        let runtime_path = runtime_dir()?.join(format!("{editor_instance_id}.json"));
        let runtime = RuntimeRecord {
            instance_id: editor_instance_id.clone(),
            address: listener
                .local_addr()
                .map_err(|e| e.to_string())?
                .to_string(),
            token: uuid::Uuid::new_v4().to_string(),
        };
        write_runtime(&runtime_path, &runtime)?;
        let (sender, incoming) = mpsc::sync_channel(REQUEST_CAPACITY);
        let listener_stop = Arc::new(AtomicBool::new(false));
        start_listener(listener, sender, runtime.token, listener_stop.clone(), ctx);
        Ok(Self {
            editor_instance_id,
            project_instance_id: uuid::Uuid::new_v4().to_string(),
            observed_revision: None,
            current_project_path: None,
            original_source_path: None,
            display_name: String::new(),
            observed_dirty: None,
            runtime_path,
            incoming,
            listener_stop,
            status: "Waiting for an automation client".into(),
            last_error: None,
        })
    }
    /// Claim bounded queued requests before executing them on the editor thread.
    /// Requests that expired while queued are never executed.
    pub fn poll(&mut self, mut project: Option<&mut EditorProject>) {
        self.observed_revision = project.as_deref().map(EditorProject::revision);
        self.observed_dirty = project.as_deref().map(EditorProject::is_dirty);
        for _ in 0..REQUEST_CAPACITY {
            let Ok(incoming) = self.incoming.try_recv() else {
                break;
            };
            if !incoming.claim() {
                continue;
            }
            let request = &incoming.request;
            let revision = project.as_deref().map(EditorProject::revision);
            let response = if request.action == AutomationAction::GetProjectStatus {
                self.status_response(revision)
            } else if project.is_none() {
                self.error_response(
                    "PROJECT_NOT_OPEN",
                    "Open a project in the editor before using automation.",
                    request.revision,
                    revision,
                )
            } else if request.project_instance_id.as_deref()
                != Some(self.project_instance_id.as_str())
            {
                self.error_response(
                    "PROJECT_INSTANCE_MISMATCH",
                    "The open project changed; inspect it again before retrying.",
                    request.revision,
                    revision,
                )
            } else if request.revision != revision {
                self.error_response(
                    "PROJECT_REVISION_MISMATCH",
                    "Project changed; inspect it again before retrying.",
                    request.revision,
                    revision,
                )
            } else {
                let project = project.as_deref_mut().unwrap();
                let result = self.execute(project, request);
                self.observed_revision = Some(project.revision());
                self.observed_dirty = Some(project.is_dirty());
                match result {
                    Ok(value) => {
                        self.status = "Automation command completed".into();
                        self.last_error = None;
                        self.success_response(self.observed_revision, value)
                    }
                    Err(error) => self.error_response(
                        error.code,
                        error.message,
                        request.revision,
                        self.observed_revision,
                    ),
                }
            };
            let _ = incoming.reply.try_send(response);
        }
    }
    fn execute(
        &self,
        project: &mut EditorProject,
        request: &AutomationRequest,
    ) -> Result<Value, automation_service::DomainError> {
        let failure = |message: String| automation_service::DomainError {
            code: "VALIDATION_FAILED",
            message,
        };
        match request.action {
            AutomationAction::EditProject => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct EditRequest {
                    actions: Vec<crate::editing::EditAction>,
                    #[serde(default)]
                    check: bool,
                }
                let args: EditRequest = serde_json::from_value(request.args.clone())
                    .map_err(|e| failure(e.to_string()))?;
                if args.actions.is_empty() || args.actions.len() > crate::editing::MAX_EDIT_ACTIONS
                {
                    return Err(automation_service::DomainError {
                        code: "INVALID_ARGUMENTS",
                        message: format!(
                            "A batch requires 1..{} edit actions",
                            crate::editing::MAX_EDIT_ACTIONS
                        ),
                    });
                }
                let before: std::collections::HashSet<_> = project
                    .get_pool()
                    .objects()
                    .iter()
                    .map(|o| o.id().value())
                    .collect();
                let applied = project
                    .edit_batch(
                        "CLI edit",
                        |scratch| {
                            for action in args.actions {
                                crate::editing::apply(scratch, action)?;
                            }
                            Ok(())
                        },
                        args.check,
                    )
                    .map_err(failure)?;
                let allocated: Vec<_> = project
                    .get_pool()
                    .objects()
                    .iter()
                    .map(|o| o.id().value())
                    .filter(|id| !before.contains(id))
                    .collect();
                Ok(
                    json!({"transaction_id":applied.as_ref().map(|a|&a.transaction_id),"revision":project.revision(),"changed_ids":applied.as_ref().map(|a|a.affected_objects.as_slice()).unwrap_or_default(),"allocated_ids":allocated,"check":args.check}),
                )
            }
            AutomationAction::ExportArtifacts => {
                let artifacts = crate::ai::export::artifacts(
                    project,
                    request.args["format"].as_str().unwrap_or("bundle"),
                )
                .map_err(failure)?;
                let encode = |bytes: &[u8]| {
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
                };
                let mut encoded = serde_json::Map::new();
                if let Some(bytes) = artifacts.iop {
                    encoded.insert("iop".into(), json!(encode(&bytes)));
                }
                if let Some(bytes) = artifacts.header {
                    encoded.insert("header".into(), json!(encode(&bytes)));
                }
                Ok(json!({"artifacts":encoded,"vt_version":artifacts.vt_version}))
            }
            _ => automation_service::execute(project, request.action, request.args.clone()),
        }
    }
    /// Invalidate client identities whenever the editor replaces or closes its project.
    pub fn project_changed(&mut self) {
        self.project_instance_id = uuid::Uuid::new_v4().to_string();
        self.observed_revision = None;
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    pub fn project_instance_id(&self) -> &str {
        &self.project_instance_id
    }
    pub fn editor_instance_id(&self) -> &str {
        &self.editor_instance_id
    }
    pub fn set_project_location(
        &mut self,
        current: Option<&std::path::Path>,
        original: Option<&std::path::Path>,
        display_name: &str,
    ) {
        self.current_project_path = current.map(std::path::Path::to_path_buf);
        self.original_source_path = original.map(std::path::Path::to_path_buf);
        self.display_name = display_name.into();
    }
    fn success_response(&self, revision: Option<u64>, value: Value) -> WireResponse {
        WireResponse {
            success: true,
            editor_instance_id: self.editor_instance_id.clone(),
            project_instance_id: self.project_instance_id.clone(),
            revision,
            result: Some(value),
            error: None,
        }
    }
    fn error_response(
        &mut self,
        code: &str,
        message: impl Into<String>,
        expected_revision: Option<u64>,
        current_revision: Option<u64>,
    ) -> WireResponse {
        let message = message.into();
        self.last_error = Some(message.clone());
        self.status = message.clone();
        WireResponse {
            success: false,
            editor_instance_id: self.editor_instance_id.clone(),
            project_instance_id: self.project_instance_id.clone(),
            revision: current_revision,
            result: None,
            error: Some(WireError {
                code: code.into(),
                message,
                expected_revision,
                current_revision,
            }),
        }
    }
    fn status_response(&self, revision: Option<u64>) -> WireResponse {
        self.success_response(revision,json!({"project_open":revision.is_some(),"current_project_path":if revision.is_some(){self.current_project_path.as_deref()}else{None},"original_source_path":if revision.is_some(){self.original_source_path.as_deref()}else{None},"display_name":revision.map(|_|self.display_name.as_str()),"dirty":self.observed_dirty,"revision":revision}))
    }
}
impl Drop for EditorAutomationBridge {
    fn drop(&mut self) {
        self.listener_stop.store(true, Ordering::Relaxed);
        let _ = fs::remove_file(&self.runtime_path);
    }
}
fn start_listener(
    listener: TcpListener,
    incoming: SyncSender<Incoming>,
    token: String,
    stop: Arc<AtomicBool>,
    context: eframe::egui::Context,
) {
    thread::Builder::new()
        .name("aitp-automation-listener".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let incoming = incoming.clone();
                        let token = token.clone();
                        let context = context.clone();
                        let _ = thread::Builder::new()
                            .name("aitp-automation-request".into())
                            .spawn(move || handle_connection(stream, incoming, token, context));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => thread::sleep(Duration::from_millis(100)),
                }
            }
        })
        .expect("could not spawn automation listener");
}

fn handle_connection(
    mut stream: TcpStream,
    incoming: SyncSender<Incoming>,
    token: String,
    context: eframe::egui::Context,
) {
    let _ = stream.set_read_timeout(Some(TOOL_TIMEOUT));
    let _ = stream.set_write_timeout(Some(TOOL_TIMEOUT));
    let request = {
        let mut line = String::new();
        let mut reader = BufReader::new(&stream).take(1024 * 1024 + 1);
        match reader.read_line(&mut line) {
            Ok(n) if n > 0 && n <= 1024 * 1024 => serde_json::from_str::<WireRequest>(&line),
            Ok(_) => Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "empty or oversized request",
            ))),
            Err(e) => Err(serde_json::Error::io(e)),
        }
    };
    let response = match request {
        Err(e) => WireResponse {
            success: false,
            editor_instance_id: String::new(),
            project_instance_id: String::new(),
            revision: None,
            result: None,
            error: Some(WireError {
                code: "INVALID_BRIDGE_REQUEST".into(),
                message: format!("Invalid bridge request: {e}"),
                expected_revision: None,
                current_revision: None,
            }),
        },
        Ok(request) if !constant_time_eq(&request.token, &token) => WireResponse {
            success: false,
            editor_instance_id: String::new(),
            project_instance_id: String::new(),
            revision: None,
            result: None,
            error: Some(WireError {
                code: "AUTHENTICATION_FAILED".into(),
                message: "Bridge authentication failed".into(),
                expected_revision: None,
                current_revision: None,
            }),
        },
        Ok(wire) => {
            let request = wire.request;
            let (reply_tx, reply_rx) = mpsc::sync_channel(1);
            let request_state = Arc::new(AtomicU8::new(REQUEST_PENDING));
            let queued = Incoming {
                request,
                reply: reply_tx,
                state: request_state.clone(),
            };
            match incoming.try_send(queued) {
                Ok(()) => {
                    context.request_repaint();
                    let timeout = || WireResponse {
                        success: false,
                        editor_instance_id: String::new(),
                        project_instance_id: String::new(),
                        revision: None,
                        result: None,
                        error: Some(WireError {
                            code: "EDITOR_TIMEOUT".into(),
                            message: "Timed out waiting for editor response".into(),
                            expected_revision: None,
                            current_revision: None,
                        }),
                    };
                    match reply_rx.recv_timeout(TOOL_TIMEOUT) {
                        Ok(response) => response,
                        Err(_) => {
                            if request_state
                                .compare_exchange(
                                    REQUEST_PENDING,
                                    REQUEST_EXPIRED,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                )
                                .is_ok()
                            {
                                timeout()
                            } else {
                                // A claimed request may already be running; it cannot be cancelled safely.
                                reply_rx
                                    .recv_timeout(Duration::from_secs(2))
                                    .unwrap_or_else(|_| timeout())
                            }
                        }
                    }
                }
                Err(TrySendError::Full(_)) => WireResponse {
                    success: false,
                    editor_instance_id: String::new(),
                    project_instance_id: String::new(),
                    revision: None,
                    result: None,
                    error: Some(WireError {
                        code: "EDITOR_BUSY".into(),
                        message: "Automation request queue is full; retry shortly".into(),
                        expected_revision: None,
                        current_revision: None,
                    }),
                },
                Err(TrySendError::Disconnected(_)) => WireResponse {
                    success: false,
                    editor_instance_id: String::new(),
                    project_instance_id: String::new(),
                    revision: None,
                    result: None,
                    error: Some(WireError {
                        code: "EDITOR_UNAVAILABLE".into(),
                        message: "Editor automation bridge is shutting down".into(),
                        expected_revision: None,
                        current_revision: None,
                    }),
                },
            }
        }
    };
    let response_bytes = serde_json::to_vec(&response).unwrap_or_default();
    let _ = stream.write_all(&response_bytes);
    let _ = stream.write_all(b"\n");
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    // The expected bearer is a fixed-size UUID, so bound work for unauthenticated
    // clients even if they submit an oversized token string.
    for index in 0..right.len() {
        difference |= left.as_bytes().get(index).copied().unwrap_or(0) as usize
            ^ right.as_bytes().get(index).copied().unwrap_or(0) as usize;
    }
    difference == 0
}

fn runtime_dir() -> Result<PathBuf, String> {
    #[cfg(not(unix))]
    return Err(
        "The automation bridge currently requires Unix owner-only runtime file permissions".into(),
    );

    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into());
    let safe_user: String = username
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let directory = std::env::temp_dir().join(format!(
        "ag-iso-terminal-designer-{}",
        if safe_user.is_empty() {
            "user"
        } else {
            &safe_user
        }
    ));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(format!(
                "Could not create automation runtime directory: {error}"
            ))
        }
    }
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|e| format!("Could not inspect automation runtime directory: {e}"))?;
    if !metadata.file_type().is_dir() {
        return Err("automation runtime path is not a directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Never publish a token in a directory owned by another local account.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err("automation runtime directory is owned by another user".into());
        }
    }
    set_owner_only(&directory, true)?;
    Ok(directory)
}

fn set_owner_only(path: &std::path::Path, directory: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, directory);
        return Err(
            "Owner-only automation runtime permissions are unavailable on this platform".into(),
        );
    }
    Ok(())
}

fn write_runtime(path: &std::path::Path, runtime: &RuntimeRecord) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(runtime).map_err(|e| e.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    set_owner_only(&temporary, false)?;
    fs::rename(temporary, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Locate the editor runtime file. Multiple open editor instances require explicit selection.
pub fn select_runtime(instance_id: Option<&str>) -> Result<(PathBuf, RuntimeRecord), String> {
    choose_runtime(runtime_records()?, instance_id)
}

/// List reachable editor identities without exposing their authentication tokens.
pub fn list_runtime_instances() -> Result<Vec<String>, String> {
    let mut identities: Vec<_> = runtime_records()?
        .into_iter()
        .map(|(_, record)| record.instance_id)
        .collect();
    identities.sort();
    Ok(identities)
}

fn runtime_records() -> Result<Vec<(PathBuf, RuntimeRecord)>, String> {
    let directory = runtime_dir()?;
    let mut records = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(contents) = fs::read(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_slice::<RuntimeRecord>(&contents) else {
            continue;
        };
        let reachable = record
            .address
            .parse::<std::net::SocketAddr>()
            .ok()
            .filter(|address| address.ip().is_loopback())
            .is_some_and(|address| {
                TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
            });
        if !reachable {
            let _ = fs::remove_file(path);
            continue;
        }
        records.push((path, record));
    }
    Ok(records)
}

fn choose_runtime(
    mut records: Vec<(PathBuf, RuntimeRecord)>,
    instance_id: Option<&str>,
) -> Result<(PathBuf, RuntimeRecord), String> {
    match instance_id {
        Some(id) => records
            .into_iter()
            .find(|(_, r)| r.instance_id == id)
            .ok_or_else(|| format!("No running editor instance matches '{id}'")),
        None if records.is_empty() => {
            Err("No running editor found. Open AgIsoTerminalDesigner before using aitd.".into())
        }
        None if records.len() > 1 => {
            Err("Multiple editors are running. Start with --instance <editor-instance-id>.".into())
        }
        None => Ok(records.remove(0)),
    }
}

/// Invoke one semantic automation action over the authenticated local protocol.
pub fn invoke_runtime_request(
    instance_id: Option<&str>,
    request: AutomationRequest,
) -> Result<AutomationResponse, AutomationClientError> {
    let (_, runtime) = select_runtime(instance_id).map_err(|message| {
        let code = if message.contains("Multiple editors") {
            "AMBIGUOUS_EDITOR_INSTANCE"
        } else if message.contains("No running editor") {
            "EDITOR_NOT_FOUND"
        } else {
            "EDITOR_CONNECTION_FAILED"
        };
        AutomationClientError {
            code: code.into(),
            message,
            expected_revision: request.revision,
            current_revision: None,
        }
    })?;
    let address = runtime
        .address
        .parse::<std::net::SocketAddr>()
        .map_err(|e| AutomationClientError {
            code: "EDITOR_CONNECTION_FAILED".into(),
            message: format!("Invalid editor bridge address: {e}"),
            expected_revision: request.revision,
            current_revision: None,
        })?;
    if !address.ip().is_loopback() {
        return Err(AutomationClientError {
            code: "EDITOR_CONNECTION_FAILED".into(),
            message: "Editor bridge address is not loopback".into(),
            expected_revision: request.revision,
            current_revision: None,
        });
    }
    let mut stream = TcpStream::connect(address).map_err(|e| AutomationClientError {
        code: "EDITOR_NOT_FOUND".into(),
        message: format!("Could not connect to editor bridge: {e}"),
        expected_revision: request.revision,
        current_revision: None,
    })?;
    stream
        .set_read_timeout(Some(TOOL_TIMEOUT))
        .map_err(|e| AutomationClientError {
            code: "EDITOR_CONNECTION_FAILED".into(),
            message: e.to_string(),
            expected_revision: request.revision,
            current_revision: None,
        })?;
    let wire_request = WireRequest {
        token: runtime.token,
        request,
    };
    serde_json::to_writer(&mut stream, &wire_request).map_err(|e| AutomationClientError {
        code: "INVALID_REQUEST".into(),
        message: e.to_string(),
        expected_revision: wire_request.request.revision,
        current_revision: None,
    })?;
    stream.write_all(b"\n").map_err(|e| AutomationClientError {
        code: "EDITOR_CONNECTION_FAILED".into(),
        message: e.to_string(),
        expected_revision: wire_request.request.revision,
        current_revision: None,
    })?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| AutomationClientError {
            code: "EDITOR_CONNECTION_FAILED".into(),
            message: e.to_string(),
            expected_revision: wire_request.request.revision,
            current_revision: None,
        })?;
    let response: WireResponse =
        serde_json::from_str(&line).map_err(|e| AutomationClientError {
            code: "INVALID_RESPONSE".into(),
            message: format!("Invalid editor response: {e}"),
            expected_revision: wire_request.request.revision,
            current_revision: None,
        })?;
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| WireError {
                code: "EDITOR_ERROR".into(),
                message: "Editor automation request failed".into(),
                expected_revision: wire_request.request.revision,
                current_revision: response.revision,
            })
            .into());
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        editing::{EditAction, EditActionKind},
        Operation,
    };
    use ag_iso_stack::object_pool::{ObjectPool, ObjectType};

    fn bridge() -> (EditorAutomationBridge, SyncSender<Incoming>) {
        let (sender, incoming) = mpsc::sync_channel(REQUEST_CAPACITY);
        (
            EditorAutomationBridge {
                editor_instance_id: "editor".into(),
                project_instance_id: "project".into(),
                observed_revision: None,
                current_project_path: None,
                original_source_path: None,
                display_name: "Test".into(),
                observed_dirty: None,
                runtime_path: PathBuf::new(),
                incoming,
                listener_stop: Arc::new(AtomicBool::new(false)),
                status: String::new(),
                last_error: None,
            },
            sender,
        )
    }

    fn edit(
        bridge: &EditorAutomationBridge,
        project: &EditorProject,
        actions: Vec<EditAction>,
        check: bool,
    ) -> AutomationRequest {
        AutomationRequest {
            project_instance_id: Some(bridge.project_instance_id.clone()),
            revision: Some(project.revision()),
            action: AutomationAction::EditProject,
            args: json!({"actions":actions,"check":check}),
        }
    }

    fn create(id: u16) -> EditAction {
        EditAction {
            kind: EditActionKind::Operation {
                operation: Operation::CreateObject {
                    object_id: Some(id),
                    object_type: ObjectType::NumberVariable,
                    name: format!("Value{id}"),
                },
            },
            line: None,
        }
    }

    fn enqueue(
        sender: &SyncSender<Incoming>,
        request: AutomationRequest,
        state: u8,
    ) -> Receiver<AutomationResponse> {
        let (reply, receiver) = mpsc::sync_channel(1);
        sender
            .send(Incoming {
                request,
                reply,
                state: Arc::new(AtomicU8::new(state)),
            })
            .unwrap();
        receiver
    }

    #[test]
    fn edits_commit_immediately_and_reject_stale_requests() {
        let (mut bridge, sender) = bridge();
        let mut project = EditorProject::from(ObjectPool::default());
        let revision = project.revision();
        let request = edit(&bridge, &project, vec![create(42)], false);
        let response = enqueue(&sender, request.clone(), REQUEST_PENDING);
        bridge.poll(Some(&mut project));
        let response = response.recv().unwrap();
        assert!(response.success, "{:?}", response.error);
        assert_eq!(response.result.unwrap()["allocated_ids"], json!([42]));
        assert!(project
            .get_pool()
            .object_by_id(ag_iso_stack::object_pool::ObjectId::new(42).unwrap())
            .is_some());
        assert_eq!(project.revision(), revision + 1);
        let response = enqueue(&sender, request, REQUEST_PENDING);
        bridge.poll(Some(&mut project));
        assert_eq!(
            response.recv().unwrap().error.unwrap().code,
            "PROJECT_REVISION_MISMATCH"
        );
        assert!(project.undo_operation());
        assert!(project.get_pool().objects().is_empty());
    }

    #[test]
    fn failed_and_check_only_batches_preserve_project() {
        let (mut bridge, sender) = bridge();
        let mut project = EditorProject::from(ObjectPool::default());
        let revision = project.revision();
        let request = edit(&bridge, &project, vec![create(42)], true);
        let response = enqueue(&sender, request, REQUEST_PENDING);
        bridge.poll(Some(&mut project));
        assert!(response.recv().unwrap().success);
        assert_eq!(project.revision(), revision);
        assert!(project.get_pool().objects().is_empty());
        let request = edit(&bridge, &project, vec![create(42), create(42)], false);
        let response = enqueue(&sender, request, REQUEST_PENDING);
        bridge.poll(Some(&mut project));
        assert!(!response.recv().unwrap().success);
        assert_eq!(project.revision(), revision);
        assert!(project.get_pool().objects().is_empty());
        assert!(!project.undo_available());
    }

    #[test]
    fn expired_edits_and_replaced_projects_cannot_mutate() {
        let (mut bridge, sender) = bridge();
        let mut project = EditorProject::from(ObjectPool::default());
        let request = edit(&bridge, &project, vec![create(42)], false);
        let expired = enqueue(&sender, request.clone(), REQUEST_EXPIRED);
        bridge.poll(Some(&mut project));
        assert!(expired.recv().is_err());
        assert!(project.get_pool().objects().is_empty());
        bridge.project_changed();
        let response = enqueue(&sender, request, REQUEST_PENDING);
        bridge.poll(Some(&mut project));
        assert_eq!(
            response.recv().unwrap().error.unwrap().code,
            "PROJECT_INSTANCE_MISMATCH"
        );
        assert!(project.get_pool().objects().is_empty());
    }

    #[test]
    fn authentication_is_checked_before_queueing() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, incoming) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(
                stream,
                sender,
                "secret".into(),
                eframe::egui::Context::default(),
            );
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(
            &mut client,
            &WireRequest {
                token: "wrong".into(),
                request: AutomationRequest {
                    project_instance_id: None,
                    revision: None,
                    action: AutomationAction::GetProjectStatus,
                    args: json!({}),
                },
            },
        )
        .unwrap();
        client.write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        let response: AutomationResponse = serde_json::from_str(&line).unwrap();
        assert_eq!(response.error.unwrap().code, "AUTHENTICATION_FAILED");
        server.join().unwrap();
        assert!(incoming.try_recv().is_err());
    }
}
