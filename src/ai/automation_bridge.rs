//! Authenticated loopback bridge between the native editor and automation clients.
//!
//! The editor thread owns `EditorProject`. Network threads only move bounded, owned
//! requests; domain work runs against a cloned `AiDesignState` on a worker thread.

use crate::{
    ai::{
        automation_service,
        automation_trace::{self, ProposalMetrics, ProposalTrace, RequestTrace, TraceSink},
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
    pub address: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireRequest {
    token: String,
    #[serde(default)]
    project_instance_id: Option<String>,
    #[serde(default)]
    revision: Option<ProjectRevision>,
    action: String,
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
    error: Option<WireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<ProjectRevision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<ProjectRevision>,
}

/// One provider-neutral action sent through the authenticated editor bridge.
/// Status is the sole bootstrap action and may omit project identity/revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRequest {
    #[serde(default)]
    pub project_instance_id: Option<String>,
    #[serde(default)]
    pub revision: Option<ProjectRevision>,
    pub action: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationResponse {
    pub success: bool,
    pub editor_instance_id: String,
    pub project_instance_id: String,
    pub revision: Option<ProjectRevision>,
    pub result: Option<Value>,
    pub error: Option<WireError>,
}

#[derive(Debug, Clone)]
pub struct AutomationClientError {
    pub code: String,
    pub message: String,
    pub expected_revision: Option<ProjectRevision>,
    pub current_revision: Option<ProjectRevision>,
}

impl std::fmt::Display for AutomationClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AutomationClientError {}

impl From<WireError> for AutomationClientError {
    fn from(error: WireError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            expected_revision: error.expected_revision,
            current_revision: error.current_revision,
        }
    }
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
    result: Result<automation_service::DomainResult, automation_service::DomainError>,
}

/// Native editor side of the automation connection. Construct it on the UI thread and call
/// `poll` once per frame. Proposal methods also belong to that thread.
pub struct EditorAutomationBridge {
    editor_instance_id: String,
    project_instance_id: String,
    observed_revision: Option<ProjectRevision>,
    pending_proposal: Option<AiProposal>,
    runtime_path: PathBuf,
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

impl EditorAutomationBridge {
    pub fn start(ctx: eframe::egui::Context) -> Result<Self, String> {
        Self::start_impl(ctx, None)
    }

    fn start_impl(
        ctx: eframe::egui::Context,
        trace_path_override: Option<PathBuf>,
    ) -> Result<Self, String> {
        #[cfg(not(unix))]
        return Err(
            "The automation bridge currently requires Unix owner-only runtime file permissions"
                .into(),
        );

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| format!("Could not start local automation bridge: {e}"))?;
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
            incoming,
            domain_jobs: jobs_tx,
            completions,
            listener_stop,
            status: "Waiting for an automation client".into(),
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
                self.error_response(
                    "PROPOSAL_STALE",
                    "Project or proposal changed while this request was running.",
                    completion.incoming.request.revision,
                    revision,
                )
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
                    Err(error) => self.error_response(
                        error.code,
                        error.message,
                        completion.incoming.request.revision,
                        revision,
                    ),
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
                let response = if incoming.request.action == "get_project_status" {
                    self.status_response(None)
                } else {
                    self.error_response(
                        "PROJECT_NOT_OPEN",
                        "Open a project in the editor before using automation.",
                        incoming.request.revision,
                        None,
                    )
                };
                self.note_call(response.success);
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
            if request.action == "get_project_status" {
                if !incoming.claim() {
                    continue;
                }
                let response = self.status_response(revision);
                self.note_call(true);
                let _ = incoming.reply.try_send(response);
                continue;
            }
            if request.project_instance_id.as_deref() != Some(self.project_instance_id.as_str()) {
                if !incoming.claim() {
                    continue;
                }
                let response = self.error_response(
                    "PROJECT_INSTANCE_MISMATCH",
                    "The open project changed; inspect it again before retrying.",
                    request.revision,
                    revision,
                );
                let _ = incoming.reply.try_send(response);
                continue;
            }
            if request.revision != revision {
                if !incoming.claim() {
                    continue;
                }
                let response = self.error_response(
                    "PROJECT_REVISION_MISMATCH",
                    "Project changed; inspect it again before staging changes.",
                    request.revision,
                    revision,
                );
                let _ = incoming.reply.try_send(response);
                continue;
            }
            if request.action == "reject_proposal" {
                if !incoming.claim() {
                    continue;
                }
                let proposal_id = request.args["proposal_id"].as_str();
                if proposal_id.is_none()
                    || self
                        .pending_proposal
                        .as_ref()
                        .map(|p| p.proposal_id.as_str())
                        != proposal_id
                {
                    let response = self.error_response(
                        "PROPOSAL_NOT_FOUND",
                        "Proposal ID does not match the pending proposal.",
                        request.revision,
                        revision,
                    );
                    self.note_call(false);
                    let _ = incoming.reply.try_send(response);
                } else {
                    self.reject_proposal();
                    self.note_call(true);
                    let response = WireResponse {
                        success: true,
                        editor_instance_id: self.editor_instance_id.clone(),
                        project_instance_id: self.project_instance_id.clone(),
                        revision,
                        result: Some(
                            serde_json::json!({"rejected": true, "proposal_id": proposal_id}),
                        ),
                        error: None,
                    };
                    let _ = incoming.reply.try_send(response);
                }
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
                    self.status = "Processing an automation request".into();
                    break;
                }
                Err(TrySendError::Full(job)) => {
                    if !job.incoming.claim() {
                        continue;
                    }
                    let response = self.error_response(
                        "EDITOR_BUSY",
                        "The editor is busy; retry shortly.",
                        job.incoming.request.revision,
                        revision,
                    );
                    self.note_call(false);
                    let _ = job.incoming.reply.try_send(response);
                }
                Err(TrySendError::Disconnected(job)) => {
                    if !job.incoming.claim() {
                        continue;
                    }
                    let response = self.error_response(
                        "EDITOR_UNAVAILABLE",
                        "The editor automation worker stopped.",
                        job.incoming.request.revision,
                        revision,
                    );
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
                timestamp_ms: automation_trace::now_ms(),
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

    fn error_response(
        &mut self,
        code: &str,
        message: impl Into<String>,
        expected_revision: Option<ProjectRevision>,
        current_revision: Option<ProjectRevision>,
    ) -> WireResponse {
        let message = message.into();
        self.last_error = Some(message.clone());
        self.status = message.clone();
        WireResponse {
            success: false,
            editor_instance_id: self.editor_instance_id.clone(),
            project_instance_id: self.project_instance_id.clone(),
            revision: self.observed_revision,
            result: None,
            error: Some(WireError {
                code: code.into(),
                message,
                expected_revision,
                current_revision,
            }),
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

impl Drop for EditorAutomationBridge {
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
        .name("aitp-automation-listener".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let incoming = incoming.clone();
                        let token = token.clone();
                        let context = context.clone();
                        let trace = trace.clone();
                        let _ = thread::Builder::new()
                            .name("aitp-automation-request".into())
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
        .expect("could not spawn automation listener");
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
            request.action.clone(),
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
    if let Some(trace) = trace {
        let (tool, requested_proposal_id, requested_project_id, requested_revision) =
            trace_meta.unwrap_or_else(|| ("<invalid>".into(), None, None, None));
        let response_proposal_id = response.result.as_ref().and_then(|value| {
            value["proposal_id"]
                .as_str()
                .or_else(|| value["proposal"]["proposal_id"].as_str())
        });
        let proposal_id = response_proposal_id.or(requested_proposal_id.as_deref());
        trace.record(&RequestTrace {
            event: "request",
            timestamp_ms: automation_trace::now_ms(),
            editor_instance_id: &response.editor_instance_id,
            project_instance_id: if response.project_instance_id.is_empty() {
                requested_project_id.as_deref().unwrap_or("")
            } else {
                &response.project_instance_id
            },
            revision: response.revision.or(requested_revision),
            action: &tool,
            request_bytes,
            response_bytes: response_bytes.len() + 1,
            duration_ms: automation_trace::duration_ms(started.elapsed()),
            success: response.success,
            error_code: response.error.as_ref().map(|error| error.code.as_str()),
            proposal_id,
        });
    }
}

fn start_domain_worker(
    jobs: Receiver<DomainJob>,
    completions: SyncSender<DomainCompletion>,
    context: eframe::egui::Context,
) {
    thread::Builder::new()
        .name("aitp-automation-domain".into())
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                let result = automation_service::execute(
                    &job.state,
                    job.pending.as_ref(),
                    &job.incoming.request.action,
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
        .expect("could not spawn automation domain worker");
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
        None if records.is_empty() => {
            Err("No running editor found. Open AgIsoTerminalDesigner before using aitd.".into())
        }
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
    request.project_instance_id.as_deref() == Some(current_project_instance)
        && request.revision == current_revision
        && completion_editor_instance == current_editor_instance
        && completion_epoch == current_epoch
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
        project_instance_id: request.project_instance_id,
        revision: request.revision,
        action: request.action,
        args: request.args,
    };
    serde_json::to_writer(&mut stream, &wire_request).map_err(|e| AutomationClientError {
        code: "INVALID_REQUEST".into(),
        message: e.to_string(),
        expected_revision: wire_request.revision,
        current_revision: None,
    })?;
    stream.write_all(b"\n").map_err(|e| AutomationClientError {
        code: "EDITOR_CONNECTION_FAILED".into(),
        message: e.to_string(),
        expected_revision: wire_request.revision,
        current_revision: None,
    })?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| AutomationClientError {
            code: "EDITOR_CONNECTION_FAILED".into(),
            message: e.to_string(),
            expected_revision: wire_request.revision,
            current_revision: None,
        })?;
    let response: WireResponse =
        serde_json::from_str(&line).map_err(|e| AutomationClientError {
            code: "INVALID_RESPONSE".into(),
            message: format!("Invalid editor response: {e}"),
            expected_revision: wire_request.revision,
            current_revision: None,
        })?;
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| WireError {
                code: "EDITOR_ERROR".into(),
                message: "Editor automation request failed".into(),
                expected_revision: wire_request.revision,
                current_revision: response.revision,
            })
            .into());
    }
    Ok(AutomationResponse {
        success: response.success,
        editor_instance_id: response.editor_instance_id,
        project_instance_id: response.project_instance_id,
        revision: response.revision,
        result: response.result,
        error: response.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(instance_id: &str) -> (PathBuf, RuntimeRecord) {
        (
            PathBuf::from(format!("/{instance_id}.json")),
            RuntimeRecord {
                instance_id: instance_id.into(),
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
            project_instance_id: Some("p1".into()),
            revision: Some(4),
            action: "propose_transaction".into(),
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
        let bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
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
        let runtime: RuntimeRecord =
            serde_json::from_slice(&fs::read(&bridge.runtime_path).unwrap()).unwrap();
        let mut stream = TcpStream::connect(&runtime.address).unwrap();
        let request = WireRequest {
            token: "wrong-token".into(),
            project_instance_id: None,
            revision: None,
            action: "get_project_status".into(),
            args: Value::Null,
        };
        serde_json::to_writer(&mut stream, &request).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let response: WireResponse = serde_json::from_str(&line).unwrap();
        assert!(!response.success);
        assert_eq!(
            response.error.as_ref().map(|error| error.message.as_str()),
            Some("Bridge authentication failed")
        );
    }

    #[test]
    fn authenticated_status_works_without_an_open_project() {
        let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
        let instance_id = bridge.editor_instance_id.clone();
        let caller = thread::spawn(move || {
            invoke_runtime_request(
                Some(&instance_id),
                AutomationRequest {
                    project_instance_id: None,
                    revision: None,
                    action: "get_project_status".into(),
                    args: Value::Null,
                },
            )
        });
        for _ in 0..100 {
            bridge.poll(None);
            if caller.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let response = caller.join().unwrap().unwrap();
        assert!(response.success);
        assert_eq!(response.result.unwrap()["project_open"], false);
    }

    fn call_tool(
        bridge: &mut EditorAutomationBridge,
        project: &EditorProject,
        tool: &'static str,
        args: Value,
    ) -> Result<Value, String> {
        let instance_id = bridge.editor_instance_id.clone();
        let project_instance_id = bridge.project_instance_id.clone();
        let revision = project.revision();
        let caller = thread::spawn(move || {
            invoke_runtime_request(
                Some(&instance_id),
                AutomationRequest {
                    project_instance_id: Some(project_instance_id),
                    revision: Some(revision),
                    action: tool.into(),
                    args,
                },
            )
            .map(|response| serde_json::to_value(response).unwrap())
            .map_err(|error| error.to_string())
        });
        for _ in 0..200 {
            bridge.poll(Some(project));
            if caller.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            caller.is_finished(),
            "automation call did not complete after egui polling"
        );
        caller.join().expect("automation caller thread panicked")
    }

    #[test]
    fn live_roundtrip_stages_rejects_applies_and_undoes() {
        let trace_path = std::env::temp_dir().join(format!(
            "agiso-automation-cycle-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let mut bridge = EditorAutomationBridge::start_impl(
            eframe::egui::Context::default(),
            Some(trace_path.clone()),
        )
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
                "description": "Add automation object",
                "operations": [{
                    "type": "CreateObject",
                    "data": {"object_id": null, "object_type": "OutputString", "name": "automation output"}
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
        let missing = call_tool(
            &mut bridge,
            &project,
            "reject_proposal",
            serde_json::json!({}),
        );
        assert!(missing.is_err());

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
            && event["action"] == "get_objects"
            && event["request_bytes"].as_u64().unwrap() > 0));
        drop(bridge);
        std::fs::remove_file(trace_path).unwrap();
    }

    #[test]
    fn concurrent_fresh_submissions_leave_only_one_proposal_staged() {
        let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
        let project = EditorProject::new_pool();
        let args = serde_json::json!({
            "transaction": {
                "operations": [{
                    "type": "CreateObject",
                    "data": {"object_id": null, "object_type": "OutputString", "name": "Concurrent automation object"}
                }]
            }
        });
        let instance_id = bridge.editor_instance_id.clone();
        let spawn_call = |args| {
            let instance_id = instance_id.clone();
            let project_instance_id = bridge.project_instance_id.clone();
            let revision = project.revision();
            thread::spawn(move || {
                invoke_runtime_request(
                    Some(&instance_id),
                    AutomationRequest {
                        project_instance_id: Some(project_instance_id),
                        revision: Some(revision),
                        action: "propose_transaction".into(),
                        args,
                    },
                )
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
            "agiso-automation-refine-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let mut bridge = EditorAutomationBridge::start_impl(
            eframe::egui::Context::default(),
            Some(path.clone()),
        )
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
