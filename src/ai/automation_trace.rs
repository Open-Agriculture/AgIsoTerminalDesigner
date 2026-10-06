//! Optional, local automation diagnostics. Trace records never contain tool arguments,
//! response bodies, or the bridge bearer token.
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct TraceSink {
    file: Mutex<File>,
    path: PathBuf,
}

#[derive(Serialize)]
pub struct RequestTrace<'a> {
    pub event: &'static str,
    pub timestamp_ms: u128,
    pub editor_instance_id: &'a str,
    pub project_instance_id: &'a str,
    pub revision: Option<u64>,
    pub action: &'a str,
    pub request_bytes: usize,
    pub response_bytes: usize,
    pub duration_ms: u128,
    pub success: bool,
    pub error_code: Option<&'a str>,
    pub proposal_id: Option<&'a str>,
}

#[derive(Default)]
pub struct ProposalMetrics {
    pub first_proposal_id: Option<String>,
    pub proposal_id: Option<String>,
    pub tool_calls: u64,
    pub failed_calls: u64,
    pub refinement_rounds: u64,
}

#[derive(Serialize)]
pub struct ProposalTrace<'a> {
    pub event: &'static str,
    pub timestamp_ms: u128,
    pub editor_instance_id: &'a str,
    pub project_instance_id: &'a str,
    pub revision: Option<u64>,
    pub first_proposal_id: &'a str,
    pub proposal_id: &'a str,
    pub tool_calls: u64,
    pub failed_calls: u64,
    pub refinement_rounds: u64,
    pub outcome: &'a str,
}

impl TraceSink {
    pub fn optional(runtime_path: &Path) -> Result<Option<Arc<Self>>, String> {
        let enabled = std::env::var("AG_ISO_AUTOMATION_TRACE")
            .ok()
            .is_some_and(|value| {
                matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
            });
        if !enabled {
            return Ok(None);
        }
        let path = runtime_path.with_extension("trace.jsonl");
        Self::create(path).map(|sink| Some(Arc::new(sink)))
    }

    pub(crate) fn create(path: PathBuf) -> Result<Self, String> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|e| format!("Could not open automation trace {}: {e}", path.display()))?;
        Ok(Self {
            file: Mutex::new(file),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record<T: Serialize>(&self, record: &T) {
        match self.file.lock() {
            Ok(mut file) => {
                let result = serde_json::to_vec(record)
                    .map_err(|error| error.to_string())
                    .and_then(|mut bytes| {
                        bytes.push(b'\n');
                        file.write_all(&bytes)
                            .and_then(|_| file.flush())
                            .map_err(|error| error.to_string())
                    });
                if let Err(error) = result {
                    log::warn!("automation trace write failed: {error}");
                }
            }
            Err(_) => log::warn!("automation trace writer lock was poisoned"),
        }
    }
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn duration_ms(duration: Duration) -> u128 {
    duration.as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_private_json_lines_without_payloads() {
        let path = std::env::temp_dir().join(format!(
            "agiso-automation-trace-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let sink = TraceSink::create(path.clone()).unwrap();
        sink.record(&RequestTrace {
            event: "request",
            timestamp_ms: now_ms(),
            editor_instance_id: "editor",
            project_instance_id: "project",
            revision: Some(4),
            action: "get_objects",
            request_bytes: 123,
            response_bytes: 456,
            duration_ms: 2,
            success: true,
            error_code: None,
            proposal_id: None,
        });
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["request_bytes"], 123);
        assert_eq!(value["action"], "get_objects");
        assert!(value.get("args").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
