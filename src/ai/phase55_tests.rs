use super::*;
use ag_iso_stack::object_pool::{ObjectId, NullableObjectId, object::Object};
use std::sync::Mutex;

fn wifi() -> AiDesignState {
    let mut state = AiDesignState {
        revision: 1, pool: Default::default(), object_info: Default::default(),
        vt_version: ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
        mask_size: 480, selected: None,
    };
    let mut ws = crate::default_object(ObjectType::WorkingSet);
    ws.mut_id().set_value(1).unwrap();
    if let Object::WorkingSet(ws) = &mut ws { ws.active_mask = NullableObjectId(Some(ObjectId::new(2).unwrap())); }
    state.pool.add(ws);
    let mut mask = crate::default_object(ObjectType::DataMask);
    mask.mut_id().set_value(2).unwrap();
    let mut info = crate::ObjectInfo::new(&mask);
    info.set_name("WiFi Screen".into());
    state.object_info.insert(mask.id(), info);
    state.pool.add(mask);
    state
}
fn call(name: &str, arguments: Value) -> AiContent {
    AiContent::ToolCall(AiToolCall { call_id: uuid::Uuid::new_v4().to_string(), name: name.into(), arguments })
}
struct Script {
    outputs: Mutex<std::collections::VecDeque<Vec<AiContent>>>,
    requests: Mutex<Vec<AiTurnRequest>>,
}
impl Script {
    fn new(outputs: Vec<Vec<AiContent>>) -> Self { Self { outputs: Mutex::new(outputs.into()), requests: Mutex::new(Vec::new()) } }
}
impl AiProvider for Script {
    fn run_turn(&self, request: AiTurnRequest) -> super::super::AiProviderFuture<Result<super::super::AiProviderResponse, AiProviderError>> {
        self.requests.lock().unwrap().push(request);
        let content = self.outputs.lock().unwrap().pop_front().expect("unexpected provider round");
        Box::pin(async move { Ok(super::super::AiProviderResponse { content, usage: Default::default() }) })
    }
}
fn plan() -> AiContent {
    call("finish_inspection", json!({"summary":"WiFi title, network/password inputs and Connect button", "required_object_types":["OutputString","InputString","Button","FontAttributes"]}))
}
fn proposal() -> AiContent {
    let mut ops = vec![json!({"type":"CreateObject","data":{"object_type":"FontAttributes","name":"WiFi font"}})];
    for (ty, name, y) in [("OutputString","WiFi",20),("InputString","Network",90),("InputString","Password",150),("Button","Connect",220)] {
        ops.push(json!({"type":"CreateObject","data":{"object_type":ty,"name":name}}));
        ops.push(json!({"type":"SetProperty","data":{"object":{"name":name},"property":"width","value":300}}));
        ops.push(json!({"type":"SetProperty","data":{"object":{"name":name},"property":"height","value":40}}));
        if ty != "Button" {
            ops.push(json!({"type":"SetObjectReference","data":{"object":{"name":name},"property":"font_attributes","reference":{"name":"WiFi font"}}}));
            ops.push(json!({"type":"SetProperty","data":{"object":{"name":name},"property":"value","value":name}}));
        }
        ops.push(json!({"type":"AddChild","data":{"parent":{"id":2},"child":{"name":name},"x":30,"y":y}}));
    }
    call("propose_transaction", json!({"description":"Simple WiFi operator screen", "operations":ops}))
}
fn conversation() -> Conversation {
    let mut conversation = Conversation::default();
    conversation.push(AiMessage::text(AiRole::User, "Create a simple WiFi operator screen."));
    conversation
}
#[test]
fn empty_wifi_finishes_in_three_rounds_with_schemas_ready_and_no_mutation() {
    let state = wifi();
    let provider = Script::new(vec![vec![plan()], vec![proposal()], vec![AiContent::Text("Proposal ready.".into())]]);
    let mut conversation = conversation();
    let proposal = run_loop(&provider, &state, &mut conversation, AiSettings::default()).unwrap().unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].conversation.task_state.as_ref().unwrap().screen_is_empty);
    assert_eq!(requests[1].settings.phase, AiTaskPhase::Propose);
    assert_eq!(requests[1].conversation.task_state.as_ref().unwrap().loaded_schemas.len(), 4);
    assert!(requests.iter().all(|r| r.settings.role == AiModelRole::Fast));
    assert_eq!(proposal.transaction.schema_version, crate::operations::CURRENT_OPERATION_SCHEMA_VERSION);
    assert_eq!(state.pool.objects().len(), 2);
    let task = conversation.task_state.unwrap();
    assert_eq!(task.first_proposal_valid, Some(true));
    assert_eq!(task.validation_failures, 0);
    assert!(task.is_complete);
}
#[test]
fn checkpoint_resumes_original_goal_phase_target_and_exact_schemas() {
    for continuation in ["Continue", "Continue, but make the Connect button wider."] {
        let state = wifi();
        let provider = Script::new(vec![vec![plan()]]);
        let mut conversation = conversation();
        run_loop_with_budget(&provider, &state, &mut conversation, AiSettings::default(), 1).unwrap();
        let before = conversation.task_state.clone().unwrap();
        assert!(before.checkpoint);
        assert_eq!(before.phase, AiTaskPhase::Propose);
        conversation.compact();
        conversation.push(AiMessage::text(AiRole::User, continuation));
        let provider = Script::new(vec![vec![proposal()], vec![AiContent::Text("Ready.".into())]]);
        assert!(run_loop(&provider, &state, &mut conversation, AiSettings::default()).unwrap().is_some());
        let requests = provider.requests.lock().unwrap();
        let resumed = requests[0].conversation.task_state.as_ref().unwrap();
        assert_eq!(resumed.goal, before.goal);
        assert_eq!(resumed.target_root, Some(2));
        assert_eq!(resumed.phase, AiTaskPhase::Propose);
        assert_eq!(resumed.schema_definitions, before.schema_definitions);
        assert!(requests[0].conversation.messages.iter().any(|m| m.content.iter().any(|c| matches!(c,AiContent::Text(text) if text.contains("properties") && text.contains("InputString")))));
        assert_eq!(!resumed.continuation_instructions.is_empty(), continuation.contains(','));
    }
}
#[test]
fn new_task_replaces_pending_and_continue_without_pending_is_ordinary() {
    let mut state = wifi();
    let mut mask = crate::default_object(ObjectType::DataMask);
    mask.mut_id().set_value(5).unwrap(); state.pool.add(mask);
    for goal in ["Change DataMask 5 background to white.", "Continue"] {
        let mut conversation = conversation();
        if goal != "Continue" {
            conversation.task_state = Some(AiTaskState::new(&state, "Build WiFi screen".into()));
        }
        conversation.push(AiMessage::text(AiRole::User, goal));
        let provider = Script::new(vec![vec![AiContent::Text("Done".into())]]);
        run_loop(&provider, &state, &mut conversation, AiSettings::default()).unwrap();
        let task = conversation.task_state.unwrap();
        assert_eq!(task.goal, goal);
        assert!(task.loaded_schemas.is_empty());
        if goal != "Continue" { assert_eq!(task.target_root, Some(5)); }
    }
}
#[test]
fn canonical_strict_schemas_and_internal_oversized_batch() {
    let defs = super::super::tools::phase_tool_definitions(AiTaskPhase::Inspect);
    let schema = &defs.iter().find(|t| t["name"] == "get_object_schema").unwrap()["parameters"];
    let canonical = super::super::tools::canonical_object_type_schema();
    assert_eq!(schema["properties"]["object_type"]["anyOf"][0], canonical);
    assert_eq!(schema["properties"]["object_types"]["anyOf"][0]["items"], canonical);
    assert!(!canonical["enum"].as_array().unwrap().contains(&json!("NumericOutput")));
    for tool in defs { assert_eq!(tool["strict"], true); assert_eq!(tool["parameters"]["required"].as_array().unwrap().len(),tool["parameters"]["properties"].as_object().unwrap().len()); }
    let types: Vec<_> = ObjectType::values().iter().take(11).map(|v| format!("{v:?}")).collect();
    let mut cache = ReadToolCache::default();
    let result = cache.execute(&wifi(), &AiToolCall { call_id:"batch".into(),name:"get_object_schema".into(),arguments:json!({"object_types":types}) },None).0;
    assert_eq!(result["success"], true);
    assert_eq!(result["schemas"].as_array().unwrap().len(),11);
    let invalid = cache.execute(&wifi(), &AiToolCall { call_id:"invalid".into(),name:"get_object_schema".into(),arguments:json!({"object_type":"NumericOutput"}) },None).0;
    assert_eq!(invalid,json!({"success":false,"invalid_types":["NumericOutput"],"suggestions":["OutputNumber"]}));
    let tools = super::super::tools::phase_tool_definitions(AiTaskPhase::Propose);
    let proposal = tools.iter().find(|t|t["name"]=="propose_transaction").unwrap();
    assert!(proposal["parameters"]["properties"].get("schema_version").is_none());
}
#[test]
fn revision_change_retains_unaffected_facts_and_revalidates_changed_target() {
    let mut state = wifi();
    let mut task = AiTaskState::new(&state, "Create WiFi screen".into());
    task.refresh_target(&state); task.capture_fingerprints(&state);
    task.phase = AiTaskPhase::Propose;
    task.loaded_schemas.insert("Button".into());
    state.revision += 1;
    task.revalidate(&state);
    assert_eq!(task.phase,AiTaskPhase::Propose);
    state.object_info.get_mut(&ObjectId::new(2).unwrap()).unwrap().set_name("Changed".into());
    state.revision += 1;
    task.revalidate(&state);
    assert_eq!(task.phase,AiTaskPhase::Inspect);
    assert!(task.loaded_schemas.contains("Button"));
    assert!(task.escalation.is_none());
}
