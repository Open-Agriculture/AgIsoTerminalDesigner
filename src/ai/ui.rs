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
    settings: AiSettings,
) -> Result<Option<AiProposal>, AiProviderError> {
    run_loop_with_budget(provider, state, conversation, settings, MAX_TOOL_ROUNDS)
}
fn run_loop_with_budget(
    provider: &dyn AiProvider,
    state: &AiDesignState,
    conversation: &mut Conversation,
    mut settings: AiSettings,
    max_rounds: usize,
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
    let resume = super::task_state::continuation_instruction(&goal).filter(|_| {
        conversation
            .task_state
            .as_ref()
            .is_some_and(|t| !t.is_complete)
    });
    let mut task = if let Some(instruction) = resume.as_ref() {
        let mut task = conversation.task_state.clone().unwrap();
        if !instruction.is_empty() {
            task.continuation_instructions.push(instruction.clone());
        }
        task.revalidate(state);
        task.checkpoint = false;
        task
    } else {
        conversation.pending_proposal = None;
        AiTaskState::new(state, goal.clone())
    };
    let goal = task.goal.clone();
    // Tiny local targets can be established without spending a provider round.
    if super::snapshot::project_summary(state).project.object_count <= 4 {
        task.refresh_target(state);
    }
    task.capture_fingerprints(state);
    conversation.task_state = Some(task.clone());
    if super::task_state::explicit_ids(&goal).len() == 1 && task.explicit_target.is_none() {
        conversation.push(AiMessage::text(AiRole::Assistant,"The explicitly requested object does not exist in this project revision. Please provide its current ID."));
        return Ok(None);
    }
    settings.role = if settings.mode == AiModelMode::Strong || task.escalation.is_some() {
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
    let mut handoff: Option<(usize, Value)> = resume
        .as_ref()
        .map(|_| (conversation.messages.len(), task.handoff()));
    if resume.is_some() {
        conversation.replay_generation += 1;
    }
    let mut proposal: Option<AiProposal> = conversation.pending_proposal.clone();
    if proposal
        .as_ref()
        .is_some_and(|p| p.base_revision != state.revision)
    {
        proposal = proposal.and_then(|p| propose_transaction(state, p.transaction).ok());
    }
    let mut phase_rounds = [0usize; 4];
    let mut calls = 0;
    let mut repairs: usize = 0;
    for round in 0..max_rounds.min(MAX_TOOL_ROUNDS) {
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
        let phase_index = match task.phase {
            AiTaskPhase::Inspect => 0,
            AiTaskPhase::Propose => 1,
            AiTaskPhase::Repair => 2,
            AiTaskPhase::Review => 3,
        };
        let soft_budget = [2, 2, 2, 1][phase_index];
        if phase_rounds[phase_index] >= soft_budget {
            task.harness_hint = Some(match task.phase {
                AiTaskPhase::Inspect => "Inspection budget reached. Finish inspection unless one specific missing fact blocks the proposal.",
                AiTaskPhase::Propose => "Proposal budget reached. Submit the transaction using loaded schemas now.",
                AiTaskPhase::Repair => "Repair only the reported validation errors; do not restart exploration.",
                AiTaskPhase::Review => "Finish the review and summarize the staged proposal.",
            }.into());
        }
        phase_rounds[phase_index] += 1;
        task.round_reason = match task.phase {
            AiTaskPhase::Inspect if task.inspection_rounds == 0 => "initial inspection",
            AiTaskPhase::Inspect => "design planning",
            AiTaskPhase::Propose => "proposal",
            AiTaskPhase::Repair => "validation repair",
            AiTaskPhase::Review => "proposal review",
        }
        .into();
        task.provider_rounds_used += 1;
        conversation.task_state = Some(task.clone());
        conversation.pending_proposal = proposal.clone();
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
            task.is_complete = proposal.is_some() || task.tool_calls_used == 0;
            task.checkpoint = !task.is_complete;
            conversation.task_state = Some(task.clone());
            conversation.pending_proposal = proposal.clone();
            conversation.debug.lock().unwrap().task = Some(task.clone());
            return Ok(proposal);
        }
        calls += tool_calls.len();
        task.tool_calls_used += tool_calls.len();
        if calls > MAX_TOOL_CALLS_PER_REQUEST {
            break;
        }
        let mut results = Vec::new();
        let mut stop_reason = None;
        for mut call in tool_calls {
            if let Some(args) = call.arguments.as_object_mut() {
                if call.name != "propose_transaction" {
                    args.retain(|_, value| !value.is_null());
                }
            }
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
            let empty_read = task.screen_is_empty
                && match call.name.as_str() {
                    "get_design_context" | "inspect_design_region" => {
                        call.arguments["scope"] != "EntirePool"
                            && call.arguments["root_id"]
                                .as_u64()
                                .is_none_or(|id| Some(id as u16) == task.target_root)
                    }
                    "get_objects" => call.arguments["ids"].as_array().is_some_and(|ids| {
                        ids.iter()
                            .all(|id| id.as_u64().map(|id| id as u16) == task.target_root)
                    }),
                    _ => false,
                };
            let (mut result, proposed, repair) = if empty_read {
                (
                    json!({"success":true,"target":task.target_root,"screen_is_empty":true,"direct_child_count":0,"hint":"No local styles or children exist. Finish inspection with required_object_types."}),
                    None,
                    false,
                )
            } else if outside_target {
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
                let types = call.arguments["required_object_types"].as_array();
                if let Some(types) =
                    types.filter(|types| types.len() <= super::tools::MAX_SCHEMA_TYPES_PER_CALL)
                {
                    let schema_call = AiToolCall {
                        call_id: call.call_id.clone(),
                        name: "get_object_schema".into(),
                        arguments: json!({"object_types":types}),
                    };
                    let mut result = if types.is_empty() {
                        json!({"success":true,"schemas":[]})
                    } else {
                        cache.execute(state, &schema_call, None).0
                    };
                    if result["success"] == true {
                        task.observe(&schema_call, &result);
                        task.phase = AiTaskPhase::Propose;
                        result["phase"] = json!("Propose");
                    }
                    (result, None, false)
                } else {
                    (
                        json!({"success":false,"error":"Provide required_object_types: at most 10 canonical types used by your plan (empty for schema-free operations)."}),
                        None,
                        false,
                    )
                }
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
            let normalized = (call.name == "propose_transaction"
                && call.arguments.get("schema_version").is_none())
                || (call.name == "get_object_schema"
                    && call.arguments["object_types"].as_array().is_some_and(|a| {
                        a.len() > super::tools::MAX_SCHEMA_TYPES_PER_CALL
                            || a.iter().collect::<std::collections::HashSet<_>>().len() != a.len()
                    }));
            if normalized {
                task.normalization_count += 1;
                conversation.debug.lock().unwrap().failure_class =
                    Some(AiFailureClass::HarnessNormalization);
            }
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
            } else {
                stop_reason = Some(
                    "Inspection stopped because repeated reads made no useful progress.".into(),
                );
            }
        }
        if !task.screen_is_empty
            && task.target_root.is_some_and(|id| {
                task.inspected_roots.contains(&id) || task.relevant_objects.contains(&id)
            })
        {
            task.refresh_target(state);
        }
        task.capture_fingerprints(state);
        conversation.task_state = Some(task.clone());
        conversation.pending_proposal = proposal.clone();
        conversation.debug.lock().unwrap().task = Some(task.clone());
        if let Some(reason) = stop_reason {
            task.checkpoint = true;
            conversation.task_state = Some(task.clone());
            conversation.debug.lock().unwrap().task = Some(task.clone());
            let outcome = if proposal.is_some() {
                "The previous valid proposal is still staged for review. No changes have been applied."
            } else {
                "No changes have been proposed or applied."
            };
            conversation.push(AiMessage::text(
                AiRole::Assistant,
                format!(
                    "{reason}\n{outcome} AI task paused before completion. Continue to resume."
                ),
            ));
            return Ok(proposal);
        }
    }
    task.checkpoint = true;
    conversation.task_state = Some(task.clone());
    conversation.pending_proposal = proposal.clone();
    conversation.debug.lock().unwrap().task = Some(task);
    conversation.push(AiMessage::text(
        AiRole::Assistant,
        "AI task paused before completion. Continue to resume from the current state.",
    ));
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
        if call.name == "get_object_schema" {
            let names = call.arguments["object_types"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(call.arguments.get("object_type"));
            let invalid: Vec<_> = names
                .filter_map(Value::as_str)
                .filter(|name| {
                    !ObjectType::values()
                        .iter()
                        .any(|ty| format!("{ty:?}") == *name)
                })
                .collect();
            if !invalid.is_empty() {
                let suggestions: Vec<_> = invalid
                    .iter()
                    .filter_map(|name| match *name {
                        "NumericOutput" => Some("OutputNumber"),
                        "SoftKey" => Some("Key"),
                        _ => None,
                    })
                    .collect();
                return (
                    json!({"success":false,"invalid_types":invalid,"suggestions":suggestions}),
                    None,
                    false,
                );
            }
        }
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
            let mut arguments = call.arguments.clone();
            if let Some(args) = arguments.as_object_mut() {
                args.insert(
                    "schema_version".into(),
                    json!(crate::operations::CURRENT_OPERATION_SCHEMA_VERSION),
                );
            }
            let tx = match serde_json::from_value::<crate::operations::OperationTransaction>(
                arguments,
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
        assert!(provider.0.load(Ordering::SeqCst) <= MAX_TOOL_ROUNDS);
        assert!(conversation.messages.len() <= MAX_TOOL_ROUNDS * 2 + 3);
        assert!(
            matches!(&conversation.messages.last().unwrap().content[0], AiContent::Text(text) if text.contains("Continue to resume"))
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
            let arguments = if round == 0 {
                json!({"required_object_types":[]})
            } else if self.valid_first && round == 1 {
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
