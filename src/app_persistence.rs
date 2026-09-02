//! Small, platform-local persistence for crash recovery and desktop recents.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const RECOVERY_VERSION: u32 = 1;
#[cfg(target_arch = "wasm32")]
const RECOVERY_KEY: &str = "ag_iso_terminal_designer_recovery_v1";
#[cfg(not(target_arch = "wasm32"))]
const RECENTS_VERSION: u32 = 1;
#[cfg(not(target_arch = "wasm32"))]
const MAX_RECENTS: usize = 5;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryRecord {
    pub version: u32,
    pub display_name: String,
    pub saved_at_unix_seconds: u64,
    pub project_json: String,
}

impl RecoveryRecord {
    pub fn new(display_name: String, project_data: Vec<u8>) -> Result<Self, String> {
        let project_json = String::from_utf8(project_data)
            .map_err(|error| format!("Project recovery data is not valid UTF-8: {error}"))?;
        Ok(Self {
            version: RECOVERY_VERSION,
            display_name,
            saved_at_unix_seconds: now_unix_seconds(),
            project_json,
        })
    }

    fn validate(self) -> Result<Self, String> {
        if self.version != RECOVERY_VERSION {
            return Err(format!("Unsupported recovery version {}", self.version));
        }
        Ok(self)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProject {
    pub path: std::path::PathBuf,
    pub display_name: String,
    pub last_opened_unix_seconds: u64,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default, Serialize, Deserialize)]
struct RecentProjectsFile {
    version: u32,
    projects: Vec<RecentProject>,
}

#[derive(Clone)]
pub struct AppPersistence {
    #[cfg(not(target_arch = "wasm32"))]
    data_dir: Option<std::path::PathBuf>,
}

impl AppPersistence {
    pub fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            data_dir: eframe::storage_dir("AgIsoTerminalDesigner"),
        }
    }

    pub fn load_recovery(&self) -> Result<Option<RecoveryRecord>, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = self.data_path("recovery.json") else {
                return Ok(None);
            };
            let data = match std::fs::read(path) {
                Ok(data) => data,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(format!("Could not read recovery data: {error}")),
            };
            return serde_json::from_slice::<RecoveryRecord>(&data)
                .map_err(|error| format!("Recovery data is invalid: {error}"))
                .and_then(RecoveryRecord::validate)
                .map(Some);
        }

        #[cfg(target_arch = "wasm32")]
        {
            let Some(storage) = web_storage()? else {
                return Ok(None);
            };
            let Some(value) = storage.get_item(RECOVERY_KEY).map_err(js_error)? else {
                return Ok(None);
            };
            serde_json::from_str::<RecoveryRecord>(&value)
                .map_err(|error| format!("Recovery data is invalid: {error}"))
                .and_then(RecoveryRecord::validate)
                .map(Some)
        }
    }

    pub fn store_recovery(&self, record: &RecoveryRecord) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = self
                .data_path("recovery.json")
                .ok_or_else(|| "No application data directory is available".to_owned())?;
            let bytes = serde_json::to_vec(record)
                .map_err(|error| format!("Could not serialize recovery data: {error}"))?;
            return atomic_write(&path, &bytes);
        }

        #[cfg(target_arch = "wasm32")]
        {
            let storage =
                web_storage()?.ok_or_else(|| "Browser local storage is unavailable".to_owned())?;
            let value = serde_json::to_string(record)
                .map_err(|error| format!("Could not serialize recovery data: {error}"))?;
            storage.set_item(RECOVERY_KEY, &value).map_err(js_error)
        }
    }

    pub fn clear_recovery(&self) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = self.data_path("recovery.json") else {
                return Ok(());
            };
            return match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("Could not remove recovery data: {error}")),
            };
        }

        #[cfg(target_arch = "wasm32")]
        {
            if let Some(storage) = web_storage()? {
                storage.remove_item(RECOVERY_KEY).map_err(js_error)?;
            }
            Ok(())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_recents(&self) -> Result<Vec<RecentProject>, String> {
        let Some(path) = self.data_path("recent_projects.json") else {
            return Ok(Vec::new());
        };
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("Could not read recent projects: {error}")),
        };
        let file: RecentProjectsFile = serde_json::from_slice(&data)
            .map_err(|error| format!("Recent-project data is invalid: {error}"))?;
        if file.version != RECENTS_VERSION {
            return Err(format!(
                "Unsupported recent-project version {}",
                file.version
            ));
        }
        Ok(file.projects.into_iter().take(MAX_RECENTS).collect())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn store_recents(&self, projects: &[RecentProject]) -> Result<(), String> {
        let path = self
            .data_path("recent_projects.json")
            .ok_or_else(|| "No application data directory is available".to_owned())?;
        let file = RecentProjectsFile {
            version: RECENTS_VERSION,
            projects: projects.iter().take(MAX_RECENTS).cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|error| format!("Could not serialize recent projects: {error}"))?;
        atomic_write(&path, &bytes)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn touch_recent(&self, projects: &mut Vec<RecentProject>, path: std::path::PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        projects.retain(|entry| entry.path != path);
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project.aitp")
            .to_owned();
        projects.insert(
            0,
            RecentProject {
                path,
                display_name,
                last_opened_unix_seconds: now_unix_seconds(),
            },
        );
        projects.truncate(MAX_RECENTS);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn data_path(&self, name: &str) -> Option<std::path::PathBuf> {
        self.data_dir.as_ref().map(|directory| directory.join(name))
    }
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(not(target_arch = "wasm32"))]
fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create application data directory: {error}"))?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, data)
        .map_err(|error| format!("Could not write temporary data: {error}"))?;
    if let Err(rename_error) = std::fs::rename(&temporary, path) {
        std::fs::copy(&temporary, path).map_err(|copy_error| {
            format!("Could not replace persisted data: {rename_error}; {copy_error}")
        })?;
        let _ = std::fs::remove_file(temporary);
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn web_storage() -> Result<Option<web_sys::Storage>, String> {
    web_sys::window()
        .ok_or_else(|| "Browser window is unavailable".to_owned())?
        .local_storage()
        .map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Browser storage operation failed".to_owned())
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    #[test]
    fn recents_are_deduplicated_and_capped() {
        let persistence = AppPersistence { data_dir: None };
        let mut projects = Vec::new();
        for index in 0..7 {
            persistence.touch_recent(
                &mut projects,
                std::path::PathBuf::from(format!("project-{index}.aitp")),
            );
        }
        assert_eq!(projects.len(), 5);
        persistence.touch_recent(&mut projects, std::path::PathBuf::from("project-4.aitp"));
        assert_eq!(projects.len(), 5);
        assert_eq!(projects[0].display_name, "project-4.aitp");
    }

    #[test]
    fn recovery_round_trips_and_clears() {
        let data_dir = std::env::temp_dir().join(format!("aitd-test-{}", uuid::Uuid::new_v4()));
        let persistence = AppPersistence {
            data_dir: Some(data_dir.clone()),
        };
        let record = RecoveryRecord::new("Test project".to_owned(), b"[1,2,3]".to_vec()).unwrap();

        persistence.store_recovery(&record).unwrap();
        assert_eq!(persistence.load_recovery().unwrap(), Some(record));
        persistence.clear_recovery().unwrap();
        assert!(persistence.load_recovery().unwrap().is_none());

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn corrupt_recovery_is_reported_without_being_loaded() {
        let data_dir = std::env::temp_dir().join(format!("aitd-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("recovery.json"), b"not json").unwrap();
        let persistence = AppPersistence {
            data_dir: Some(data_dir.clone()),
        };

        assert!(persistence.load_recovery().is_err());
        assert!(data_dir.join("recovery.json").exists());

        let _ = std::fs::remove_dir_all(data_dir);
    }
}
