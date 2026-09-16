//! Application-facing orchestration state. It owns snapshots, never an EditorProject.
use super::backend::{assistant_message, SYSTEM_INSTRUCTIONS};
use super::task_state::{
    AiEscalationReason, AiFailureClass, AiModelMode, AiModelRole, AiTaskPhase, AiTaskState,
};
use super::{
    analyze_design, get_design_context, get_object_schema, get_objects, propose_transaction,
    AiContent, AiDesignState, AiMessage, AiProposal, AiProvider, AiProviderError, AiRole,
    AiSettings, AiToolCall, AiToolResult, AiTurnRequest, Conversation, DesignAnalysisScope,
    DesignContextRequest, GetObjectsRequest,
};
use ag_iso_stack::object_pool::ObjectType;
use serde_json::{json, Value};
use std::sync::mpsc::Sender;

pub const MAX_TOOL_ROUNDS: usize = 8;
pub const MAX_TOOL_CALLS_PER_REQUEST: usize = 24;
pub const MAX_PROPOSAL_REPAIR_ATTEMPTS: usize = 2;
pub const MAX_VISUAL_REFINEMENT_ROUNDS: u8 = 2;
#[derive(Debug, Clone)]
pub enum AiRequestState {
    Idle,
    Running { request_id: String },
    ProposalReady,
    Error,
}
#[derive(Debug, Clone)]
pub struct AiAssistantState {
    pub request_state: AiRequestState,
    pub conversation: Conversation,
    pub active_proposal: Option<AiProposal>,
    pub last_error: Option<String>,
    pub settings: AiSettings,
    pub request_revision: Option<u64>,
}
impl Default for AiAssistantState {
    fn default() -> Self {
        Self {
            request_state: AiRequestState::Idle,
            conversation: Conversation::default(),
            active_proposal: None,
            last_error: None,
            settings: AiSettings::default(),
            request_revision: None,
        }
    }
}
#[derive(Debug, Clone)]
pub enum AiWorkerEvent {
    Finished {
        request_id: String,
        conversation: Conversation,
        proposal: Option<AiProposal>,
    },
    Failed {
        request_id: String,
        error: String,
        conversation: Conversation,
    },
}

pub fn run_request(
    provider: &dyn AiProvider,
    design: AiDesignState,
    mut conversation: Conversation,
    settings: AiSettings,
    request_id: String,
    sender: Sender<AiWorkerEvent>,
) {
    log::info!("AI request started: model={}", settings.model);
    conversation.compact();
    // Separate this worker's diagnostics from a cancelled/replaced conversation.
    let debug = conversation.debug.lock().unwrap().clone();
    conversation.debug = std::sync::Arc::new(std::sync::Mutex::new(debug));
    let result = run_loop(provider, &design, &mut conversation, settings);
    conversation.compact();
    match result {
        Ok(proposal) => {
            log::info!("AI request completed: proposal={}", proposal.is_some());
            let _ = sender.send(AiWorkerEvent::Finished {
                request_id,
                conversation,
                proposal,
            });
        }
        Err(error) => {
            log::warn!("AI request failed; details available in Assistant");
            let _ = sender.send(AiWorkerEvent::Failed {
                request_id,
                error: super::debug_trace::safe_text(error.to_string()),
                conversation,
            });
        }
    }
}
pub(crate) fn run_loop(
    provider: &dyn AiProvider,
    state: &AiDesignState,
    conversation: &mut Conversation,
    mut settings: AiSettings,
) -> Result<Option<AiProposal>, AiProviderError> {
    let runtime =
        tokio::runtime::Runtime::new().map_err(|e| AiProviderError::Http(e.to_string()))?;
    conversation.debug.lock().unwrap().begin_request();
    let mut cache = ReadToolCache::default();
    let goal = conversation
        .messages
        .iter()
        .rev()
        .find(|m| m.role == AiRole::User)
        .map(|m| {
            m.content
                .iter()
                .filter_map(|c| {
                    if let AiContent::Text(t) = c {
                        Some(t.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let mut task = AiTaskState::new(state, goal.clone());
    if super::task_state::explicit_ids(&goal).len() == 1 && task.explicit_target.is_none() {
        conversation.push(AiMessage::text(AiRole::Assistant,"The explicitly requested object does not exist in this project revision. Please provide its current ID."));
        return Ok(None);
    }
    settings.role = if settings.mode == AiModelMode::Strong {
        AiModelRole::Strong
    } else {
        AiModelRole::Fast
    };
    let mut summary = super::snapshot::project_summary(state);
    if let Some(target) = &task.explicit_target {
        summary.selected_object = None;
        if target.object_type.ends_with("Mask") {
            summary.project.active_mask = Some(target.clone());
        }
    }
    let summary = serde_json::to_string(&summary).unwrap();
    // A single explicitly named mask bounds forward inspection. Multiple explicit
    // targets deliberately remain unbound for legitimate cross-mask work.
    let target_subtree = task
        .explicit_target
        .as_ref()
        .filter(|t| t.object_type.ends_with("Mask"))
        .map(|target| {
            let root = ag_iso_stack::object_pool::ObjectId::new(target.id).ok();
            let mut ids: std::collections::BTreeSet<u16> = super::snapshot::context_layout(
                &state.pool,
                &state.object_info,
                root,
                super::snapshot::MAX_SNAPSHOT_DEPTH,
            )
            .take(8192)
            .map(|p| p.object.id)
            .collect();
            ids.insert(target.id);
            ids
        });
    let mut pending_escalation = None;
    let mut handoff: Option<(usize, Value)> = None;
    let mut proposal: Option<AiProposal> = None;
    let mut calls = 0;
    let mut repairs: usize = 0;
    for round in 0..MAX_TOOL_ROUNDS {
        if conversation
            .cancelled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            conversation.debug.lock().unwrap().failure_class = Some(AiFailureClass::StaleProject);
            return Err(AiProviderError::Cancelled);
        }
        if let Some(reason) = pending_escalation.take() {
            if settings.mode == AiModelMode::Automatic && settings.role == AiModelRole::Fast {
                settings.role = AiModelRole::Strong;
                task.escalation = Some(reason);
                let mut packet = task.handoff();
                if let Some(base) = &proposal {
                    let value = serde_json::to_value(&base.transaction).unwrap();
                    packet["staged_base"] = if value.to_string().len() <= 16_384 {
                        value
                    } else {
                        json!({"proposal_id":base.proposal_id,"operation_count":base.summary.operation_count,"transaction_omitted":true})
                    };
                    packet["repair_semantics"] = json!("A valid base is staged. propose_transaction supplies a correction against that candidate.");
                }
                conversation
                    .debug
                    .lock()
                    .unwrap()
                    .trace("Model escalation", &packet);
                handoff = Some((conversation.messages.len(), packet));
                conversation.replay_generation += 1;
                repairs = 0;
            }
        }
        settings.phase = task.phase;
        let instructions = format!("{SYSTEM_INSTRUCTIONS}\nProject: {summary}");
        let mut provider_conversation = conversation.clone();
        if let Some((cursor, packet)) = &handoff {
            provider_conversation.messages = vec![
                AiMessage::text(AiRole::User, &goal),
                AiMessage::text(AiRole::User, packet.to_string()),
            ];
            provider_conversation
                .messages
                .extend_from_slice(&conversation.messages[*cursor..]);
        }
        provider_conversation.task_state = Some(task.clone());
        let exposed_tools = super::tools::phase_tool_definitions(task.phase);
        let allowed: Vec<String> = exposed_tools
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        let progress_before = task.progress();
        let usage_count = {
            let mut debug = conversation.debug.lock().unwrap();
            debug.provider_rounds = round + 1;
            debug.current_model = super::providers::openai::model_id(&settings).into();
            debug.task = Some(task.clone());
            debug.user_request.request_count
        };
        let started = std::time::Instant::now();
        let response = runtime.block_on(provider.run_turn(AiTurnRequest {
            instructions,
            conversation: provider_conversation,
            tools: exposed_tools,
            settings: settings.clone(),
        }));
        {
            let mut debug = conversation.debug.lock().unwrap();
            debug.latency_ms += started.elapsed().as_millis() as u64;
            if debug.user_request.request_count == usage_count {
                if let Ok(response) = &response {
                    debug.record_usage(response.usage.clone());
                }
            }
        }
        let response = response.map_err(|error| {
            conversation.debug.lock().unwrap().failure_class =
                Some(AiFailureClass::ProviderTransport);
            error
        })?;
        if conversation
            .cancelled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            conversation.debug.lock().unwrap().failure_class = Some(AiFailureClass::StaleProject);
            return Err(AiProviderError::Cancelled);
        }
        let tool_calls: Vec<_> = response
            .content
            .iter()
            .filter_map(|c| {
                if let AiContent::ToolCall(call) = c {
                    Some(call.clone())
                } else {
                    None
                }
            })
            .collect();
        conversation.push(assistant_message(response.content));
        if tool_calls.is_empty() {
            return Ok(proposal);
        }
        calls += tool_calls.len();
        if calls > MAX_TOOL_CALLS_PER_REQUEST {
            return Err(AiProviderError::InvalidResponse(format!(
                "tool-call limit ({MAX_TOOL_CALLS_PER_REQUEST}) exceeded"
            )));
        }
        let mut results = Vec::new();
        let mut stop_reason = None;
        for mut call in tool_calls {
            if stop_reason.is_some() {
                results.push(AiContent::ToolResult(AiToolResult { call_id: call.call_id, result: json!({"success":false,"error":"Assistant stopped at its safety limit; this call was not executed."}) }));
                continue;
            }
            {
                let mut debug = conversation.debug.lock().unwrap();
                debug.tool_calls += 1;
                debug.trace(format!("← Tool call {}", call.name), &call.arguments);
            }
            let cache_hits_before = cache.1;
            // Resolve implicit scope without changing the UI selection.
            if call.name == "get_design_context" && task.explicit_target.is_some() {
                if call.arguments.get("root_id").is_none()
                    && call.arguments["scope"] != "EntirePool"
                {
                    call.arguments["root_id"] = json!(task.target_root);
                }
                call.arguments["include_selection"] = json!(false);
            }
            let outside_target = target_subtree.as_ref().is_some_and(|ids| {
                if matches!(
                    call.name.as_str(),
                    "get_design_context" | "inspect_design_region"
                ) {
                    call.arguments["scope"] == "EntirePool"
                        || call.arguments["root_id"]
                            .as_u64()
                            .is_some_and(|id| !ids.contains(&(id as u16)))
                } else if call.name == "get_objects" {
                    call.arguments["ids"].as_array().is_some_and(|requested| {
                        requested.iter().filter_map(Value::as_u64).any(|id| {
                            !ids.contains(&(id as u16))
                                && ag_iso_stack::object_pool::ObjectId::new(id as u16)
                                    .ok()
                                    .and_then(|id| state.pool.object_by_id(id))
                                    .is_some_and(|o| {
                                        format!("{:?}", o.object_type()).ends_with("Mask")
                                    })
                        })
                    })
                } else {
                    false
                }
            });
            let (mut result, proposed, repair) = if outside_target {
                (
                    json!({"success":false,"error":"Forward inspection is bounded to the explicit request mask. Use its descendants; unrelated UI selection is not a target."}),
                    None,
                    false,
                )
            } else if !allowed.contains(&call.name) {
                (
                    json!({"success":false,"error":"Tool unavailable in this phase. Call finish_inspection before proposing."}),
                    None,
                    false,
                )
            } else if call.name == "finish_inspection" {
                task.phase = AiTaskPhase::Propose;
                (json!({"success":true,"phase":"Propose"}), None, false)
            } else if call.name == "request_stronger_model" {
                if call.arguments["reason"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() && s.len() <= 512)
                {
                    pending_escalation = Some(AiEscalationReason::ModelRequestedHelp);
                    (
                        json!({"success":true,"escalation_available":settings.mode == AiModelMode::Automatic && settings.role == AiModelRole::Fast}),
                        None,
                        false,
                    )
                } else {
                    (
                        json!({"success":false,"error":"A short concrete reason is required"}),
                        None,
                        false,
                    )
                }
            } else {
                cache.execute(state, &call, proposal.as_ref())
            };
            if result["success"] == false {
                conversation.debug.lock().unwrap().failure_class =
                    Some(if result["failure_class"] == "ModelDecision" {
                        AiFailureClass::ModelDecision
                    } else if repair {
                        AiFailureClass::DomainValidation
                    } else if !allowed.contains(&call.name) {
                        AiFailureClass::ModelDecision
                    } else {
                        AiFailureClass::ToolExecution
                    });
            }
            if allowed.contains(&call.name) {
                task.observe(&call, &result);
            }
            if repair
                && task.repeated_validation_failure()
                && settings.role == AiModelRole::Fast
                && settings.mode == AiModelMode::Automatic
            {
                pending_escalation = Some(AiEscalationReason::RepeatedValidationFailure);
            }
            if repair {
                repairs += 1;
                result["repair_attempts_remaining"] =
                    json!(MAX_PROPOSAL_REPAIR_ATTEMPTS.saturating_sub(repairs.saturating_sub(1)));
                if repairs > MAX_PROPOSAL_REPAIR_ATTEMPTS && pending_escalation.is_none() {
                    let diagnostic = result["diagnostics"]
                        .as_str()
                        .unwrap_or("Unknown validation failure");
                    stop_reason = Some(format!("I could not produce a valid proposal after the initial attempt and {MAX_PROPOSAL_REPAIR_ATTEMPTS} repair attempts. Validation reported: {diagnostic}"));
                }
            }
            if let Some(next) = proposed {
                if next.refinement_round > MAX_VISUAL_REFINEMENT_ROUNDS {
                    result = json!({"success":false,"error":"Visual refinement limit reached; the previous staged proposal was retained."});
                    stop_reason = Some("I reached the visual refinement limit.".to_owned());
                } else {
                    if let Ok(preview) = super::render_proposal_preview(state, &next) {
                        conversation.debug.lock().unwrap().trace("→ Preview image", &json!({"bytes":preview.image.bytes.len(),"proposal_id":next.proposal_id}));
                        results.push(AiContent::Image(preview.image));
                    }
                    proposal = Some(next);
                }
            }
            conversation.debug.lock().unwrap().record_tool(
                &call.name,
                cache.1 > cache_hits_before,
                &result,
            );
            conversation
                .debug
                .lock()
                .unwrap()
                .trace(format!("→ Tool result {}", call.name), &result);
            results.push(AiContent::ToolResult(AiToolResult {
                call_id: call.call_id,
                result,
            }));
        }
        conversation.push(AiMessage {
            role: AiRole::Tool,
            content: results,
        });
        if let Some(hint) = task.inspection_finished(progress_before) {
            task.harness_hint = Some(hint.into());
            conversation
                .debug
                .lock()
                .unwrap()
                .trace("Harness hint", &json!({"hint":hint}));
        }
        if task.stuck() {
            if settings.role == AiModelRole::Fast && settings.mode == AiModelMode::Automatic {
                pending_escalation = Some(AiEscalationReason::InspectionLoop);
                task.phase = AiTaskPhase::Propose;
            } else {
                stop_reason = Some(
                    "Inspection stopped because repeated reads made no useful progress.".into(),
                );
            }
        }
        conversation.debug.lock().unwrap().task = Some(task.clone());
        if let Some(reason) = stop_reason {
            let outcome = if proposal.is_some() {
                "The previous valid proposal is still staged for review. No changes have been applied."
            } else {
                "No changes have been proposed or applied."
            };
            conversation.push(AiMessage::text(
                AiRole::Assistant,
                format!("{reason}\n{outcome}"),
            ));
            return Ok(proposal);
        }
    }
    conversation.push(AiMessage::text(AiRole::Assistant, if proposal.is_some() {
  "I reached the assistant's tool-round limit. The staged proposal is available for review, but I may not have completed all checks. No changes have been applied."
 } else {
  "I reached the assistant's tool-round limit before finishing. No changes have been proposed. Try asking for a smaller step, or ask me to continue."
 }));
    Ok(proposal)
}
/// Typed defaults are normalized before keying. The cache belongs to one loop only.
#[derive(Default)]
struct ReadToolCache(
    std::collections::HashMap<(u64, String, String), Value>,
    usize,
);
impl ReadToolCache {
    fn key(state: &AiDesignState, call: &AiToolCall) -> Option<(u64, String, String)> {
        let args = match call.name.as_str() {
            "inspect_design_region" => serde_json::to_value(
                serde_json::from_value::<super::design_inspection::InspectDesignRegionRequest>(
                    call.arguments.clone(),
                )
                .ok()?,
            )
            .ok()?,
            "get_design_context" => serde_json::to_value(
                serde_json::from_value::<DesignContextRequest>(call.arguments.clone()).ok()?,
            )
            .ok()?,
            "get_objects" => serde_json::to_value(
                serde_json::from_value::<GetObjectsRequest>(call.arguments.clone()).ok()?,
            )
            .ok()?,
            "get_object_schema" => serde_json::to_value(
                serde_json::from_value::<super::tools::GetObjectSchemaRequest>(
                    call.arguments.clone(),
                )
                .ok()?,
            )
            .ok()?,
            _ => return None,
        };
        Some((state.revision, call.name.clone(), args.to_string()))
    }
    fn execute(
        &mut self,
        state: &AiDesignState,
        call: &AiToolCall,
        proposal: Option<&AiProposal>,
    ) -> (Value, Option<AiProposal>, bool) {
        if call.name == "get_object_schema" && call.arguments.get("object_types").is_some() {
            let types = serde_json::from_value::<super::tools::GetObjectSchemaRequest>(
                call.arguments.clone(),
            )
            .map_err(|e| e.to_string())
            .and_then(|r| r.types());
            let types = match types {
                Ok(v) => v,
                Err(e) => return (json!({"success":false,"error":e}), None, false),
            };
            let mut schemas = Vec::new();
            for name in types {
                let single = AiToolCall {
                    name: call.name.clone(),
                    call_id: call.call_id.clone(),
                    arguments: json!({"object_type":name}),
                };
                let result = self.execute(state, &single, proposal).0;
                if result["success"] != true {
                    return (result, None, false);
                }
                schemas.push(result["schema"].clone());
            }
            return (json!({"success":true,"schemas":schemas}), None, false);
        }
        let key = Self::key(state, call);
        if let Some(value) = key.as_ref().and_then(|key| self.0.get(key)) {
            self.1 += 1;
            return (value.clone(), None, false);
        }
        let result = execute_tool(state, call, proposal);
        if let Some(key) = key {
            self.0.insert(key, result.0.clone());
        }
        result
    }
}
fn execute_tool(
    state: &AiDesignState,
    call: &AiToolCall,
    proposal: Option<&AiProposal>,
) -> (Value, Option<AiProposal>, bool) {
    let parse = |error: String| (json!({"success":false,"error":error}), None, false);
    match call.name.as_str() {
        "inspect_design_region" => match serde_json::from_value::<
            super::design_inspection::InspectDesignRegionRequest,
        >(call.arguments.clone())
        .map_err(|e| e.to_string())
        .and_then(|r| {
            super::design_inspection::inspect_design_region(state, r).map_err(|e| format!("{e:?}"))
        }) {
            Ok(v) => (json!({"success":true,"region":v}), None, false),
            Err(e) => parse(e),
        },
        "get_design_context" => {
            match serde_json::from_value::<DesignContextRequest>(call.arguments.clone())
                .map_err(|e| e.to_string())
                .and_then(|r| get_design_context(state, r).map_err(|e| format!("{e:?}")))
            {
                Ok(v) => (
                    json!({"success":true,"context":super::wire::context(v)}),
                    None,
                    false,
                ),
                Err(e) => parse(e),
            }
        }
        "get_objects" => match serde_json::from_value::<GetObjectsRequest>(call.arguments.clone())
            .map_err(|e| e.to_string())
            .and_then(|r| get_objects(state, r).map_err(|e| format!("{e:?}")))
        {
            Ok(v) => (
                json!({"success":true,"objects":super::wire::objects(v)}),
                None,
                false,
            ),
            Err(e) => parse(e),
        },
        "get_object_schema" => {
            let name = call
                .arguments
                .get("object_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            let ty = ObjectType::values()
                .into_iter()
                .find(|v| format!("{v:?}") == name);
            match ty
                .ok_or_else(|| format!("unknown object type: {name}"))
                .and_then(|v| get_object_schema(state, v).map_err(|e| format!("{e:?}")))
            {
                Ok(v) => (
                    json!({"success":true,"schema":super::wire::schema(v)}),
                    None,
                    false,
                ),
                Err(e) => parse(e),
            }
        }
        "analyze_design" => match analyze_design(
            state,
            proposal,
            if proposal.is_some() {
                DesignAnalysisScope::Proposal
            } else {
                DesignAnalysisScope::ActiveMask
            },
        ) {
            Ok(v) => (json!({"success":true,"analysis":v}), None, false),
            Err(e) => parse(format!("{e:?}")),
        },
        "propose_transaction" => {
            let tx = match serde_json::from_value::<crate::operations::OperationTransaction>(
                call.arguments.clone(),
            ) {
                Ok(tx) => tx,
                Err(error) => {
                    return (
                        json!({"success":false,"diagnostics":error.to_string(),"failure_class":"ModelDecision"}),
                        None,
                        true,
                    )
                }
            };
            match match proposal {
                Some(base) => super::propose_refinement(state, base, tx),
                None => propose_transaction(state, tx),
            }
            .map_err(|e| serde_json::to_string(&e).unwrap_or_else(|_| format!("{e:?}")))
            {
                Ok(p) => {
                    let analysis =
                        analyze_design(state, Some(&p), DesignAnalysisScope::Proposal).ok();
                    let summary = json!({"success":true,"proposal_id":p.proposal_id,"base_revision":p.base_revision,"description":p.transaction.description,"operation_count":p.summary.operation_count,"affected_objects":p.summary.affected_objects,"refinement_round":p.refinement_round,"design_diagnostics":analysis.map(|v|v.diagnostics)});
                    (summary, Some(p), false)
                }
                Err(e) => (json!({"success":false,"diagnostics":e}), None, true),
            }
        }
        _ => parse(format!("unsupported tool: {}", call.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct RepeatingProvider(AtomicUsize);
    impl AiProvider for RepeatingProvider {
        fn run_turn(
            &self,
            request: AiTurnRequest,
        ) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>>
        {
            let round = self.0.fetch_add(1, Ordering::SeqCst);
            assert!(!request.instructions.contains("tool rounds remaining"));
            Box::pin(async move {
                Ok(super::super::AiProviderResponse {
                    content: vec![AiContent::ToolCall(AiToolCall {
                        call_id: format!("call_{round}"),
                        name: "get_object_schema".into(),
                        arguments: json!({"object_type":"Button"}),
                    })],
                    usage: Default::default(),
                })
            })
        }
    }
    #[test]
    fn round_limit_preserves_conversation_and_returns_a_clear_stop_message() {
        let provider = RepeatingProvider(AtomicUsize::new(0));
        let state = AiDesignState {
            revision: 0,
            pool: Default::default(),
            object_info: Default::default(),
            vt_version: ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
            mask_size: 480,
            selected: None,
        };
        let mut conversation = Conversation::default();
        let proposal =
            run_loop(&provider, &state, &mut conversation, AiSettings::default()).unwrap();
        assert!(proposal.is_none());
        assert_eq!(provider.0.load(Ordering::SeqCst), MAX_TOOL_ROUNDS);
        assert!(conversation.messages.len() <= MAX_TOOL_ROUNDS * 2 + 3);
        assert!(
            matches!(&conversation.messages.last().unwrap().content[0], AiContent::Text(text) if text.contains("No changes have been proposed"))
        );
    }
    struct InvalidProposalProvider {
        calls: AtomicUsize,
        valid_first: bool,
    }
    impl AiProvider for InvalidProposalProvider {
        fn run_turn(
            &self,
            _: AiTurnRequest,
        ) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>>
        {
            let round = self.calls.fetch_add(1, Ordering::SeqCst);
            let arguments = if self.valid_first && round == 1 {
                json!({"schema_version":2,"operations":[{"type":"RenameObject","data":{"object":{"id":10},"name":"Renamed"}}]})
            } else {
                json!({"schema_version":2,"operations":[{"type":"SetProperty","data":{}}]})
            };
            Box::pin(async move {
                Ok(super::super::AiProviderResponse {
                    content: vec![AiContent::ToolCall(AiToolCall {
                        call_id: format!("proposal_{round}"),
                        name: if round == 0 {
                            "finish_inspection"
                        } else {
                            "propose_transaction"
                        }
                        .into(),
                        arguments,
                    })],
                    usage: Default::default(),
                })
            })
        }
    }
    #[test]
    fn repair_limit_exposes_validation_and_preserves_valid_work() {
        for valid_first in [false, true] {
            let provider = InvalidProposalProvider {
                calls: AtomicUsize::new(0),
                valid_first,
            };
            let mut state = AiDesignState {
                revision: 0,
                pool: Default::default(),
                object_info: Default::default(),
                vt_version: ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
                mask_size: 480,
                selected: None,
            };
            let mut button = crate::default_object(ObjectType::Button);
            button.mut_id().set_value(10).unwrap();
            state.pool.add(button);
            let mut conversation = Conversation::default();
            let proposal = run_loop(
                &provider,
                &state,
                &mut conversation,
                AiSettings {
                    mode: AiModelMode::Fast,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(proposal.is_some(), valid_first);
            assert_eq!(
                provider.calls.load(Ordering::SeqCst),
                4 + usize::from(valid_first)
            );
            assert!(
                matches!(&conversation.messages.last().unwrap().content[0], AiContent::Text(text) if text.contains("missing field `object`") && text.contains("No changes"))
            );
            assert!(state.object_info.is_empty());
        }
    }
}

#[cfg(test)]
mod efficiency_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct ProgressiveProvider(AtomicUsize);
    impl AiProvider for ProgressiveProvider {
        fn run_turn(
            &self,
            request: AiTurnRequest,
        ) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>>
        {
            let round = self.0.fetch_add(1, Ordering::SeqCst);
            assert!(request.instructions.contains("Project:"));
            assert!(!request.instructions.contains("\"properties\":"));
            let (name, arguments) = match round {
                0 => ("get_design_context", json!({})),
                1 => {
                    assert!(request.conversation.messages.iter().flat_map(|m| &m.content).any(|c| matches!(c, AiContent::ToolResult(r) if r.result["context"]["selection"]["id"] == 10)));
                    ("get_objects", json!({"ids":[10]}))
                }
                2 => {
                    assert!(request.conversation.messages.iter().flat_map(|m| &m.content).any(|c| matches!(c, AiContent::ToolResult(r) if r.result["objects"][0]["properties"].is_object())));
                    (
                        "propose_transaction",
                        json!({"schema_version":2,"operations":[{"type":"RenameObject","data":{"object":{"id":10},"name":"Renamed"}}]}),
                    )
                }
                _ => ("", json!({})),
            };
            let mut content = if round < 3 {
                vec![AiContent::ToolCall(AiToolCall {
                    call_id: format!("call_{round}"),
                    name: name.into(),
                    arguments,
                })]
            } else {
                vec![AiContent::Text("Staged for review".into())]
            };
            if round == 1 {
                content.push(AiContent::ToolCall(AiToolCall {
                    call_id: "finish".into(),
                    name: "finish_inspection".into(),
                    arguments: json!({}),
                }));
            }
            let total_tokens = [1000, 1400, 800, 50][round];
            Box::pin(async move {
                Ok(super::super::AiProviderResponse {
                    content,
                    usage: super::super::AiTokenUsage {
                        total_tokens,
                        ..Default::default()
                    },
                })
            })
        }
    }
    fn state() -> AiDesignState {
        let mut state = super::super::efficiency_tests::pool(0, 0);
        // A standalone button avoids unrelated reference validation in a synthetic pool.
        state.pool = Default::default();
        let mut button = crate::default_object(ObjectType::Button);
        button.mut_id().set_value(10).unwrap();
        state.pool.add(button);
        state
    }
    #[test]
    fn progressive_retrieval_stages_change_and_aggregates_calls() {
        let provider = ProgressiveProvider(AtomicUsize::new(0));
        let mut conversation = Conversation::default();
        conversation.push(AiMessage::text(AiRole::User, "Rename selected object"));
        let proposal = run_loop(
            &provider,
            &state(),
            &mut conversation,
            AiSettings::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(proposal.summary.operation_count, 1);
        // The complete tool chain remains present until the request has ended.
        assert_eq!(
            conversation
                .messages
                .iter()
                .flat_map(|m| &m.content)
                .filter(|c| matches!(c, AiContent::ToolResult(_)))
                .count(),
            4
        );
        assert_eq!(
            conversation
                .debug
                .lock()
                .unwrap()
                .user_request
                .tokens
                .total_tokens,
            3250
        );
        assert_eq!(
            conversation
                .debug
                .lock()
                .unwrap()
                .conversation
                .request_count,
            4
        );
        conversation.compact();
        assert_eq!(proposal.summary.operation_count, 1);
        assert!(conversation
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .all(|c| matches!(c, AiContent::Text(_))));
    }
    #[test]
    fn typed_cache_uses_revision_and_is_request_local() {
        let mut cache = ReadToolCache::default();
        let mut state = state();
        let call = AiToolCall {
            call_id: "one".into(),
            name: "get_objects".into(),
            arguments: json!({"ids":[10]}),
        };
        let first = cache.execute(&state, &call, None).0;
        assert_eq!(cache.execute(&state, &call, None).0, first);
        assert_eq!(cache.0.len(), 1);
        let explicit_defaults = AiToolCall {
            arguments: serde_json::to_value(GetObjectsRequest {
                ids: vec![10],
                ..Default::default()
            })
            .unwrap(),
            ..call.clone()
        };
        assert_eq!(
            ReadToolCache::key(&state, &call),
            ReadToolCache::key(&state, &explicit_defaults)
        );
        // Removing data without changing revision demonstrates the second call is served
        // from cache, not re-executed. Real edits always increment revision.
        state.pool = Default::default();
        assert_eq!(cache.execute(&state, &call, None).0, first);
        state.revision += 1;
        assert_eq!(cache.execute(&state, &call, None).0["success"], false);
        assert_eq!(
            ReadToolCache::default().execute(&state, &call, None).0["success"],
            false
        );
    }
    struct FailingProvider(AtomicUsize);
    impl AiProvider for FailingProvider {
        fn run_turn(
            &self,
            _: AiTurnRequest,
        ) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>>
        {
            let round = self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if round == 1 {
                    return Err(AiProviderError::Timeout);
                }
                Ok(super::super::AiProviderResponse {
                    content: if round == 0 {
                        vec![AiContent::ToolCall(AiToolCall {
                            call_id: "c".into(),
                            name: "get_objects".into(),
                            arguments: json!({"ids":[10]}),
                        })]
                    } else {
                        vec![AiContent::Text("Done".into())]
                    },
                    usage: super::super::AiTokenUsage {
                        total_tokens: 100,
                        ..Default::default()
                    },
                })
            })
        }
    }
    #[test]
    fn failed_worker_retains_usage_and_next_request_accumulates() {
        let provider = FailingProvider(AtomicUsize::new(0));
        let (sender, receiver) = std::sync::mpsc::channel();
        run_request(
            &provider,
            state(),
            Conversation::default(),
            AiSettings::default(),
            "first".into(),
            sender.clone(),
        );
        let AiWorkerEvent::Failed { conversation, .. } = receiver.recv().unwrap() else {
            panic!("expected failure")
        };
        assert_eq!(
            conversation
                .debug
                .lock()
                .unwrap()
                .user_request
                .tokens
                .total_tokens,
            100
        );
        assert!(conversation
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .all(|c| matches!(c, AiContent::Text(_))));
        run_request(
            &provider,
            state(),
            conversation,
            AiSettings::default(),
            "second".into(),
            sender,
        );
        let AiWorkerEvent::Finished { conversation, .. } = receiver.recv().unwrap() else {
            panic!("expected success")
        };
        let debug = conversation.debug.lock().unwrap();
        assert_eq!(debug.user_request.tokens.total_tokens, 100);
        assert_eq!(debug.conversation.tokens.total_tokens, 200);
    }
}

#[cfg(test)]
mod phase52_tests {
    use super::*;
    #[test]
    fn batched_schemas_are_bounded_sorted_and_cached_per_type() {
        let state = super::super::efficiency_tests::pool(0, 0);
        let mut cache = ReadToolCache::default();
        let call = |types: Value| AiToolCall {
            call_id: "batch".into(),
            name: "get_object_schema".into(),
            arguments: json!({"object_types":types}),
        };
        let first = cache
            .execute(
                &state,
                &call(json!(["OutputNumber", "NumberVariable", "Container"])),
                None,
            )
            .0;
        assert_eq!(first["success"], true);
        assert_eq!(
            first["schemas"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["Container", "NumberVariable", "OutputNumber"]
        );
        assert_eq!(cache.0.len(), 3);
        let second = cache
            .execute(&state, &call(json!(["OutputNumber", "Button"])), None)
            .0;
        assert_eq!(second["schemas"].as_array().unwrap().len(), 2);
        assert_eq!(cache.0.len(), 4);
        assert_eq!(cache.1, 1);
        assert_eq!(
            cache
                .execute(&state, &call(json!(vec!["Button"; 11])), None)
                .0["success"],
            false
        );
        assert_eq!(
            cache.execute(&state, &call(json!([])), None).0["success"],
            false
        );
        assert_eq!(
            cache.execute(&state, &call(json!(["Invalid"])), None).0["success"],
            false
        );
    }
}

#[cfg(test)]
mod phase52_workflows {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    struct Workflow {
        step: AtomicUsize,
        calls: Vec<(&'static str, Value)>,
        bodies: Mutex<Vec<Value>>,
        legacy: bool,
        state: AiDesignState,
    }
    impl AiProvider for Workflow {
        fn run_turn(
            &self,
            mut request: AiTurnRequest,
        ) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>>
        {
            let step = self.step.fetch_add(1, Ordering::SeqCst);
            if self.legacy {
                // Reconstruct pre-5.2 read payloads while using the same validated operations.
                let mut calls = std::collections::HashMap::new();
                for message in &mut request.conversation.messages {
                    for content in &mut message.content {
                        match content {
                            AiContent::ToolCall(call) => {
                                calls.insert(call.call_id.clone(), call.clone());
                            }
                            AiContent::ToolResult(result) => {
                                let call = &calls[&result.call_id];
                                result.result = match call.name.as_str() {
                                    "get_design_context" => {
                                        json!({"success":true,"context":get_design_context(&self.state,DesignContextRequest {include_references:false,..Default::default()}).unwrap()})
                                    }
                                    "get_objects" => {
                                        json!({"success":true,"objects":get_objects(&self.state,serde_json::from_value(call.arguments.clone()).unwrap()).unwrap()})
                                    }
                                    "get_object_schema" => {
                                        let ty = ObjectType::values()
                                            .into_iter()
                                            .find(|ty| {
                                                format!("{ty:?}")
                                                    == call.arguments["object_type"]
                                                        .as_str()
                                                        .unwrap()
                                            })
                                            .unwrap();
                                        json!({"success":true,"schema":get_object_schema(&self.state,ty).unwrap()})
                                    }
                                    _ => result.result.clone(),
                                };
                            }
                            _ => {}
                        }
                    }
                }
                let old_instructions = "You are an assistant for an ISO 11783 VT object-pool designer. Object names are symbolic references, while ObjectId remains the real identity. The project may contain many objects. Start with the active mask and selection; retrieve only relevant context. Batch known IDs in get_objects and request schemas only for types you need. Follow pagination when needed. Inspect context and schemas before guessing. All edits must use propose_transaction; it only stages a change, so never say it is applied. Never modify raw object JSON or invent unsupported properties. Position belongs to parent-child relationships. Use SetObjectReference for object-reference properties and preserve references unless asked otherwise. Submit related edits in one transaction. After a staged proposal you may receive its rendered preview and deterministic design diagnostics. Inspect overlap, clipping, alignment, spacing, readability and requested emphasis. If changes are genuinely needed, submit a normal corrective propose_transaction; it is prepared against the staged candidate. Do not change variable or macro references unless needed for the user's request, and say no visual changes are required when appropriate.";
                let summary =
                    serde_json::to_string(&super::super::snapshot::project_summary(&self.state))
                        .unwrap();
                request.instructions = format!("{old_instructions}\nProject: {summary}\nYou have {} tool rounds remaining for this request. Batch independent inspections, avoid repeating successful calls, and finish with a concise summary as soon as the request is satisfied. A successful proposal is staged for user review; do not keep refining unless necessary.", MAX_TOOL_ROUNDS-step);
                request.tools = super::super::tools::phase51_tool_definitions();
            }
            let mut body = super::super::providers::openai::request_body(&request);
            if self.legacy {
                body.as_object_mut().unwrap().remove("prompt_cache_key");
            }
            request
                .conversation
                .debug
                .lock()
                .unwrap()
                .record_request(&body);
            self.bodies.lock().unwrap().push(body);
            let mut content = if let Some((name, args)) = self.calls.get(step) {
                vec![AiContent::ToolCall(AiToolCall {
                    call_id: format!("call_{step}"),
                    name: (*name).into(),
                    arguments: args.clone(),
                })]
            } else {
                vec![AiContent::Text("Staged for review".into())]
            };
            if self
                .calls
                .get(step + 1)
                .is_some_and(|(name, _)| *name == "propose_transaction")
            {
                content.push(AiContent::ToolCall(AiToolCall {
                    call_id: format!("finish_{step}"),
                    name: "finish_inspection".into(),
                    arguments: json!({}),
                }));
            }
            Box::pin(async move {
                Ok(super::super::AiProviderResponse {
                    content,
                    usage: Default::default(),
                })
            })
        }
    }
    #[test]
    fn representative_large_pool_workflows() {
        use ag_iso_stack::object_pool::{object::Object, ObjectId};
        let mut state = super::super::efficiency_tests::pool(384, 20);
        // Valid font target for all existing displays and the newly created readout.
        let mut font = crate::default_object(ObjectType::FontAttributes);
        font.mut_id().set_value(500).unwrap();
        state.pool.add(font);
        for object in state.pool.objects_mut() {
            if let Object::OutputString(v) = object {
                v.font_attributes = ObjectId::new(500).unwrap();
            }
        }
        let tasks = [
      ("A",json!([{"type":"SetProperty","data":{"object":{"id":10},"property":"width","value":100}}])),
      ("B",json!([
        {"type":"CreateObject","data":{"object_id":600,"object_type":"OutputNumber","name":"Readout"}},
        {"type":"CreateObject","data":{"object_id":601,"object_type":"NumberVariable","name":"Rate"}},
        {"type":"SetObjectReference","data":{"object":{"id":600},"property":"font_attributes","reference":{"id":500}}},
        {"type":"SetObjectReference","data":{"object":{"id":600},"property":"variable_reference","reference":{"id":601}}},
        {"type":"AddChild","data":{"parent":{"id":2},"child":{"id":600},"x":30,"y":80}}
      ])),
      ("C",json!([10,11,12].into_iter().map(|id|json!({"type":"SetChildPosition","data":{"parent_id":2,"child_id":id,"x":30,"y":id*20}})).collect::<Vec<_>>()))
    ];
        for (task, operations) in tasks {
            let mut totals = Vec::new();
            for legacy in [true, false] {
                let mut calls = vec![("get_design_context", json!({}))];
                match (task, legacy) {
                    ("A", true) => calls.push(("get_objects", json!({"ids":[10]}))),
                    ("B", true) => {
                        for ty in ["OutputNumber", "NumberVariable", "Container"] {
                            calls.push(("get_object_schema", json!({"object_type":ty})));
                        }
                    }
                    ("B", false) => calls.push((
                        "get_object_schema",
                        json!({"object_types":["OutputNumber","NumberVariable","Container"]}),
                    )),
                    ("C", true) => {
                        for id in [10, 11, 12] {
                            calls.push(("get_objects", json!({"ids":[id]})));
                        }
                    }
                    ("C", false) => calls.push(("get_objects", json!({"ids":[10,11,12]}))),
                    _ => {}
                }
                calls.push((
                    "propose_transaction",
                    json!({"schema_version":2,"operations":operations}),
                ));
                let provider = Workflow {
                    step: AtomicUsize::new(0),
                    calls,
                    bodies: Mutex::new(Vec::new()),
                    legacy,
                    state: state.clone(),
                };
                let mut conversation = Conversation::default();
                conversation.push(AiMessage::text(AiRole::User, format!("Task {task}")));
                let proposal =
                    run_loop(&provider, &state, &mut conversation, AiSettings::default()).unwrap();
                assert!(
                    proposal.is_some(),
                    "Task {task}: {:?}",
                    conversation.messages
                );
                let bodies = provider.bodies.lock().unwrap();
                let bytes: Vec<_> = bodies.iter().map(|b| b.to_string().len()).collect();
                totals.push(bytes.iter().sum::<usize>());
                let debug = conversation.debug.lock().unwrap();
                println!("Task {task} {}: rounds {}, calls {}, request bytes {:?}, sum {}; final components {:?}",if legacy {"reconstructed baseline"} else {"compact batched"},bodies.len(),debug.tool_calls,bytes,totals.last().unwrap(),debug.rounds.last().unwrap());
                assert!(legacy || bodies.len() <= 4);
            }
            assert!(totals[1] < totals[0]);
        }
    }
}

#[cfg(test)]
mod phase54_tests {
    use super::*;
    use std::sync::Mutex;
    struct Script {
        rounds: Mutex<Vec<Vec<(&'static str, Value)>>>,
        requests: Mutex<Vec<AiTurnRequest>>,
    }
    impl AiProvider for Script {
        fn run_turn(
            &self,
            request: AiTurnRequest,
        ) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>>
        {
            let mut requests = self.requests.lock().unwrap();
            let index = requests.len();
            requests.push(request);
            let mut rounds = self.rounds.lock().unwrap();
            let calls = if rounds.is_empty() {
                vec![]
            } else {
                rounds.remove(0)
            };
            let content = if calls.is_empty() {
                vec![AiContent::Text("Done".into())]
            } else {
                calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, args))| {
                        AiContent::ToolCall(AiToolCall {
                            call_id: format!("r{index}_{i}"),
                            name: name.into(),
                            arguments: args,
                        })
                    })
                    .collect()
            };
            Box::pin(async move {
                Ok(super::super::AiProviderResponse {
                    content,
                    usage: super::super::AiTokenUsage {
                        input_tokens: 100,
                        cached_input_tokens: 40,
                        output_tokens: 10,
                        total_tokens: 110,
                        ..Default::default()
                    },
                })
            })
        }
    }
    fn tx(valid: bool) -> Value {
        if valid {
            json!({"schema_version":2,"operations":[{"type":"RenameObject","data":{"object":{"id":10},"name":"Renamed"}}]})
        } else {
            json!({"schema_version":2,"operations":[{"type":"SetProperty","data":{}}]})
        }
    }
    fn fixture() -> AiDesignState {
        let mut state = super::super::efficiency_tests::pool(0, 0);
        let mut button = crate::default_object(ObjectType::Button);
        button.mut_id().set_value(10).unwrap();
        state.pool.add(button);
        state
    }
    fn script(rounds: Vec<Vec<(&'static str, Value)>>) -> Script {
        Script {
            rounds: Mutex::new(rounds),
            requests: Mutex::new(vec![]),
        }
    }
    #[test]
    fn normal_and_forced_modes_lazy_tools_and_usage() {
        for mode in [
            AiModelMode::Automatic,
            AiModelMode::Fast,
            AiModelMode::Strong,
        ] {
            let provider = script(vec![
                vec![
                    ("get_objects", json!({"ids":[10]})),
                    ("finish_inspection", json!({})),
                ],
                vec![("propose_transaction", tx(true))],
            ]);
            let mut c = Conversation::default();
            let p = run_loop(
                &provider,
                &fixture(),
                &mut c,
                AiSettings {
                    mode,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(p.is_some());
            let requests = provider.requests.lock().unwrap();
            assert_eq!(requests.len(), 3);
            let expected = if mode == AiModelMode::Strong {
                AiModelRole::Strong
            } else {
                AiModelRole::Fast
            };
            assert!(requests.iter().all(|r| r.settings.role == expected));
            assert!(!requests[0]
                .tools
                .iter()
                .any(|v| v["name"] == "propose_transaction"));
            assert!(requests[1]
                .tools
                .iter()
                .any(|v| v["name"] == "propose_transaction"));
            assert!(
                serde_json::to_string(&requests[0].tools).unwrap().len()
                    < serde_json::to_string(&super::super::tools::tool_definitions())
                        .unwrap()
                        .len()
            );
            let d = c.debug.lock().unwrap();
            assert_eq!(d.model_usage.len(), 1);
            let u = d.model_usage.values().next().unwrap();
            assert_eq!(u.request_count, 3);
            assert_eq!(u.tokens.input_tokens, 300);
            assert_eq!(u.tokens.cached_input_tokens, 120);
            assert_eq!(u.tokens.uncached_input_tokens(), 180);
            assert_eq!(u.tokens.output_tokens, 30);
            assert_eq!(u.tokens.total_tokens, 330);
        }
    }
    #[test]
    fn validation_escalates_once_with_compact_handoff_and_shared_cache() {
        let provider = script(vec![
            vec![
                ("get_objects", json!({"ids":[10]})),
                ("finish_inspection", json!({})),
            ],
            vec![("propose_transaction", tx(false))],
            vec![("propose_transaction", tx(false))],
            vec![
                ("get_objects", json!({"ids":[10]})),
                ("propose_transaction", tx(true)),
            ],
        ]);
        let mut c = Conversation::default();
        assert!(
            run_loop(&provider, &fixture(), &mut c, AiSettings::default())
                .unwrap()
                .is_some()
        );
        let r = provider.requests.lock().unwrap();
        assert!(r[..3].iter().all(|r| r.settings.role == AiModelRole::Fast));
        assert!(r[3..]
            .iter()
            .all(|r| r.settings.role == AiModelRole::Strong));
        assert_eq!(r[3].conversation.messages.len(), 2);
        assert!(r[3]
            .conversation
            .messages
            .iter()
            .all(|m| m.content.iter().all(|c| matches!(c, AiContent::Text(_)))));
        let d = c.debug.lock().unwrap();
        assert_eq!(
            d.task.as_ref().unwrap().escalation,
            Some(AiEscalationReason::RepeatedValidationFailure)
        );
        assert_eq!(d.tools["get_objects"].cache_hits, 1);
        assert_eq!(
            d.entries
                .iter()
                .filter(|e| e.label == "Model escalation")
                .count(),
            1
        );
        assert_eq!(d.user_request.tokens.input_tokens, 500);
    }
    #[test]
    fn repeated_inspection_warns_before_escalating() {
        let mut rounds = vec![vec![("get_object_schema", json!({"object_type":"Button"}))]; 4];
        rounds.push(vec![("propose_transaction", tx(true))]);
        let provider = script(rounds);
        let mut c = Conversation::default();
        assert!(
            run_loop(&provider, &fixture(), &mut c, AiSettings::default())
                .unwrap()
                .is_some()
        );
        let r = provider.requests.lock().unwrap();
        assert!(r[2]
            .conversation
            .task_state
            .as_ref()
            .unwrap()
            .harness_hint
            .is_some());
        assert_eq!(r[4].settings.role, AiModelRole::Strong);
        assert_eq!(
            c.debug.lock().unwrap().task.as_ref().unwrap().escalation,
            Some(AiEscalationReason::InspectionLoop)
        );
    }
    #[test]
    fn focused_cache_normalizes_defaults_and_rejects_unrelated_mask_reads() {
        let mut state = fixture();
        let mut other = crate::default_object(ObjectType::DataMask);
        other.mut_id().set_value(3).unwrap();
        state.pool.add(other);
        let mut cache = ReadToolCache::default();
        let call = AiToolCall {
            call_id: "a".into(),
            name: "inspect_design_region".into(),
            arguments: json!({"root_id":2}),
        };
        assert_eq!(cache.execute(&state, &call, None).0["success"], true);
        let normalized = AiToolCall {
            arguments: serde_json::to_value(
                super::super::design_inspection::InspectDesignRegionRequest {
                    root_id: 2,
                    ..Default::default()
                },
            )
            .unwrap(),
            ..call
        };
        cache.execute(&state, &normalized, None);
        assert_eq!(cache.1, 1);
        state.revision += 1;
        cache.execute(&state, &normalized, None);
        assert_eq!(cache.1, 1);
        let provider = script(vec![vec![
            ("inspect_design_region", json!({"root_id":3})),
            ("finish_inspection", json!({})),
        ]]);
        let mut c = Conversation::default();
        c.push(AiMessage::text(AiRole::User, "Modify DataMask 2"));
        run_loop(&provider, &state, &mut c, AiSettings::default()).unwrap();
        let requests = provider.requests.lock().unwrap();
        assert!(requests[1]
            .conversation
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .any(|v| matches!(v,AiContent::ToolResult(r) if r.result["success"]==false)));
        assert!(!c
            .debug
            .lock()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .inspected_roots
            .contains(&3));
    }
    #[test]
    fn cancellation_and_transport_errors_do_not_trigger_quality_escalation() {
        struct Unavailable;
        impl AiProvider for Unavailable {
            fn run_turn(
                &self,
                _: AiTurnRequest,
            ) -> super::super::AiProviderFuture<
                Result<super::super::AiProviderResponse, AiProviderError>,
            > {
                Box::pin(async { Err(AiProviderError::Timeout) })
            }
        }
        let mut c = Conversation::default();
        assert!(matches!(
            run_loop(&Unavailable, &fixture(), &mut c, AiSettings::default()),
            Err(AiProviderError::Timeout)
        ));
        let d = c.debug.lock().unwrap();
        assert_eq!(d.failure_class, Some(AiFailureClass::ProviderTransport));
        assert_eq!(d.task.as_ref().unwrap().validation_failures, 0);
        assert!(d.task.as_ref().unwrap().escalation.is_none());
        drop(d);
        c.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let provider = script(vec![]);
        assert!(matches!(
            run_loop(&provider, &fixture(), &mut c, AiSettings::default()),
            Err(AiProviderError::Cancelled)
        ));
        assert!(provider.requests.lock().unwrap().is_empty());
    }
    #[test]
    fn explicit_help_respects_forced_modes() {
        for mode in [
            AiModelMode::Automatic,
            AiModelMode::Fast,
            AiModelMode::Strong,
        ] {
            let provider = script(vec![
                vec![
                    (
                        "request_stronger_model",
                        json!({"reason":"Conflicting relationship constraints"}),
                    ),
                    ("finish_inspection", json!({})),
                ],
                vec![("propose_transaction", tx(true))],
            ]);
            let mut c = Conversation::default();
            assert!(run_loop(
                &provider,
                &fixture(),
                &mut c,
                AiSettings {
                    mode,
                    ..Default::default()
                }
            )
            .unwrap()
            .is_some());
            let d = c.debug.lock().unwrap();
            assert_eq!(
                d.task.as_ref().unwrap().escalation.is_some(),
                mode == AiModelMode::Automatic
            );
            let r = provider.requests.lock().unwrap();
            assert_eq!(
                r[1].settings.role,
                if mode == AiModelMode::Fast {
                    AiModelRole::Fast
                } else {
                    AiModelRole::Strong
                }
            );
        }
    }
    #[test]
    fn explicit_target_overrides_selection_without_mutating_it() {
        let state = fixture();
        let original = state.selected;
        let provider = script(vec![
            vec![
                ("get_design_context", json!({"scope":"Selection"})),
                ("finish_inspection", json!({})),
            ],
            vec![("propose_transaction", tx(true))],
        ]);
        let mut c = Conversation::default();
        c.push(AiMessage::text(AiRole::User, "Modify DataMask 2"));
        run_loop(&provider, &state, &mut c, AiSettings::default()).unwrap();
        assert_eq!(state.selected, original);
        let r = provider.requests.lock().unwrap();
        let result = r[1]
            .conversation
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .find_map(|c| {
                if let AiContent::ToolResult(r) = c {
                    Some(&r.result)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            result["context"].get("selection").is_none()
                || result["context"]["selection"].is_null()
        );
        assert_eq!(result["context"]["objects"][0]["id"], 2);
        assert_eq!(
            c.debug
                .lock()
                .unwrap()
                .task
                .as_ref()
                .unwrap()
                .explicit_target
                .as_ref()
                .unwrap()
                .id,
            2
        );
    }
}
