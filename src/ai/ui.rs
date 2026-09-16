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
#[derive(Debug, Clone)] pub struct AiAssistantState { pub request_state: AiRequestState, pub conversation: Conversation, pub active_proposal: Option<AiProposal>, pub last_error: Option<String>, pub settings: AiSettings }
impl Default for AiAssistantState { fn default() -> Self { Self { request_state: AiRequestState::Idle, conversation: Conversation::default(), active_proposal: None, last_error: None, settings: AiSettings::default() } } }
#[derive(Debug, Clone)] pub enum AiWorkerEvent { Finished { request_id: String, conversation: Conversation, proposal: Option<AiProposal> }, Failed { request_id: String, error: String, conversation: Conversation } }

pub fn run_request(provider: &dyn AiProvider, design: AiDesignState, mut conversation: Conversation, settings: AiSettings, request_id: String, sender: Sender<AiWorkerEvent>) {
 log::info!("AI request started: model={}", settings.model);
 conversation.compact();
 // Separate this worker's diagnostics from a cancelled/replaced conversation.
 let debug = conversation.debug.lock().unwrap().clone();
 conversation.debug = std::sync::Arc::new(std::sync::Mutex::new(debug));
 let result = run_loop(provider, &design, &mut conversation, settings);
 conversation.compact();
 match result { Ok(proposal) => { log::info!("AI request completed: proposal={}", proposal.is_some()); let _=sender.send(AiWorkerEvent::Finished { request_id, conversation, proposal }); }, Err(error) => { log::warn!("AI request failed; details available in Assistant"); let _=sender.send(AiWorkerEvent::Failed { request_id, error:super::debug_trace::safe_text(error.to_string()), conversation }); } }
}
fn run_loop(provider: &dyn AiProvider, state: &AiDesignState, conversation: &mut Conversation, settings: AiSettings) -> Result<Option<AiProposal>, AiProviderError> {
 let runtime = tokio::runtime::Runtime::new().map_err(|e| AiProviderError::Http(e.to_string()))?;
 conversation.debug.lock().unwrap().begin_request();
 let mut cache = ReadToolCache::default();
 let summary = serde_json::to_string(&super::snapshot::project_summary(state)).unwrap();
 let mut proposal = None; let mut calls = 0; let mut repairs: usize = 0;
 for round in 0..MAX_TOOL_ROUNDS {
  let instructions = format!("{SYSTEM_INSTRUCTIONS}\nProject: {summary}\nYou have {} tool rounds remaining for this request. Batch independent inspections, avoid repeating successful calls, and finish with a concise summary as soon as the request is satisfied. A successful proposal is staged for user review; do not keep refining unless necessary.", MAX_TOOL_ROUNDS - round);
  let usage_count = {
   let mut debug = conversation.debug.lock().unwrap();
   debug.provider_rounds = round + 1;
   debug.user_request.request_count
  };
  let response = runtime.block_on(provider.run_turn(AiTurnRequest { instructions, conversation: conversation.clone(), tools: super::tools::tool_definitions(), settings: settings.clone() }));
  {
   let mut debug = conversation.debug.lock().unwrap();
   if debug.user_request.request_count == usage_count {
    debug.record_usage(response.as_ref().map(|r| r.usage.clone()).unwrap_or_default());
   }
  }
  let response = response?;
  let tool_calls: Vec<_> = response.content.iter().filter_map(|c| if let AiContent::ToolCall(call)=c { Some(call.clone()) } else { None }).collect();
  conversation.push(assistant_message(response.content));
  if tool_calls.is_empty() { return Ok(proposal); }
  calls += tool_calls.len(); if calls > MAX_TOOL_CALLS_PER_REQUEST { return Err(AiProviderError::InvalidResponse(format!("tool-call limit ({MAX_TOOL_CALLS_PER_REQUEST}) exceeded"))); }
  let mut results = Vec::new();
  let mut stop_reason = None;
  for call in tool_calls {
   if stop_reason.is_some() {
    results.push(AiContent::ToolResult(AiToolResult { call_id: call.call_id, result: json!({"success":false,"error":"Assistant stopped at its safety limit; this call was not executed."}) }));
    continue;
   }
   {
    let mut debug = conversation.debug.lock().unwrap();
    debug.tool_calls += 1;
    debug.trace(format!("← Tool call {}", call.name), &call.arguments);
   }
   let (mut result, proposed, repair) = cache.execute(state, &call, proposal.as_ref());
   if repair {
    repairs += 1;
    result["repair_attempts_remaining"] = json!(MAX_PROPOSAL_REPAIR_ATTEMPTS.saturating_sub(repairs.saturating_sub(1)));
    if repairs > MAX_PROPOSAL_REPAIR_ATTEMPTS {
     let diagnostic = result["diagnostics"].as_str().unwrap_or("Unknown validation failure");
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
   conversation.debug.lock().unwrap().trace(format!("→ Tool result {}", call.name), &result);
   results.push(AiContent::ToolResult(AiToolResult { call_id:call.call_id, result }));
  }
  conversation.push(AiMessage { role:AiRole::Tool, content:results });
  if let Some(reason) = stop_reason {
   let outcome = if proposal.is_some() { "The previous valid proposal is still staged for review. No changes have been applied." } else { "No changes have been proposed or applied." };
   conversation.push(AiMessage::text(AiRole::Assistant, format!("{reason}\n{outcome}")));
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
struct ReadToolCache(std::collections::HashMap<(u64, String, String), Value>);
impl ReadToolCache {
 fn key(state: &AiDesignState, call: &AiToolCall) -> Option<(u64, String, String)> {
  let args = match call.name.as_str() {
   "get_design_context" => serde_json::to_value(serde_json::from_value::<DesignContextRequest>(call.arguments.clone()).ok()?).ok()?,
   "get_objects" => serde_json::to_value(serde_json::from_value::<GetObjectsRequest>(call.arguments.clone()).ok()?).ok()?,
   "get_object_schema" => serde_json::to_value(serde_json::from_value::<super::tools::GetObjectSchemaRequest>(call.arguments.clone()).ok()?).ok()?,
   _ => return None,
  };
  Some((state.revision, call.name.clone(), args.to_string()))
 }
 fn execute(&mut self, state: &AiDesignState, call: &AiToolCall, proposal: Option<&AiProposal>) -> (Value, Option<AiProposal>, bool) {
  let key = Self::key(state, call);
  if let Some(value) = key.as_ref().and_then(|key| self.0.get(key)) { return (value.clone(), None, false); }
  let result = execute_tool(state, call, proposal);
  if let Some(key) = key { self.0.insert(key, result.0.clone()); }
  result
 }
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

#[cfg(test)]
mod tests {
 use super::*;
 use std::sync::atomic::{AtomicUsize, Ordering};
 struct RepeatingProvider(AtomicUsize);
 impl AiProvider for RepeatingProvider {
  fn run_turn(&self, request: AiTurnRequest) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>> {
   let round = self.0.fetch_add(1, Ordering::SeqCst);
   assert!(request.instructions.contains(&format!("{} tool rounds remaining", MAX_TOOL_ROUNDS - round)));
   Box::pin(async move { Ok(super::super::AiProviderResponse { content: vec![AiContent::ToolCall(AiToolCall {
    call_id: format!("call_{round}"), name: "get_object_schema".into(), arguments: json!({"object_type":"Button"}),
   })], usage: Default::default() }) })
  }
 }
 #[test]
 fn round_limit_preserves_conversation_and_returns_a_clear_stop_message() {
  let provider = RepeatingProvider(AtomicUsize::new(0));
  let state = AiDesignState {
   revision: 0, pool: Default::default(), object_info: Default::default(),
   vt_version: ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
   mask_size: 480, selected: None,
  };
  let mut conversation = Conversation::default();
  let proposal = run_loop(&provider, &state, &mut conversation, AiSettings::default()).unwrap();
  assert!(proposal.is_none());
  assert_eq!(provider.0.load(Ordering::SeqCst), MAX_TOOL_ROUNDS);
  assert_eq!(conversation.messages.len(), MAX_TOOL_ROUNDS * 2 + 1);
  assert!(matches!(&conversation.messages.last().unwrap().content[0], AiContent::Text(text) if text.contains("No changes have been proposed")));
 }
 struct InvalidProposalProvider { calls: AtomicUsize, valid_first: bool }
 impl AiProvider for InvalidProposalProvider {
  fn run_turn(&self, _: AiTurnRequest) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>> {
   let round = self.calls.fetch_add(1, Ordering::SeqCst);
   let arguments = if self.valid_first && round == 0 {
    json!({"schema_version":2,"operations":[{"type":"RenameObject","data":{"object":{"id":10},"name":"Renamed"}}]})
   } else {
    json!({"schema_version":2,"operations":[{"type":"SetProperty","data":{}}]})
   };
   Box::pin(async move { Ok(super::super::AiProviderResponse { content: vec![AiContent::ToolCall(AiToolCall {
    call_id: format!("proposal_{round}"), name: "propose_transaction".into(), arguments,
   })], usage: Default::default() }) })
  }
 }
 #[test]
 fn repair_limit_exposes_validation_and_preserves_valid_work() {
  for valid_first in [false, true] {
   let provider = InvalidProposalProvider { calls: AtomicUsize::new(0), valid_first };
   let mut state = AiDesignState {
    revision: 0, pool: Default::default(), object_info: Default::default(),
    vt_version: ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
    mask_size: 480, selected: None,
   };
   let mut button = crate::default_object(ObjectType::Button);
   button.mut_id().set_value(10).unwrap();
   state.pool.add(button);
   let mut conversation = Conversation::default();
   let proposal = run_loop(&provider, &state, &mut conversation, AiSettings::default()).unwrap();
   assert_eq!(proposal.is_some(), valid_first);
   assert_eq!(provider.calls.load(Ordering::SeqCst), 3 + usize::from(valid_first));
   assert!(matches!(&conversation.messages.last().unwrap().content[0], AiContent::Text(text) if text.contains("missing field `object`") && text.contains("No changes")));
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
  fn run_turn(&self, request: AiTurnRequest) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>> {
   let round = self.0.fetch_add(1, Ordering::SeqCst);
   assert!(request.instructions.contains("Project:"));
   assert!(!request.instructions.contains("\"properties\":"));
   let (name, arguments) = match round {
    0 => ("get_design_context", json!({})),
    1 => {
     assert!(request.conversation.messages.iter().flat_map(|m| &m.content).any(|c| matches!(c, AiContent::ToolResult(r) if r.result["context"]["objects"][0]["id"] == 10)));
     ("get_objects", json!({"ids":[10]}))
    }
    2 => {
     assert!(request.conversation.messages.iter().flat_map(|m| &m.content).any(|c| matches!(c, AiContent::ToolResult(r) if r.result["objects"][0]["properties"].is_object())));
     ("propose_transaction", json!({"schema_version":2,"operations":[{"type":"RenameObject","data":{"object":{"id":10},"name":"Renamed"}}]}))
    }
    _ => ("", json!({})),
   };
   let content = if round < 3 { vec![AiContent::ToolCall(AiToolCall { call_id:format!("call_{round}"), name:name.into(), arguments })] } else { vec![AiContent::Text("Staged for review".into())] };
   let total_tokens = [1000,1400,800,50][round];
   Box::pin(async move { Ok(super::super::AiProviderResponse { content, usage: super::super::AiTokenUsage { total_tokens, ..Default::default() } }) })
  }
 }
 fn state() -> AiDesignState {
  let mut state = super::super::efficiency_tests::pool(0, 0);
  // A standalone button avoids unrelated reference validation in a synthetic pool.
  state.pool = Default::default();
  let mut button = crate::default_object(ObjectType::Button);
  button.mut_id().set_value(10).unwrap(); state.pool.add(button);
  state
 }
 #[test]
 fn progressive_retrieval_stages_change_and_aggregates_calls() {
  let provider = ProgressiveProvider(AtomicUsize::new(0));
  let mut conversation = Conversation::default();
  conversation.push(AiMessage::text(AiRole::User, "Rename selected object"));
  let proposal = run_loop(&provider, &state(), &mut conversation, AiSettings::default()).unwrap().unwrap();
  assert_eq!(proposal.summary.operation_count, 1);
  // The complete tool chain remains present until the request has ended.
  assert_eq!(conversation.messages.iter().flat_map(|m| &m.content).filter(|c| matches!(c, AiContent::ToolResult(_))).count(), 3);
  assert_eq!(conversation.debug.lock().unwrap().user_request.tokens.total_tokens, 3250);
  assert_eq!(conversation.debug.lock().unwrap().conversation.request_count, 4);
  conversation.compact();
  assert_eq!(proposal.summary.operation_count, 1);
  assert!(conversation.messages.iter().flat_map(|m| &m.content).all(|c| matches!(c, AiContent::Text(_))));
 }
 #[test]
 fn typed_cache_uses_revision_and_is_request_local() {
  let mut cache = ReadToolCache::default();
  let mut state = state();
  let call = AiToolCall { call_id:"one".into(), name:"get_objects".into(), arguments:json!({"ids":[10]}) };
  let first = cache.execute(&state, &call, None).0;
  assert_eq!(cache.execute(&state, &call, None).0, first);
  assert_eq!(cache.0.len(), 1);
  let explicit_defaults = AiToolCall { arguments:serde_json::to_value(GetObjectsRequest { ids:vec![10], ..Default::default() }).unwrap(), ..call.clone() };
  assert_eq!(ReadToolCache::key(&state, &call), ReadToolCache::key(&state, &explicit_defaults));
  // Removing data without changing revision demonstrates the second call is served
  // from cache, not re-executed. Real edits always increment revision.
  state.pool = Default::default();
  assert_eq!(cache.execute(&state, &call, None).0, first);
  state.revision += 1;
  assert_eq!(cache.execute(&state, &call, None).0["success"], false);
  assert_eq!(ReadToolCache::default().execute(&state, &call, None).0["success"], false);
 }
 struct FailingProvider(AtomicUsize);
 impl AiProvider for FailingProvider {
  fn run_turn(&self, _: AiTurnRequest) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>> {
   let round = self.0.fetch_add(1, Ordering::SeqCst);
   Box::pin(async move {
    if round == 1 { return Err(AiProviderError::Timeout); }
    Ok(super::super::AiProviderResponse {
     content: if round == 0 { vec![AiContent::ToolCall(AiToolCall { call_id:"c".into(), name:"get_objects".into(), arguments:json!({"ids":[10]}) })] } else { vec![AiContent::Text("Done".into())] },
     usage: super::super::AiTokenUsage { total_tokens:100, ..Default::default() },
    })
   })
  }
 }
 #[test]
 fn failed_worker_retains_usage_and_next_request_accumulates() {
  let provider = FailingProvider(AtomicUsize::new(0));
  let (sender, receiver) = std::sync::mpsc::channel();
  run_request(&provider, state(), Conversation::default(), AiSettings::default(), "first".into(), sender.clone());
  let AiWorkerEvent::Failed { conversation, .. } = receiver.recv().unwrap() else { panic!("expected failure") };
  assert_eq!(conversation.debug.lock().unwrap().user_request.tokens.total_tokens, 100);
  assert!(conversation.messages.iter().flat_map(|m| &m.content).all(|c| matches!(c, AiContent::Text(_))));
  run_request(&provider, state(), conversation, AiSettings::default(), "second".into(), sender);
  let AiWorkerEvent::Finished { conversation, .. } = receiver.recv().unwrap() else { panic!("expected success") };
  let debug = conversation.debug.lock().unwrap();
  assert_eq!(debug.user_request.tokens.total_tokens, 100);
  assert_eq!(debug.conversation.tokens.total_tokens, 200);
 }

}
