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
    collections::HashMap,
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
/// Status and historical draft/submission queries may omit project identity/revision.
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
    draft: Option<crate::ai::draft::Draft>,
    editor_instance_id: String,
    proposal_epoch: u64,
}

struct DomainCompletion {
    incoming: Incoming,
    editor_instance_id: String,
    proposal_epoch: u64,
    result: Result<DomainOutput, automation_service::DomainError>,
}

struct DomainOutput {
    result: automation_service::DomainResult,
    draft: Option<crate::ai::draft::Draft>,
}

/// A submission receipt remains queryable after review and project switches.
#[derive(Debug, Clone, Serialize)]
struct ProposalOutcome {
    proposal_id: String,
    draft_id: Option<String>,
    generation: Option<u64>,
    project_instance: String,
    base_revision: u64,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    applied_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transaction_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Native editor side of the automation connection. Construct it on the UI thread and call
/// `poll` once per frame. Proposal methods also belong to that thread.
pub struct EditorAutomationBridge {
    editor_instance_id: String,
    project_instance_id: String,
    observed_revision: Option<ProjectRevision>,
    current_project_path: Option<PathBuf>,
    original_source_path: Option<PathBuf>,
    display_name: String,
    observed_dirty: Option<bool>,
    pending_proposal: Option<AiProposal>,
    active_draft: Option<crate::ai::draft::Draft>,
    draft_records: HashMap<String, Value>,
    outcomes: HashMap<String, ProposalOutcome>,
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
            current_project_path: None,
            original_source_path: None,
            display_name: String::new(),
            observed_dirty: None,
            pending_proposal: None,
            active_draft: None,
            draft_records: HashMap::new(),
            outcomes: HashMap::new(),
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
        self.observed_dirty = project.map(EditorProject::is_dirty);
        let revision = project.map(EditorProject::revision);
        if revision != self.observed_revision {
            if self
                .active_draft
                .as_ref()
                .is_some_and(|d| Some(d.base_revision) != revision)
            {
                self.close_draft("invalidated");
                self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
            }
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
                    Ok(output) => {
                        if let Some(draft) = output.draft {
                            self.active_draft = Some(draft);
                            self.remember_draft("editing", None);
                            self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
                        }
                        let result = output.result;
                        if let Some(proposal) = result.proposal {
                            if let Some(previous) = self.pending_proposal.as_ref() {
                                if let Some(outcome) = self.outcomes.get_mut(&previous.proposal_id)
                                {
                                    outcome.status = "superseded".into();
                                }
                            }
                            if self.pending_proposal.is_some() {
                                self.proposal_metrics.refinement_rounds += 1;
                            } else {
                                self.proposal_metrics.first_proposal_id =
                                    Some(proposal.proposal_id.clone());
                            }
                            self.proposal_metrics.proposal_id = Some(proposal.proposal_id.clone());
                            self.outcomes.insert(
                                proposal.proposal_id.clone(),
                                ProposalOutcome {
                                    proposal_id: proposal.proposal_id.clone(),
                                    draft_id: self.active_draft.as_ref().map(|d| d.id.clone()),
                                    generation: self.active_draft.as_ref().map(|d| d.generation),
                                    project_instance: self.project_instance_id.clone(),
                                    base_revision: proposal.base_revision,
                                    status: "pending".into(),
                                    applied_revision: None,
                                    transaction_id: None,
                                    error: None,
                                },
                            );
                            self.remember_draft("submitted", Some(&proposal.proposal_id));
                            self.pending_proposal = Some(proposal);
                            self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
                            self.status = "A proposal is ready for review".into();
                        }
                        self.last_error = None;
                        self.success_response(revision, result.value)
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
                let response =
                    if let Some(response) = self.bootstrap_response(&incoming.request, None) {
                        response
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
            if matches!(
                request.action.as_str(),
                "get_project_status" | "proposal_outcome" | "draft_status"
            ) {
                if !incoming.claim() {
                    continue;
                }
                let response = self.bootstrap_response(request, revision).unwrap();
                self.note_call(response.success);
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
            if matches!(
                request.action.as_str(),
                "begin_draft" | "reopen_draft" | "discard_draft"
            ) {
                if !incoming.claim() {
                    continue;
                }
                let response = self.draft_lifecycle(request, project);
                self.note_call(response.success);
                let _ = incoming.reply.try_send(response);
                continue;
            }
            if matches!(request.action.as_str(), "edit_draft" | "submit_draft") {
                if let Err(message) = self.guard_draft(&request.args, true) {
                    if !incoming.claim() {
                        continue;
                    }
                    let response =
                        self.error_response("DRAFT_CONFLICT", message, request.revision, revision);
                    let _ = incoming.reply.try_send(response);
                    continue;
                }
            }
            if request.action == "export_artifacts" {
                if let Some(id) = request.args["proposal_id"].as_str() {
                    let valid = self.outcomes.get(id).is_some_and(|outcome| {
                        outcome.status == "applied"
                            && outcome.project_instance == self.project_instance_id
                            && outcome.applied_revision == revision
                    });
                    if !valid {
                        if !incoming.claim() {
                            continue;
                        }
                        let response = self.error_response("EXPORT_REVISION_CONFLICT", "Export requires this proposal's applied project and revision. Later edits or undo require a live export.", request.revision, revision);
                        let _ = incoming.reply.try_send(response);
                        continue;
                    }
                }
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
            let state = match self.read_state(project, &request.args) {
                Ok(state) => state,
                Err(message) => {
                    if !incoming.claim() {
                        continue;
                    }
                    let response = self.error_response(
                        "VIEW_UNAVAILABLE",
                        message,
                        request.revision,
                        revision,
                    );
                    let _ = incoming.reply.try_send(response);
                    continue;
                }
            };
            let job = DomainJob {
                incoming,
                state,
                pending: self.pending_proposal.clone(),
                draft: self.active_draft.clone(),
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

    fn remember_draft(&mut self, status: &str, proposal_id: Option<&str>) {
        if let Some(draft) = &self.active_draft {
            let previous = self.draft_records.get(&draft.id);
            let latest = proposal_id.map(str::to_owned).or_else(|| {
                previous
                    .and_then(|r| r["proposal_id"].as_str())
                    .map(str::to_owned)
            });
            self.draft_records.insert(draft.id.clone(), serde_json::json!({
                "draft_id":draft.id,"generation":draft.generation,"base_revision":draft.base_revision,
                "name":draft.name,"status":status,"project_instance":self.project_instance_id,
                "proposal_id":latest,"volatile":true,
            }));
        }
    }

    fn close_draft(&mut self, status: &str) {
        self.remember_draft(status, None);
        self.active_draft = None;
    }

    fn bootstrap_response(
        &mut self,
        request: &WireRequest,
        revision: Option<u64>,
    ) -> Option<WireResponse> {
        let response = match request.action.as_str() {
            "get_project_status" => self.status_response(revision),
            "proposal_outcome" => {
                match request.args["proposal_id"]
                    .as_str()
                    .and_then(|id| self.outcomes.get(id))
                    .cloned()
                {
                    Some(outcome) => {
                        self.success_response(revision, serde_json::json!({"outcome":outcome}))
                    }
                    None => self.error_response(
                        "PROPOSAL_UNKNOWN",
                        "No submission receipt matches this ID in this editor session.",
                        None,
                        revision,
                    ),
                }
            }
            "draft_status" => {
                let id = request.args["draft_id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| self.active_draft.as_ref().map(|d| d.id.clone()));
                match id.and_then(|id|self.draft_records.get(&id)).cloned() {
                    Some(mut draft) => {
                        if let Some(outcome) = draft["proposal_id"].as_str().and_then(|id|self.outcomes.get(id)) {
                            draft["outcome"] = serde_json::to_value(outcome).unwrap();
                        }
                        self.success_response(revision, serde_json::json!({"draft":draft}))
                    }
                    None => self.error_response("DRAFT_UNKNOWN", "No draft matches this ID. Begin a draft, or select its original editor instance.", None, revision),
                }
            }
            _ => return None,
        };
        Some(response)
    }

    fn guard_draft(&self, args: &Value, require_editing: bool) -> Result<(), String> {
        let draft = self
            .active_draft
            .as_ref()
            .ok_or("No active draft; begin one first")?;
        if args["draft_id"].as_str() != Some(draft.id.as_str())
            || args["generation"].as_u64() != Some(draft.generation)
        {
            return Err("Draft ID or generation changed; inspect the draft before retrying".into());
        }
        if require_editing && self.pending_proposal.is_some() {
            return Err("Draft is submitted for review; reopen it before editing".into());
        }
        Ok(())
    }

    fn draft_lifecycle(&mut self, request: &WireRequest, project: &EditorProject) -> WireResponse {
        let revision = Some(project.revision());
        if request.action == "begin_draft" {
            if request.args["name"]
                .as_str()
                .is_some_and(|name| name.len() > crate::operations::MAX_DESCRIPTION_LENGTH)
            {
                return self.error_response(
                    "VALIDATION_FAILED",
                    "Draft name exceeds the transaction description limit",
                    request.revision,
                    revision,
                );
            }
            if self.active_draft.is_some() || self.pending_proposal.is_some() {
                return self.error_response("DRAFT_CONFLICT", "An active draft or proposal already exists; inspect, reopen, reject, or discard it first.", request.revision, revision);
            }
            self.active_draft = Some(crate::ai::draft::begin(
                &project.ai_design_state(),
                request.args["name"].as_str().map(str::to_owned),
            ));
            self.remember_draft("editing", None);
        } else {
            if let Err(message) = self.guard_draft(&request.args, false) {
                return self.error_response("DRAFT_CONFLICT", message, request.revision, revision);
            }
            if request.action == "reopen_draft" {
                if self.pending_proposal.is_none() {
                    return self.error_response(
                        "DRAFT_CONFLICT",
                        "Draft is already editable",
                        request.revision,
                        revision,
                    );
                }
                self.finish_proposal("superseded");
                self.pending_proposal = None;
                self.active_draft.as_mut().unwrap().generation += 1;
                self.remember_draft("editing", None);
            } else {
                if self.pending_proposal.is_some() {
                    self.finish_proposal("rejected");
                }
                self.pending_proposal = None;
                self.close_draft("discarded");
                self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
                return self.success_response(
                    revision,
                    serde_json::json!({"discarded":true,"draft_id":request.args["draft_id"]}),
                );
            }
        }
        self.proposal_epoch = self.proposal_epoch.wrapping_add(1);
        let draft = self.active_draft.as_ref().unwrap();
        self.success_response(
            revision,
            serde_json::json!({"draft":self.draft_records[&draft.id]}),
        )
    }

    fn read_state(
        &self,
        project: &EditorProject,
        args: &Value,
    ) -> Result<crate::ai::AiDesignState, String> {
        let live = project.ai_design_state();
        match args["view"].as_str().unwrap_or("live") {
            "live" => Ok(live),
            "draft" => {
                let draft = self.active_draft.as_ref().ok_or("No active draft")?;
                if let Some(id) = args["draft_id"].as_str() {
                    if id != draft.id {
                        return Err("Draft ID does not match the active draft".into());
                    }
                }
                Ok(live)
            }
            "proposal" => {
                let proposal = self
                    .pending_proposal
                    .as_ref()
                    .ok_or("No pending proposal")?;
                if let Some(id) = args["proposal_id"].as_str() {
                    if id != proposal.proposal_id {
                        return Err("Proposal ID does not match the pending review".into());
                    }
                }
                Ok(live)
            }
            _ => Err("view must be live, draft, or proposal".into()),
        }
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
        self.close_draft("rejected");
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
        self.close_draft("invalidated");
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

    /// Publish native file identity from the application, without changing pool content.
    /// Paths are absolute; the original source is retained through Save As.
    /// An unsaved or recovered project may have no known path.
    pub fn set_project_location(
        &mut self,
        current: Option<&std::path::Path>,
        original: Option<&std::path::Path>,
        display_name: &str,
    ) {
        self.current_project_path = current.map(std::path::Path::to_path_buf);
        self.original_source_path = original.map(std::path::Path::to_path_buf);
        self.display_name = display_name.to_owned();
    }
    pub fn trace_path(&self) -> Option<&std::path::Path> {
        self.trace.as_ref().map(|trace| trace.path())
    }

    /// Record a successful GUI commit, including its exact resulting revision.
    pub fn record_applied(&mut self, proposal_id: &str, revision: u64, transaction_id: &str) {
        if let Some(outcome) = self.outcomes.get_mut(proposal_id) {
            outcome.status = "applied".into();
            outcome.applied_revision = Some(revision);
            outcome.transaction_id = Some(transaction_id.into());
        }
        self.close_draft("applied");
        if self.proposal_metrics.proposal_id.as_deref() == Some(proposal_id) {
            self.finish_proposal("applied");
        }
    }

    /// Record a failed GUI commit without reporting the submission as applied.
    pub fn record_apply_failed(&mut self, proposal_id: &str, error: String) {
        if let Some(outcome) = self.outcomes.get_mut(proposal_id) {
            outcome.status = "apply_failed".into();
            outcome.error = Some(error);
        }
        self.close_draft("apply_failed");
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
        if let Some(id) = &self.proposal_metrics.proposal_id {
            if let Some(record) = self.outcomes.get_mut(id) {
                record.status = outcome.into();
            }
        }
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
                "current_project_path": if revision.is_some() {self.current_project_path.as_deref().and_then(std::path::Path::to_str)} else {None},
                "original_source_path": if revision.is_some() {self.original_source_path.as_deref().and_then(std::path::Path::to_str)} else {None},
                "display_name": revision.map(|_|self.display_name.as_str()),
                "dirty": self.observed_dirty,
                "revision": revision,
                "proposal": self.pending_proposal.as_ref().map(|p| serde_json::json!({
                    "proposal_id": p.proposal_id,
                    "base_revision": p.base_revision,
                    "operation_count": p.summary.operation_count,
                })),
                "draft": self.active_draft.as_ref().and_then(|d|self.draft_records.get(&d.id)),
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
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execute_domain_job(&job)
                }))
                .unwrap_or_else(|_| {
                    Err(automation_service::DomainError {
                        code: "AUTOMATION_FAILED",
                        message: "Could not prepare the command; the draft is unchanged.".into(),
                    })
                });
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

fn proposal_state(
    live: &crate::ai::AiDesignState,
    proposal: &AiProposal,
) -> crate::ai::AiDesignState {
    let mut state = live.clone();
    state.pool = proposal.prepared.pool.clone();
    state.object_info = proposal.prepared.object_info.clone();
    state.canonical_pool = Some(proposal.canonical_pool.clone());
    state.components = proposal.components.clone();
    state.graphics = proposal.graphics.clone();
    state.soft_key_layout = proposal.soft_key_layout;
    if let Some(content) = &proposal.candidate_content {
        state.mask_size = content.mask_size;
        state.soft_key_size = content.soft_key_size;
        state.soft_key_layout = content.soft_key_layout;
        state.vt_version = content.vt_version;
        state.object_id_ranges = content.object_id_ranges.clone();
    }
    state
}

fn execute_domain_job(job: &DomainJob) -> Result<DomainOutput, automation_service::DomainError> {
    let request = &job.incoming.request;
    let failure = |message: String| automation_service::DomainError {
        code: "VALIDATION_FAILED",
        message,
    };
    let value = |value| DomainOutput {
        result: automation_service::DomainResult {
            value,
            proposal: None,
        },
        draft: None,
    };
    match request.action.as_str() {
        "edit_draft" => {
            let draft = job
                .draft
                .as_ref()
                .ok_or_else(|| failure("No active draft".into()))?;
            let actions = request.args["actions"]
                .as_array()
                .cloned()
                .ok_or_else(|| failure("Provide an actions array".into()))?;
            let next = crate::ai::draft::edit(&job.state, draft, actions).map_err(failure)?;
            let result = serde_json::json!({"draft":{"draft_id":next.id,"generation":next.generation,
                "base_revision":next.base_revision,"name":next.name,"status":"editing","volatile":true}});
            Ok(DomainOutput {
                result: automation_service::DomainResult {
                    value: result,
                    proposal: None,
                },
                draft: Some(next),
            })
        }
        "submit_draft" => {
            let draft = job
                .draft
                .as_ref()
                .ok_or_else(|| failure("No active draft".into()))?;
            let proposal = crate::ai::draft::submit(&job.state, draft).map_err(failure)?;
            let result = serde_json::json!({"proposal_id":proposal.proposal_id,"draft_id":draft.id,
                "generation":draft.generation,"base_revision":draft.base_revision,"status":"pending"});
            Ok(DomainOutput {
                result: automation_service::DomainResult {
                    value: result,
                    proposal: Some(proposal),
                },
                draft: None,
            })
        }
        "draft_diff" => {
            let draft = job
                .draft
                .as_ref()
                .ok_or_else(|| failure("No active draft".into()))?;
            if request.args["draft_id"].as_str() != Some(draft.id.as_str()) {
                return Err(failure("Draft does not match the active candidate".into()));
            }
            Ok(value(
                serde_json::json!({"diff":automation_service::diff_states(&job.state,&draft.design_state(&job.state)),
                "draft_id":draft.id,"generation":draft.generation}),
            ))
        }
        "proposal_diff" => {
            let proposal = job
                .pending
                .as_ref()
                .ok_or_else(|| failure("No pending proposal".into()))?;
            if request.args["proposal_id"]
                .as_str()
                .is_some_and(|id| id != proposal.proposal_id)
            {
                return Err(failure("Proposal does not match the pending review".into()));
            }
            Ok(value(
                serde_json::json!({"diff":automation_service::diff_states(&job.state,&proposal_state(&job.state,proposal)),"proposal_id":proposal.proposal_id}),
            ))
        }
        "export_artifacts" => {
            if request.args["view"]
                .as_str()
                .is_some_and(|view| view != "live")
            {
                return Err(failure(
                    "Production exports require the live project".into(),
                ));
            }
            let mut artifacts = crate::ai::export::artifacts(
                &job.state,
                request.args["format"].as_str().unwrap_or("bundle"),
            )
            .map_err(failure)?;
            artifacts["proposal_id"] = request.args["proposal_id"].clone();
            Ok(value(artifacts))
        }
        _ => {
            // New inspection requests select their state explicitly in the bridge.
            // Legacy proposal endpoints retain their pending-candidate contract.
            let pending = if request.args.get("view").is_some()
                && !matches!(
                    request.action.as_str(),
                    "preview_proposal" | "proposal_status"
                ) {
                None
            } else {
                job.pending.as_ref()
            };
            let selected_state = match request.args["view"].as_str().unwrap_or("live") {
                "draft" => job
                    .draft
                    .as_ref()
                    .map(|draft| draft.design_state(&job.state)),
                "proposal" => job
                    .pending
                    .as_ref()
                    .map(|proposal| proposal_state(&job.state, proposal)),
                _ => None,
            };
            let mut result = automation_service::execute(
                selected_state.as_ref().unwrap_or(&job.state),
                pending,
                &request.action,
                request.args.clone(),
            )?;
            if let Some(view) = request.args["view"].as_str() {
                result.value["view"] = serde_json::json!(view);
                if view == "draft" {
                    if let Some(draft) = &job.draft {
                        result.value["draft_id"] = serde_json::json!(draft.id);
                        result.value["generation"] = serde_json::json!(draft.generation);
                    }
                } else if view == "proposal" {
                    if let Some(proposal) = &job.pending {
                        result.value["proposal_id"] = serde_json::json!(proposal.proposal_id);
                    }
                }
            }
            Ok(DomainOutput {
                result,
                draft: None,
            })
        }
    }
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
    fn draft_review_receipts_and_guarded_exports_roundtrip() {
        let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
        let mut project = EditorProject::new_pool();
        let begin = call_tool(
            &mut bridge,
            &project,
            "begin_draft",
            serde_json::json!({"name":"CLI smoke"}),
        )
        .unwrap();
        assert_eq!(begin["success"], true, "{begin}");
        let id = begin["result"]["draft"]["draft_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let edit = call_tool(&mut bridge, &project, "edit_draft", serde_json::json!({"draft_id":id,"generation":0,"actions":[
            {"kind":"operation","operation":{"type":"CreateObject","data":{"object_type":"NumberVariable","object_id":null,"name":"CLI value"}}}
        ]})).unwrap();
        assert_eq!(edit["success"], true, "{edit}");
        assert_eq!(edit["result"]["draft"]["generation"], 1);
        let conflict = call_tool(
            &mut bridge,
            &project,
            "edit_draft",
            serde_json::json!({"draft_id":id,"generation":0,"actions":[]}),
        )
        .unwrap_err();
        assert!(conflict.starts_with("DRAFT_CONFLICT:"), "{conflict}");
        let view = call_tool(
            &mut bridge,
            &project,
            "list_objects",
            serde_json::json!({"view":"draft","draft_id":id,"query":"CLI value"}),
        )
        .unwrap();
        assert_eq!(view["result"]["objects"].as_array().unwrap().len(), 1);
        let submitted = call_tool(
            &mut bridge,
            &project,
            "submit_draft",
            serde_json::json!({"draft_id":id,"generation":1}),
        )
        .unwrap();
        assert_eq!(submitted["success"], true, "{submitted}");
        let proposal_id = submitted["result"]["proposal_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let pending = call_tool(
            &mut bridge,
            &project,
            "proposal_outcome",
            serde_json::json!({"proposal_id":proposal_id}),
        )
        .unwrap();
        assert_eq!(pending["result"]["outcome"]["status"], "pending");
        let applied = project
            .apply_ai_proposal(bridge.take_proposal().unwrap())
            .unwrap();
        let applied_revision = project.revision();
        bridge.record_applied(&proposal_id, applied_revision, &applied.transaction_id);
        let receipt = call_tool(
            &mut bridge,
            &project,
            "proposal_outcome",
            serde_json::json!({"proposal_id":proposal_id}),
        )
        .unwrap();
        assert_eq!(receipt["result"]["outcome"]["status"], "applied");
        assert_eq!(
            receipt["result"]["outcome"]["applied_revision"],
            applied_revision
        );
        let exported = call_tool(
            &mut bridge,
            &project,
            "export_artifacts",
            serde_json::json!({"format":"bundle","proposal_id":proposal_id}),
        )
        .unwrap();
        assert_eq!(exported["success"], true, "{exported}");
        assert!(exported["result"]["artifacts"]["iop"].is_string());
        assert!(exported["result"]["artifacts"]["header"].is_string());
        assert!(project.undo_operation());
        let stale = call_tool(
            &mut bridge,
            &project,
            "export_artifacts",
            serde_json::json!({"format":"bundle","proposal_id":proposal_id}),
        )
        .unwrap_err();
        assert!(stale.starts_with("EXPORT_REVISION_CONFLICT:"), "{stale}");
        bridge.invalidate_proposal();
        let receipt = call_tool(
            &mut bridge,
            &project,
            "proposal_outcome",
            serde_json::json!({"proposal_id":proposal_id}),
        )
        .unwrap();
        assert_eq!(receipt["result"]["outcome"]["status"], "applied");
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
        let applied = project.apply_ai_proposal(proposal).unwrap();
        bridge.record_applied(&applied_id, project.revision(), &applied.transaction_id);
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
