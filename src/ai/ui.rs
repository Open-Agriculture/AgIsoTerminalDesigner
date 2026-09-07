//! Application-facing orchestration state. It owns snapshots, never an EditorProject.
use super::{get_design_context, get_object_schema, get_objects, propose_transaction, analyze_design, AiContent, AiDesignState, AiMessage, AiProposal, AiProvider, AiProviderError, AiRole, AiSettings, AiToolCall, AiToolResult, AiTurnRequest, Conversation, DesignContextRequest, DesignAnalysisScope, GetObjectsRequest};
use super::backend::{assistant_message, SYSTEM_INSTRUCTIONS};
use ag_iso_stack::object_pool::ObjectType;
use serde_json::{json, Value};
use std::sync::mpsc::Sender;

pub const MAX_TOOL_ROUNDS: usize = 8;
pub const MAX_TOOL_CALLS_PER_REQUEST: usize = 24;
pub const MAX_PROPOSAL_REPAIR_ATTEMPTS: usize = 2;
pub const MAX_VISUAL_REFINEMENT_ROUNDS: u8 = 2;
#[derive(Debug, Clone)] pub enum AiRequestState { Idle, Running { request_id: String }, ProposalReady, Error }
#[derive(Debug, Clone)] pub struct AiAssistantState { pub request_state: AiRequestState, pub conversation: Conversation, pub active_proposal: Option<AiProposal>, pub last_error: Option<String> }
impl Default for AiAssistantState { fn default() -> Self { Self { request_state: AiRequestState::Idle, conversation: Conversation::default(), active_proposal: None, last_error: None } } }
#[derive(Debug, Clone)] pub enum AiWorkerEvent { Finished { request_id: String, conversation: Conversation, proposal: Option<AiProposal> }, Failed { request_id: String, error: String } }

pub fn run_request(provider: &dyn AiProvider, design: AiDesignState, mut conversation: Conversation, settings: AiSettings, request_id: String, sender: Sender<AiWorkerEvent>) {
 log::info!("AI request started: model={}", settings.model);
 let result = run_loop(provider, &design, &mut conversation, settings);
 match result { Ok(proposal) => { log::info!("AI request completed: proposal={}", proposal.is_some()); let _=sender.send(AiWorkerEvent::Finished { request_id, conversation, proposal }); }, Err(error) => { log::warn!("AI request failed: {error}"); let _=sender.send(AiWorkerEvent::Failed { request_id, error:error.to_string() }); } }
}
fn run_loop(provider: &dyn AiProvider, state: &AiDesignState, conversation: &mut Conversation, settings: AiSettings) -> Result<Option<AiProposal>, AiProviderError> {
 let runtime = tokio::runtime::Runtime::new().map_err(|e| AiProviderError::Http(e.to_string()))?;
 let mut proposal = None; let mut calls = 0; let mut repairs = 0;
 for _round in 0..MAX_TOOL_ROUNDS {
  let response = runtime.block_on(provider.run_turn(AiTurnRequest { instructions: SYSTEM_INSTRUCTIONS.into(), conversation: conversation.clone(), tools: super::tools::tool_definitions(), settings: settings.clone() }))?;
  let tool_calls: Vec<_> = response.content.iter().filter_map(|c| if let AiContent::ToolCall(call)=c { Some(call.clone()) } else { None }).collect();
  conversation.push(assistant_message(response.content));
  if tool_calls.is_empty() { return Ok(proposal); }
  calls += tool_calls.len(); if calls > MAX_TOOL_CALLS_PER_REQUEST { return Err(AiProviderError::InvalidResponse(format!("tool-call limit ({MAX_TOOL_CALLS_PER_REQUEST}) exceeded"))); }
  let mut results = Vec::new();
 for call in tool_calls { let (result, proposed, repair) = execute_tool(state, &call, proposal.as_ref()); if repair { repairs += 1; } if repairs > MAX_PROPOSAL_REPAIR_ATTEMPTS { return Err(AiProviderError::InvalidResponse("proposal repair limit exceeded".into())); } if let Some(next) = proposed { if next.refinement_round > MAX_VISUAL_REFINEMENT_ROUNDS { return Err(AiProviderError::InvalidResponse("visual refinement limit exceeded".into())); } if let Ok(preview) = super::render_proposal_preview(state, &next) { results.push(AiContent::Image(preview.image)); } proposal=Some(next); } results.push(AiContent::ToolResult(AiToolResult { call_id:call.call_id, result })); }
  conversation.push(AiMessage { role:AiRole::Tool, content:results });
 }
 Err(AiProviderError::InvalidResponse(format!("tool-round limit ({MAX_TOOL_ROUNDS}) exceeded")))
}
fn execute_tool(state: &AiDesignState, call: &AiToolCall, proposal: Option<&AiProposal>) -> (Value, Option<AiProposal>, bool) {
 let parse = |error: String| (json!({"success":false,"error":error}), None, false);
 match call.name.as_str() {
  "get_design_context" => match serde_json::from_value::<DesignContextRequest>(call.arguments.clone()).map_err(|e|e.to_string()).and_then(|r|get_design_context(state,r).map_err(|e|format!("{e:?}"))) { Ok(v)=>(json!({"success":true,"context":v}),None,false), Err(e)=>parse(e) },
  "get_objects" => match serde_json::from_value::<GetObjectsRequest>(call.arguments.clone()).map_err(|e|e.to_string()).and_then(|r|get_objects(state,r).map_err(|e|format!("{e:?}"))) { Ok(v)=>(json!({"success":true,"objects":v}),None,false), Err(e)=>parse(e) },
  "get_object_schema" => { let name=call.arguments.get("object_type").and_then(Value::as_str).unwrap_or(""); let ty=ObjectType::values().into_iter().find(|v|format!("{v:?}")==name); match ty.ok_or_else(||format!("unknown object type: {name}")).and_then(|v|get_object_schema(state,v).map_err(|e|format!("{e:?}"))) { Ok(v)=>(json!({"success":true,"schema":v}),None,false), Err(e)=>parse(e) } },
  "analyze_design" => match analyze_design(state, proposal, if proposal.is_some() { DesignAnalysisScope::Proposal } else { DesignAnalysisScope::ActiveMask }) { Ok(v) => (json!({"success":true,"analysis":v}),None,false), Err(e) => parse(format!("{e:?}")) },
  "propose_transaction" => { match serde_json::from_value(call.arguments.clone()).map_err(|e|e.to_string()).and_then(|tx| match proposal { Some(base) => super::propose_refinement(state, base, tx), None => propose_transaction(state,tx) }.map_err(|e|serde_json::to_string(&e).unwrap_or_else(|_|format!("{e:?}")))) { Ok(p)=> { let analysis=analyze_design(state,Some(&p),DesignAnalysisScope::Proposal).ok(); let summary=json!({"success":true,"proposal_id":p.proposal_id,"base_revision":p.base_revision,"description":p.transaction.description,"operation_count":p.summary.operation_count,"affected_objects":p.summary.affected_objects,"refinement_round":p.refinement_round,"design_diagnostics":analysis.map(|v|v.diagnostics)}); (summary,Some(p),false) }, Err(e)=>(json!({"success":false,"diagnostics":e}),None,true) } },
  _ => parse(format!("unsupported tool: {}",call.name)),
 }
}
