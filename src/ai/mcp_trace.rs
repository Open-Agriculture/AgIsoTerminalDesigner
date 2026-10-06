//! Optional, local MCP diagnostics. Trace records never contain tool arguments,
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
    pub tool: &'a str,
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
        let enabled = std::env::var("AG_ISO_MCP_TRACE").ok().is_some_and(|value| {
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
            .map_err(|e| format!("Could not open MCP trace {}: {e}", path.display()))?;
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
                    log::warn!("MCP trace write failed: {error}");
                }
            }
            Err(_) => log::warn!("MCP trace writer lock was poisoned"),
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

pub fn error_code(message: &str) -> &'static str {
    if message.contains("authentication") {
        "authentication_failed"
    } else if message.contains("queue is full") || message.contains("busy") {
        "busy"
    } else if message.contains("Timed out") {
        "timeout"
    } else if message.contains("project")
        && (message.contains("changed") || message.contains("revision"))
    {
        "stale_project"
    } else if message.contains("proposal")
        && (message.contains("ID") || message.contains("matches") || message.contains("awaiting"))
    {
        "stale_proposal"
    } else if message.contains("Open a project") || message.contains("No project is open") {
        "no_project"
    } else if message.contains("worker stopped") || message.contains("shutting down") {
        "worker_unavailable"
    } else if message.contains("Invalid bridge request") {
        "invalid_bridge_request"
    } else if message.contains("Unsupported tool") {
        "unsupported_tool"
    } else if message.starts_with('[') {
        "validation_failed"
    } else {
        "tool_error"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_private_json_lines_without_payloads() {
        let path = std::env::temp_dir().join(format!(
            "agiso-mcp-trace-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let sink = TraceSink::create(path.clone()).unwrap();
        sink.record(&RequestTrace {
            event: "request",
            timestamp_ms: now_ms(),
            editor_instance_id: "editor",
            project_instance_id: "project",
            revision: Some(4),
            tool: "get_objects",
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
        assert_eq!(value["tool"], "get_objects");
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

    #[test]
    fn error_codes_group_actionable_failures() {
        assert_eq!(error_code("Bridge authentication failed"), "authentication_failed");
        assert_eq!(error_code("The project revision changed; retry the request"), "stale_project");
        assert_eq!(error_code("[{\"code\":\"OBJECT_NOT_FOUND\"}]"), "validation_failed");
    }
}
