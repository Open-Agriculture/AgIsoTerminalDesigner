//! Authenticated loopback bridge between the native editor and its stdio MCP child.
//!
//! The editor thread owns `EditorProject`. Network threads only move bounded, owned
//! requests; domain work runs against a cloned `AiDesignState` on a worker thread.

use crate::{
    ai::{
        mcp_domain::{self, DomainResult},
        mcp_trace::{self, ProposalMetrics, ProposalTrace, RequestTrace, TraceSink},
        AiProposal, ProjectRevision,
    },
    EditorProject,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const REQUEST_CAPACITY: usize = 32;
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
const REQUEST_PENDING: u8 = 0;
const REQUEST_CLAIMED: u8 = 1;
const REQUEST_EXPIRED: u8 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub instance_id: String,
    pub project_instance_id: String,
    pub revision: Option<ProjectRevision>,
    pub address: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireRequest {
    token: String,
    project_instance_id: String,
    revision: ProjectRevision,
    tool: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireResponse {
    success: bool,
    editor_instance_id: String,
    project_instance_id: String,
    revision: Option<ProjectRevision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

struct Incoming {
    request: WireRequest,
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

struct DomainJob {
    incoming: Incoming,
    state: crate::ai::AiDesignState,
    pending: Option<AiProposal>,
    editor_instance_id: String,
    proposal_epoch: u64,
}

struct DomainCompletion {
    incoming: Incoming,
    editor_instance_id: String,
    proposal_epoch: u64,
    result: Result<mcp_domain::DomainResult, mcp_domain::DomainError>,
}

/// Native editor side of the MCP connection. Construct it on the UI thread and call
/// `poll` once per frame. Proposal methods also belong to that thread.
pub struct EditorMcpBridge {
    editor_instance_id: String,
    project_instance_id: String,
    observed_revision: Option<ProjectRevision>,
    pending_proposal: Option<AiProposal>,
    runtime_path: PathBuf,
    runtime: RuntimeRecord,
    incoming: Receiver<Incoming>,
    domain_jobs: SyncSender<DomainJob>,
    completions: Receiver<DomainCompletion>,
    listener_stop: Arc<AtomicBool>,
    status: String,
    last_error: Option<String>,
    context: eframe::egui::Context,
    work_in_flight: bool,
    proposal_epoch: u64,
    trace: Option<Arc<TraceSink>>,
    proposal_metrics: ProposalMetrics,
}

impl EditorMcpBridge {
    pub fn start(ctx: eframe::egui::Context) -> Result<Self, String> {
        Self::start_impl(ctx, None)
    }

    fn start_impl(
        ctx: eframe::egui::Context,
        trace_path_override: Option<PathBuf>,
    ) -> Result<Self, String> {
        #[cfg(not(unix))]
        return Err(
            "The MCP bridge currently requires Unix owner-only runtime file permissions".into(),
        );

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| format!("Could not start local MCP bridge: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string();
        let editor_instance_id = uuid::Uuid::new_v4().to_string();
        let project_instance_id = uuid::Uuid::new_v4().to_string();
        let runtime_path = runtime_dir()?.join(format!("{editor_instance_id}.json"));
        let trace = match trace_path_override {
            Some(path) => Some(Arc::new(TraceSink::create(path)?)),
            None => TraceSink::optional(&runtime_path)?,
        };
        let runtime = RuntimeRecord {
            instance_id: editor_instance_id.clone(),
            project_instance_id: project_instance_id.clone(),
            revision: None,
            address,
            token: uuid::Uuid::new_v4().to_string(),
        };
        write_runtime(&runtime_path, &runtime)?;

        let (incoming_tx, incoming) = mpsc::sync_channel(REQUEST_CAPACITY);
        let (jobs_tx, jobs_rx) = mpsc::sync_channel::<DomainJob>(1);
        let (completion_tx, completions) = mpsc::sync_channel(1);
        let listener_stop = Arc::new(AtomicBool::new(false));
        start_listener(
            listener,
            incoming_tx,
            runtime.token.clone(),
            listener_stop.clone(),
            ctx.clone(),
            trace.clone(),
        );
        start_domain_worker(jobs_rx, completion_tx, ctx.clone());

        Ok(Self {
            editor_instance_id,
            project_instance_id,
            observed_revision: None,
            pending_proposal: None,
            runtime_path,
            runtime,
            incoming,
            domain_jobs: jobs_tx,
            completions,
            listener_stop,
            status: "Waiting for an MCP client".into(),
            last_error: None,
            context: ctx,
            work_in_flight: false,
            proposal_epoch: 0,
            trace,
            proposal_metrics: ProposalMetrics::default(),
        })
    }

    /// Process network requests and publish project identity/revision changes.
    pub fn poll(&mut self, project: Option<&EditorProject>) {
        let revision = project.map(EditorProject::revision);
        if revision != self.observed_revision {
            if self
                .pending_proposal
                .as_ref()
                .is_some_and(|p| Some(p.base_revision) != revision)
            {
                self.finish_proposal("invalidated");
                self.pending_proposal = None;
                self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
                self.status = "The staged proposal was cleared because the project changed".into();
            }
            self.observed_revision = revision;
            self.runtime.revision = revision;
            if let Err(e) = write_runtime(&self.runtime_path, &self.runtime) {
                self.last_error = Some(format!("Could not update MCP runtime details: {e}"));
            }
        }

        if let Ok(completion) = self.completions.try_recv() {
            self.work_in_flight = false;
            let still_current = completion.incoming.claim()
                && completion_is_current(
                    &completion.incoming.request,
                    &self.project_instance_id,
                    revision,
                    &completion.editor_instance_id,
                    &self.editor_instance_id,
                    completion.proposal_epoch,
                    self.proposal_epoch,
                );
            let response = if !still_current {
                self.error_response("The project changed while this request was running")
            } else {
                match completion.result {
                    Ok(result) => {
                        if let Some(proposal) = result.proposal {
                            if self.pending_proposal.is_some() {
                                self.proposal_metrics.refinement_rounds += 1;
                            } else {
                                self.proposal_metrics.first_proposal_id =
                                    Some(proposal.proposal_id.clone());
                            }
                            self.proposal_metrics.proposal_id = Some(proposal.proposal_id.clone());
                            self.pending_proposal = Some(proposal);
                            self.status = "A proposal is ready for review".into();
                        }
                        self.last_error = None;
                        WireResponse {
                            success: true,
                            editor_instance_id: self.editor_instance_id.clone(),
                            project_instance_id: self.project_instance_id.clone(),
                            revision,
                            result: Some(result.value),
                            error: None,
                        }
                    }
                    Err(error) => self.error_response(error.message),
                }
            };
            if still_current {
                self.note_call(response.success);
            }
            if completion.incoming.state.load(Ordering::Acquire) == REQUEST_CLAIMED {
                let _ = completion.incoming.reply.try_send(response);
            }
            self.context.request_repaint();
        }

        if self.work_in_flight {
            return;
        }
        let Some(project) = project else {
            while let Ok(incoming) = self.incoming.try_recv() {
                if !incoming.claim() {
                    continue;
                }
                let response = if incoming.request.tool == "get_project_status"
                    && incoming.request.project_instance_id == self.project_instance_id
                {
                    self.status_response(None)
                } else {
                    self.error_response("Open a project in the editor before using MCP tools")
                };
                if incoming.request.project_instance_id == self.project_instance_id {
                    self.note_call(response.success);
                }
                let _ = incoming.reply.try_send(response);
            }
            return;
        };

        for _ in 0..REQUEST_CAPACITY {
            let incoming = match self.incoming.try_recv() {
                Ok(incoming) => incoming,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            if incoming.state.load(Ordering::Acquire) == REQUEST_EXPIRED {
                continue;
            }
            let request = &incoming.request;
            if request.project_instance_id != self.project_instance_id {
                if !incoming.claim() {
                    continue;
                }
                let response = self.error_response("The open project changed; retry the request");
                let _ = incoming.reply.try_send(response);
                continue;
            }
            if Some(request.revision) != revision {
                if !incoming.claim() {
                    continue;
                }
                let response =
                    self.error_response("The project revision changed; retry the request");
                let _ = incoming.reply.try_send(response);
                continue;
            }
            if request.tool == "get_project_status" {
                if !incoming.claim() {
                    continue;
                }
                let response = self.status_response(revision);
                self.note_call(true);
                let _ = incoming.reply.try_send(response);
                continue;
            }
            let job = DomainJob {
                incoming,
                state: project.ai_design_state(),
                pending: self.pending_proposal.clone(),
                editor_instance_id: self.editor_instance_id.clone(),
                proposal_epoch: self.proposal_epoch,
            };
            match self.domain_jobs.try_send(job) {
                Ok(()) => {
                    self.work_in_flight = true;
                    self.status = "Processing an MCP request".into();
                    break;
                }
                Err(TrySendError::Full(job)) => {
                    if !job.incoming.claim() {
                        continue;
                    }
                    let response = self.error_response("The MCP bridge is busy; retry shortly");
                    self.note_call(false);
                    let _ = job.incoming.reply.try_send(response);
                }
                Err(TrySendError::Disconnected(job)) => {
                    if !job.incoming.claim() {
                        continue;
                    }
                    let response = self.error_response("The MCP domain worker stopped");
                    self.note_call(false);
                    let _ = job.incoming.reply.try_send(response);
                }
            }
        }
    }

    pub fn proposal(&self) -> Option<&AiProposal> {
        self.pending_proposal.as_ref()
    }
    pub fn take_proposal(&mut self) -> Option<AiProposal> {
        self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
        self.pending_proposal.take()
    }
    pub fn reject_proposal(&mut self) {
        if self.pending_proposal.is_some() {
            self.finish_proposal("rejected");
        }
        self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
        self.pending_proposal = None;
        self.status = "Proposal rejected".into();
    }
    pub fn invalidate_proposal(&mut self) {
        if self.pending_proposal.is_some() {
            self.finish_proposal("invalidated");
        } else {
            self.proposal_metrics = ProposalMetrics::default();
        }
        self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
        self.pending_proposal = None;
        self.project_instance_id = uuid::Uuid::new_v4().to_string();
        self.runtime.project_instance_id = self.project_instance_id.clone();
        self.runtime.revision = self.observed_revision;
        if let Err(e) = write_runtime(&self.runtime_path, &self.runtime) {
            self.last_error = Some(format!("Could not update MCP runtime details: {e}"));
        }
        self.status = "Project identity changed; staged proposal cleared".into();
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
    pub fn trace_path(&self) -> Option<&std::path::Path> {
        self.trace.as_ref().map(|trace| trace.path())
    }

    pub fn record_applied(&mut self, proposal_id: &str) {
        if self.proposal_metrics.proposal_id.as_deref() == Some(proposal_id) {
            self.finish_proposal("applied");
        }
    }

    pub fn record_apply_failed(&mut self, proposal_id: &str) {
        if self.proposal_metrics.proposal_id.as_deref() == Some(proposal_id) {
            self.finish_proposal("apply_failed");
        }
    }

    fn note_call(&mut self, success: bool) {
        self.proposal_metrics.tool_calls += 1;
        if !success {
            self.proposal_metrics.failed_calls += 1;
        }
    }

    fn finish_proposal(&mut self, outcome: &str) {
        if let (Some(trace), Some(first), Some(id)) = (
            &self.trace,
            &self.proposal_metrics.first_proposal_id,
            &self.proposal_metrics.proposal_id,
        ) {
            trace.record(&ProposalTrace {
                event: "proposal",
                timestamp_ms: mcp_trace::now_ms(),
                editor_instance_id: &self.editor_instance_id,
                project_instance_id: &self.project_instance_id,
                revision: self.observed_revision,
                first_proposal_id: first,
                proposal_id: id,
                tool_calls: self.proposal_metrics.tool_calls,
                failed_calls: self.proposal_metrics.failed_calls,
                refinement_rounds: self.proposal_metrics.refinement_rounds,
                outcome,
            });
        }
        self.proposal_metrics = ProposalMetrics::default();
    }

    fn error_response(&mut self, error: impl Into<String>) -> WireResponse {
        let error = error.into();
        self.last_error = Some(error.clone());
        self.status = error.clone();
        WireResponse {
            success: false,
            editor_instance_id: self.editor_instance_id.clone(),
            project_instance_id: self.project_instance_id.clone(),
            revision: self.observed_revision,
            result: None,
            error: Some(error),
        }
    }

    fn status_response(&self, revision: Option<ProjectRevision>) -> WireResponse {
        WireResponse {
            success: true,
            editor_instance_id: self.editor_instance_id.clone(),
            project_instance_id: self.project_instance_id.clone(),
            revision,
            result: Some(serde_json::json!({
                "project_open": revision.is_some(),
                "revision": revision,
                "proposal": self.pending_proposal.as_ref().map(|p| serde_json::json!({
                    "proposal_id": p.proposal_id,
                    "base_revision": p.base_revision,
                    "operation_count": p.summary.operation_count,
                })),
            })),
            error: None,
        }
    }
}

impl Drop for EditorMcpBridge {
    fn drop(&mut self) {
        if self.pending_proposal.is_some() {
            self.finish_proposal("closed");
        }
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
    trace: Option<Arc<TraceSink>>,
) {
    thread::Builder::new()
        .name("aitp-mcp-listener".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let incoming = incoming.clone();
                        let token = token.clone();
                        let context = context.clone();
                        let trace = trace.clone();
                        let _ = thread::Builder::new()
                            .name("aitp-mcp-request".into())
                            .spawn(move || {
                                handle_connection(stream, incoming, token, context, trace)
                            });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => thread::sleep(Duration::from_millis(100)),
                }
            }
        })
        .expect("could not spawn MCP listener");
}

fn handle_connection(
    mut stream: TcpStream,
    incoming: SyncSender<Incoming>,
    token: String,
    context: eframe::egui::Context,
    trace: Option<Arc<TraceSink>>,
) {
    let started = Instant::now();
    let _ = stream.set_read_timeout(Some(TOOL_TIMEOUT));
    let mut request_bytes = 0;
    let request = {
        let mut line = String::new();
        let mut reader = BufReader::new(&stream).take(1024 * 1024 + 1);
        match reader.read_line(&mut line) {
            Ok(n) if n > 0 && n <= 1024 * 1024 => {
                request_bytes = n;
                serde_json::from_str::<WireRequest>(&line)
            }
            Ok(n) => {
                request_bytes = n;
                Err(serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "empty or oversized request",
                )))
            }
            Err(e) => Err(serde_json::Error::io(e)),
        }
    };
    let trace_meta = request.as_ref().ok().map(|request| {
        (
            request.tool.clone(),
            request.args["proposal_id"].as_str().map(str::to_owned),
            request.project_instance_id.clone(),
            request.revision,
        )
    });
    let response = match request {
        Err(e) => WireResponse {
            success: false,
            editor_instance_id: String::new(),
            project_instance_id: String::new(),
            revision: None,
            result: None,
            error: Some(format!("Invalid bridge request: {e}")),
        },
        Ok(request) if !constant_time_eq(&request.token, &token) => WireResponse {
            success: false,
            editor_instance_id: String::new(),
            project_instance_id: String::new(),
            revision: None,
            result: None,
            error: Some("Bridge authentication failed".into()),
        },
        Ok(request) => {
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
                        error: Some("Timed out waiting for editor response".into()),
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
                                // The editor already committed a response; allow its send to finish.
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
                    error: Some("MCP request queue is full; retry shortly".into()),
                },
                Err(TrySendError::Disconnected(_)) => WireResponse {
                    success: false,
                    editor_instance_id: String::new(),
                    project_instance_id: String::new(),
                    revision: None,
                    result: None,
                    error: Some("Editor MCP bridge is shutting down".into()),
                },
            }
        }
    };
    let response_bytes = serde_json::to_vec(&response).unwrap_or_default();
    let _ = stream.write_all(&response_bytes);
    let _ = stream.write_all(b"\n");
    if let Some(trace) = trace {
        let (tool, requested_proposal_id, requested_project_id, requested_revision) =
            trace_meta.unwrap_or_else(|| ("<invalid>".into(), None, String::new(), 0));
        let response_proposal_id = response.result.as_ref().and_then(|value| {
            value["proposal_id"]
                .as_str()
                .or_else(|| value["proposal"]["proposal_id"].as_str())
        });
        let proposal_id = response_proposal_id.or(requested_proposal_id.as_deref());
        trace.record(&RequestTrace {
            event: "request",
            timestamp_ms: mcp_trace::now_ms(),
            editor_instance_id: &response.editor_instance_id,
            project_instance_id: if response.project_instance_id.is_empty() {
                &requested_project_id
            } else {
                &response.project_instance_id
            },
            revision: response
                .revision
                .or(trace_meta_revision(requested_revision, &tool)),
            tool: &tool,
            request_bytes,
            response_bytes: response_bytes.len() + 1,
            duration_ms: mcp_trace::duration_ms(started.elapsed()),
            success: response.success,
            error_code: response.error.as_deref().map(mcp_trace::error_code),
            proposal_id,
        });
    }
}

fn trace_meta_revision(revision: u64, tool: &str) -> Option<u64> {
    (tool != "<invalid>").then_some(revision)
}

fn start_domain_worker(
    jobs: Receiver<DomainJob>,
    completions: SyncSender<DomainCompletion>,
    context: eframe::egui::Context,
) {
    thread::Builder::new()
        .name("aitp-mcp-domain".into())
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                let result = mcp_domain::execute(
                    &job.state,
                    job.pending.as_ref(),
                    &job.incoming.request.tool,
                    job.incoming.request.args.clone(),
                );
                let completion = DomainCompletion {
                    incoming: job.incoming,
                    editor_instance_id: job.editor_instance_id,
                    proposal_epoch: job.proposal_epoch,
                    result,
                };
                if completions.send(completion).is_err() {
                    break;
                }
                context.request_repaint();
            }
        })
        .expect("could not spawn MCP domain worker");
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
        "The MCP bridge currently requires Unix owner-only runtime file permissions".into(),
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
        Err(error) => return Err(format!("Could not create MCP runtime directory: {error}")),
    }
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|e| format!("Could not inspect MCP runtime directory: {e}"))?;
    if !metadata.file_type().is_dir() {
        return Err("MCP runtime path is not a directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Never publish a token in a directory owned by another local account.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err("MCP runtime directory is owned by another user".into());
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
        return Err("Owner-only MCP runtime permissions are unavailable on this platform".into());
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
    choose_runtime(records, instance_id)
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
        None if records.is_empty() => Err(
            "No running editor found. Open AgIsoTerminalDesigner before starting the MCP server."
                .into(),
        ),
        None if records.len() > 1 => {
            Err("Multiple editors are running. Start with --instance <editor-instance-id>.".into())
        }
        None => Ok(records.remove(0)),
    }
}

fn completion_is_current(
    request: &WireRequest,
    current_project_instance: &str,
    current_revision: Option<ProjectRevision>,
    completion_editor_instance: &str,
    current_editor_instance: &str,
    completion_epoch: u64,
    current_epoch: u64,
) -> bool {
    request.project_instance_id == current_project_instance
        && Some(request.revision) == current_revision
        && completion_editor_instance == current_editor_instance
        && completion_epoch == current_epoch
}

/// Invoke one native editor tool using the authenticated local protocol.
pub fn invoke_runtime_tool(
    instance_id: Option<&str>,
    tool: &str,
    args: Value,
) -> Result<Value, String> {
    let (_, runtime) = select_runtime(instance_id)?;
    let revision = match runtime.revision {
        Some(revision) => revision,
        None if tool == "get_project_status" => 0,
        None => return Err("No project is open in the selected editor".into()),
    };
    let address = runtime
        .address
        .parse::<std::net::SocketAddr>()
        .map_err(|e| format!("Invalid editor bridge address: {e}"))?;
    if !address.ip().is_loopback() {
        return Err("Editor bridge address is not loopback".into());
    }
    let mut stream = TcpStream::connect(address)
        .map_err(|e| format!("Could not connect to editor bridge: {e}"))?;
    stream
        .set_read_timeout(Some(TOOL_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let request = WireRequest {
        token: runtime.token,
        project_instance_id: runtime.project_instance_id,
        revision,
        tool: tool.into(),
        args,
    };
    serde_json::to_writer(&mut stream, &request).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let response: WireResponse =
        serde_json::from_str(&line).map_err(|e| format!("Invalid editor response: {e}"))?;
    if response.success {
        Ok(serde_json::to_value(response).unwrap_or(Value::Null))
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Editor MCP request failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(instance_id: &str) -> (PathBuf, RuntimeRecord) {
        (
            PathBuf::from(format!("/{instance_id}.json")),
            RuntimeRecord {
                instance_id: instance_id.into(),
                project_instance_id: "project".into(),
                revision: Some(1),
                address: "127.0.0.1:1".into(),
                token: "secret".into(),
            },
        )
    }

    #[test]
    fn instance_selection_requires_choice_when_ambiguous() {
        let records = vec![record("one"), record("two")];
        assert!(choose_runtime(records.clone(), None)
            .unwrap_err()
            .contains("Multiple editors"));
        assert_eq!(
            choose_runtime(records, Some("two")).unwrap().1.instance_id,
            "two"
        );
    }

    #[test]
    fn completion_is_discarded_after_project_or_proposal_changes() {
        let request = WireRequest {
            token: "token".into(),
            project_instance_id: "p1".into(),
            revision: 4,
            tool: "propose_transaction".into(),
            args: Value::Null,
        };
        assert!(completion_is_current(
            &request,
            "p1",
            Some(4),
            "e1",
            "e1",
            7,
            7
        ));
        assert!(!completion_is_current(
            &request,
            "p2",
            Some(4),
            "e1",
            "e1",
            7,
            7
        ));
        assert!(!completion_is_current(
            &request,
            "p1",
            Some(5),
            "e1",
            "e1",
            7,
            7
        ));
        assert!(!completion_is_current(
            &request,
            "p1",
            Some(4),
            "e1",
            "e1",
            7,
            8
        ));
    }

    #[test]
    fn loopback_rejects_a_bad_token() {
        let bridge = EditorMcpBridge::start(eframe::egui::Context::default()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&bridge.runtime_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let mut stream = TcpStream::connect(&bridge.runtime.address).unwrap();
        let request = WireRequest {
            token: "wrong-token".into(),
            project_instance_id: bridge.project_instance_id.clone(),
            revision: 0,
            tool: "get_project_status".into(),
            args: Value::Null,
        };
        serde_json::to_writer(&mut stream, &request).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: WireResponse = serde_json::from_str(&line).unwrap();
        assert!(!response.success);
        assert_eq!(
            response.error.as_deref(),
            Some("Bridge authentication failed")
        );
    }

    #[test]
    fn authenticated_status_works_without_an_open_project() {
        let mut bridge = EditorMcpBridge::start(eframe::egui::Context::default()).unwrap();
        let instance_id = bridge.editor_instance_id.clone();
        let caller = thread::spawn(move || {
            invoke_runtime_tool(Some(&instance_id), "get_project_status", Value::Null)
        });
        for _ in 0..100 {
            bridge.poll(None);
            if caller.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let response = caller.join().unwrap().unwrap();
        assert_eq!(response["success"], true);
        assert_eq!(response["result"]["project_open"], false);
    }

    fn call_tool(
        bridge: &mut EditorMcpBridge,
        project: &EditorProject,
        tool: &'static str,
        args: Value,
    ) -> Result<Value, String> {
        let instance_id = bridge.editor_instance_id.clone();
        let caller = thread::spawn(move || invoke_runtime_tool(Some(&instance_id), tool, args));
        for _ in 0..200 {
            bridge.poll(Some(project));
            if caller.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            caller.is_finished(),
            "MCP call did not complete after egui polling"
        );
        caller.join().expect("MCP caller thread panicked")
    }

    #[test]
    fn live_roundtrip_stages_rejects_applies_and_undoes() {
        let trace_path = std::env::temp_dir().join(format!(
            "agiso-mcp-cycle-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let mut bridge =
            EditorMcpBridge::start_impl(eframe::egui::Context::default(), Some(trace_path.clone()))
                .unwrap();
        let mut project = EditorProject::new_pool();
        let unsaved_id = project
            .create_object(
                ag_iso_stack::object_pool::ObjectType::OutputString,
                "Unsaved".into(),
            )
            .unwrap();

        let objects = call_tool(
            &mut bridge,
            &project,
            "get_objects",
            serde_json::json!({"ids":[unsaved_id.value()]}),
        )
        .unwrap();
        assert_eq!(objects["result"]["objects"][0]["id"], unsaved_id.value());

        let context = call_tool(
            &mut bridge,
            &project,
            "get_design_context",
            serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(context["success"], true);
        assert_eq!(context["revision"], project.revision());

        let proposal_args = serde_json::json!({
            "transaction": {
                "description": "Add MCP object",
                "operations": [{
                    "type": "CreateObject",
                    "data": {"object_id": null, "object_type": "OutputString", "name": "MCP output"}
                }]
            }
        });
        let staged = call_tool(
            &mut bridge,
            &project,
            "propose_transaction",
            proposal_args.clone(),
        )
        .unwrap();
        assert_eq!(staged["success"], true, "{staged}");
        let proposal_id = staged["result"]["proposal_id"].as_str().unwrap();
        assert_eq!(bridge.proposal().unwrap().proposal_id, proposal_id);

        let status = call_tool(
            &mut bridge,
            &project,
            "proposal_status",
            serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(status["result"]["proposal"]["proposal_id"], proposal_id);

        let invalid_refinement = call_tool(
            &mut bridge,
            &project,
            "propose_transaction",
            proposal_args.clone(),
        );
        assert!(invalid_refinement.is_err());
        assert_eq!(bridge.proposal().unwrap().proposal_id, proposal_id);

        bridge.reject_proposal();
        assert!(bridge.proposal().is_none());

        let staged =
            call_tool(&mut bridge, &project, "propose_transaction", proposal_args).unwrap();
        assert_eq!(staged["success"], true, "{staged}");
        let before = project.get_pool().objects().len();
        let proposal = bridge.take_proposal().unwrap();
        let applied_id = proposal.proposal_id.clone();
        project.apply_ai_proposal(proposal).unwrap();
        bridge.record_applied(&applied_id);
        assert_eq!(project.get_pool().objects().len(), before + 1);
        assert!(project.undo_operation());
        assert_eq!(project.get_pool().objects().len(), before);

        let invalid = call_tool(
            &mut bridge,
            &project,
            "propose_transaction",
            serde_json::json!({
                "transaction": {
                    "operations": [{"type":"DeleteObject","data":{"object_id":65534}}]
                }
            }),
        );
        assert!(invalid.is_err());
        assert!(bridge.proposal().is_none());
        let events: Vec<Value> = std::fs::read_to_string(&trace_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let rejected = events
            .iter()
            .find(|event| event["event"] == "proposal" && event["outcome"] == "rejected")
            .unwrap();
        assert_eq!(rejected["failed_calls"], 1);
        assert!(rejected["tool_calls"].as_u64().unwrap() >= 4);
        assert!(events.iter().any(|event| event["event"] == "proposal"
            && event["outcome"] == "applied"
            && event["proposal_id"] == applied_id));
        assert!(events.iter().any(|event| event["event"] == "request"
            && event["tool"] == "get_objects"
            && event["request_bytes"].as_u64().unwrap() > 0));
        drop(bridge);
        std::fs::remove_file(trace_path).unwrap();
    }

    #[test]
    fn concurrent_fresh_submissions_leave_only_one_proposal_staged() {
        let mut bridge = EditorMcpBridge::start(eframe::egui::Context::default()).unwrap();
        let project = EditorProject::new_pool();
        let args = serde_json::json!({
            "transaction": {
                "operations": [{
                    "type": "CreateObject",
                    "data": {"object_id": null, "object_type": "OutputString", "name": "Concurrent MCP object"}
                }]
            }
        });
        let instance_id = bridge.editor_instance_id.clone();
        let spawn_call = |args| {
            let instance_id = instance_id.clone();
            thread::spawn(move || {
                invoke_runtime_tool(Some(&instance_id), "propose_transaction", args)
            })
        };
        let first = spawn_call(args.clone());
        let second = spawn_call(args);
        for _ in 0..400 {
            bridge.poll(Some(&project));
            if first.is_finished() && second.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(first.is_finished() && second.is_finished());
        let outcomes = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert_eq!(
            outcomes.iter().filter(|outcome| outcome.is_err()).count(),
            1
        );
        assert!(bridge.proposal().is_some());
    }

    #[test]
    fn trace_counts_refinement_as_one_proposal() {
        let path = std::env::temp_dir().join(format!(
            "agiso-mcp-refine-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let mut bridge =
            EditorMcpBridge::start_impl(eframe::egui::Context::default(), Some(path.clone()))
                .unwrap();
        let project = EditorProject::new_pool();
        let first = call_tool(&mut bridge, &project, "propose_transaction", serde_json::json!({
            "operations":[{"type":"CreateObject","data":{"object_id":100,"object_type":"OutputString","name":"Text"}}]
        })).unwrap();
        let first_id = first["result"]["proposal_id"].as_str().unwrap().to_owned();
        let refined = call_tool(&mut bridge, &project, "propose_transaction", serde_json::json!({
            "proposal_id":first_id,
            "transaction":{"operations":[{"type":"SetProperty","data":{"object":{"id":100},"property":"width","value":180}}]}
        })).unwrap();
        let final_id = refined["result"]["proposal_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(first_id, final_id);
        bridge.reject_proposal();
        let event: Value = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|event| event["event"] == "proposal")
            .unwrap();
        assert_eq!(event["first_proposal_id"], first_id);
        assert_eq!(event["proposal_id"], final_id);
        assert_eq!(event["tool_calls"], 2);
        assert_eq!(event["failed_calls"], 0);
        assert_eq!(event["refinement_rounds"], 1);
        assert_eq!(event["outcome"], "rejected");
        drop(bridge);
        std::fs::remove_file(path).unwrap();
    }
}
