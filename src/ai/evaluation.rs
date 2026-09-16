//! Offline, invariant-based evaluation. No analytics persistence or pricing model.
use super::task_state::{AiEscalationReason, AiModelMode};
use super::*;
use ag_iso_stack::object_pool::ObjectId;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub struct EvaluationCase {
    pub name: String,
    pub goal: String,
    pub state: AiDesignState,
    pub allowed_existing: BTreeSet<u16>,
    pub allowed_roots: BTreeSet<u16>,
    pub invariants: Vec<Invariant>,
}
#[derive(Debug, Clone)]
pub enum Invariant {
    Property(u16, String, Value),
    Position(u16, u16, i32, i32),
    CreatedType(String, usize),
    NewDisplay(String, String),
    MacroReference(u16, u16),
}
impl Invariant {
    fn holds(&self, before: &AiDesignState, p: &AiProposal) -> bool {
        match self {
            Self::Property(id, name, value) => {
                ObjectId::new(*id)
                    .ok()
                    .and_then(|id| p.prepared.pool.object_by_id(id))
                    .and_then(|o| crate::object_properties::get_object_property(o, name).ok())
                    .as_ref()
                    == Some(value)
            }
            Self::Position(parent, child, x, y) => ObjectId::new(*parent)
                .ok()
                .and_then(|id| p.prepared.pool.object_by_id(id))
                .is_some_and(|o| {
                    super::snapshot::display_children(o)
                        .iter()
                        .any(|(id, px, py)| id.value() == *child && px == x && py == y)
                }),
            Self::CreatedType(ty, count) => {
                p.prepared
                    .pool
                    .objects()
                    .iter()
                    .filter(|o| {
                        before.pool.object_by_id(o.id()).is_none()
                            && format!("{:?}", o.object_type()) == *ty
                    })
                    .count()
                    == *count
            }
            Self::NewDisplay(ty, value) => p
                .prepared
                .pool
                .objects()
                .iter()
                .filter(|o| {
                    before.pool.object_by_id(o.id()).is_none()
                        && format!("{:?}", o.object_type()) == *ty
                })
                .any(|o| {
                    let font =
                        crate::object_properties::get_object_property(o, "font_attributes").ok();
                    let text = crate::object_properties::get_object_property(o, "value").ok();
                    font == Some(json!(500))
                        && (value.is_empty() || text == Some(json!(value)))
                        && super::snapshot::context_layout(
                            &p.prepared.pool,
                            &p.prepared.object_info,
                            super::snapshot::active_mask(&before.pool),
                            6,
                        )
                        .any(|i| i.object.id == o.id().value())
                }),
            Self::MacroReference(id, macro_id) => ObjectId::new(*id)
                .ok()
                .and_then(|id| p.prepared.pool.object_by_id(id))
                .and_then(crate::operations::macro_refs)
                .is_some_and(|refs| refs.iter().any(|r| u16::from(r.macro_id) == *macro_id)),
        }
    }
}
#[derive(Debug, Default, Serialize)]
pub struct ProposalQuality {
    pub operations: usize,
    pub modified_existing: usize,
    pub created: usize,
    pub created_styles: usize,
    pub unrelated_objects: usize,
    pub unrelated_masks: usize,
}
#[derive(Debug, Serialize)]
pub struct AiEvaluationResult {
    pub case: String,
    pub mode: AiModelMode,
    pub measurement: String,
    pub task_success: bool,
    pub first_proposal_valid: bool,
    pub final_proposal_valid: bool,
    pub reviewer_pass: Option<bool>,
    pub error: Option<String>,
    pub provider_rounds: usize,
    pub tool_calls: usize,
    pub latency_ms: u64,
    pub per_model: std::collections::BTreeMap<String, AiUsageStats>,
    pub usage: AiTokenUsage,
    pub token_measurements_available: bool,
    pub largest_request_bytes: usize,
    pub largest_tool_schema_bytes: usize,
    pub inspection_tool_schema_bytes: usize,
    pub uncached_input_tokens: u64,
    pub wrong_target_reads: usize,
    pub repeated_schema_reads: usize,
    pub validation_failures: usize,
    pub escalation_reason: Option<AiEscalationReason>,
    pub quality: ProposalQuality,
    pub diagnostics: Vec<DesignDiagnostic>,
    pub largest_tool_results: Vec<(String, usize)>,
}
pub fn evaluate(
    provider: &dyn AiProvider,
    case: &EvaluationCase,
    mode: AiModelMode,
    measurement: &str,
) -> AiEvaluationResult {
    let mut conversation = Conversation::default();
    conversation.push(AiMessage::text(AiRole::User, &case.goal));
    let outcome = super::ui::run_loop(
        provider,
        &case.state,
        &mut conversation,
        AiSettings {
            mode,
            ..Default::default()
        },
    );
    let error = outcome.as_ref().err().map(ToString::to_string);
    let proposal = outcome.ok().flatten();
    let mut quality = ProposalQuality::default();
    let mut task_success = false;
    let mut diagnostics = vec![];
    let mut reviewer_pass = None;
    if let Some(p) = &proposal {
        quality.operations = p.summary.operation_count;
        for old in case.state.pool.objects() {
            let changed = p
                .prepared
                .pool
                .object_by_id(old.id())
                .map(|o| serde_json::to_value(o).unwrap())
                != Some(serde_json::to_value(old).unwrap())
                || p.prepared.object_info.get(&old.id()).map(|v| &v.name)
                    != case.state.object_info.get(&old.id()).map(|v| &v.name);
            if changed {
                quality.modified_existing += 1;
                if !case.allowed_existing.contains(&old.id().value()) {
                    quality.unrelated_objects += 1;
                }
                if format!("{:?}", old.object_type()).ends_with("Mask")
                    && !case.allowed_roots.contains(&old.id().value())
                {
                    quality.unrelated_masks += 1;
                }
            }
        }
        for new in p
            .prepared
            .pool
            .objects()
            .iter()
            .filter(|o| case.state.pool.object_by_id(o.id()).is_none())
        {
            quality.created += 1;
            if format!("{:?}", new.object_type()).ends_with("Attributes") {
                quality.created_styles += 1;
            }
        }
        if let Ok(analysis) = analyze_design(&case.state, Some(p), DesignAnalysisScope::Proposal) {
            diagnostics = analysis.diagnostics;
            reviewer_pass = Some(
                !diagnostics
                    .iter()
                    .any(|d| d.severity == DesignDiagnosticSeverity::Error),
            );
        }
        task_success = case.invariants.iter().all(|i| i.holds(&case.state, p))
            && quality.unrelated_objects == 0
            && quality.unrelated_masks == 0;
    }
    let d = conversation.debug.lock().unwrap();
    let wrong_target_reads = d
        .entries
        .iter()
        .filter(|e| {
            e.label == "← Tool call get_design_context"
                || e.label == "← Tool call inspect_design_region"
        })
        .filter(|e| {
            serde_json::from_str::<Value>(&e.payload)
                .ok()
                .and_then(|v| v["root_id"].as_u64())
                .is_some_and(|id| {
                    case.state
                        .pool
                        .object_by_id(ObjectId::new(id as u16).unwrap())
                        .is_some_and(|o| format!("{:?}", o.object_type()).ends_with("Mask"))
                        && !case.allowed_roots.contains(&(id as u16))
                })
        })
        .count();
    let failures = d.task.as_ref().map_or(0, |t| t.validation_failures);
    AiEvaluationResult {
        case: case.name.clone(),
        mode,
        measurement: measurement.into(),
        task_success,
        first_proposal_valid: d
            .task
            .as_ref()
            .and_then(|t| t.first_proposal_valid)
            .unwrap_or(false),
        final_proposal_valid: proposal.is_some(),
        reviewer_pass,
        error,
        provider_rounds: d.provider_rounds,
        tool_calls: d.tool_calls,
        latency_ms: d.latency_ms,
        per_model: d.model_usage.clone(),
        usage: d.user_request.tokens.clone(),
        token_measurements_available: measurement == "live" && !d.rounds.is_empty(),
        largest_request_bytes: d.rounds.iter().map(|r| r.request_bytes).max().unwrap_or(0),
        largest_tool_schema_bytes: d
            .rounds
            .iter()
            .map(|r| r.tool_definitions_bytes)
            .max()
            .unwrap_or(0),
        inspection_tool_schema_bytes: d.rounds.first().map_or(0, |r| r.tool_definitions_bytes),
        uncached_input_tokens: d.user_request.tokens.uncached_input_tokens(),
        wrong_target_reads,
        repeated_schema_reads: d.tools.get("get_object_schema").map_or(0, |s| s.cache_hits),
        validation_failures: failures,
        escalation_reason: d.task.as_ref().and_then(|t| t.escalation),
        quality,
        diagnostics,
        largest_tool_results: d.largest_tool_results.clone(),
    }
}
/// Pre-rollout gate: every paired case must retain the strong-only success/validity/
/// review outcome, without increasing unrelated mutations. Mock runs never qualify.
pub fn quality_gate(results: &[AiEvaluationResult], expected_cases: usize) -> bool {
    if expected_cases < 14 {
        return false;
    }
    let names: BTreeSet<_> = results.iter().map(|r| r.case.as_str()).collect();
    names.len() == expected_cases
        && names.into_iter().all(|name| {
            let find = |mode| {
                results
                    .iter()
                    .find(|r| r.case == name && r.mode == mode && r.measurement == "live")
            };
            match (
                find(AiModelMode::Automatic),
                find(AiModelMode::Strong),
                find(AiModelMode::Fast),
            ) {
                (Some(a), Some(s), Some(_)) => {
                    a.task_success
                        && a.final_proposal_valid
                        && a.reviewer_pass == Some(true)
                        && (!s.first_proposal_valid || a.first_proposal_valid)
                        && a.quality.unrelated_objects <= s.quality.unrelated_objects
                        && a.wrong_target_reads == 0
                }
                _ => false,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{
        object::Object,
        object_attributes::{ObjectRef, Point},
        ObjectType,
    };
    use std::sync::Mutex;
    fn fixture() -> AiDesignState {
        let mut state = crate::ai::efficiency_tests::pool(0, 0);
        for (id, ty) in [
            (3, ObjectType::DataMask),
            (10, ObjectType::OutputString),
            (11, ObjectType::OutputString),
            (12, ObjectType::OutputNumber),
            (13, ObjectType::NumberVariable),
            (20, ObjectType::Button),
            (30, ObjectType::Macro),
            (500, ObjectType::FontAttributes),
        ] {
            let mut o = crate::default_object(ty);
            o.mut_id().set_value(id).unwrap();
            match &mut o {
                Object::OutputString(v) => {
                    v.font_attributes = ObjectId::new(500).unwrap();
                    v.width = 80;
                    v.height = 20;
                    v.value = "Meters".into();
                }
                Object::OutputNumber(v) => {
                    v.font_attributes = ObjectId::new(500).unwrap();
                    v.width = 80;
                    v.height = 20;
                }
                _ => {}
            }
            state.pool.add(o);
        }
        for (root, ids) in [(2, vec![10, 11, 12]), (3, vec![20])] {
            if let Some(Object::DataMask(m)) =
                state.pool.object_mut_by_id(ObjectId::new(root).unwrap())
            {
                for (index, id) in ids.into_iter().enumerate() {
                    m.object_refs.push(ObjectRef {
                        id: ObjectId::new(id).unwrap(),
                        offset: Point {
                            x: 20,
                            y: 100 + index as i16 * 60,
                        },
                    });
                }
            }
        }
        state.selected = ObjectId::new(3).ok();
        state
    }
    fn op(ty: &str, data: Value) -> Value {
        json!({"type":ty,"data":data})
    }
    fn property(id: u16, name: &str, value: Value) -> Value {
        op(
            "SetProperty",
            json!({"object":{"id":id},"property":name,"value":value}),
        )
    }
    fn position(id: u16, x: i32, y: i32) -> Value {
        op(
            "SetChildPosition",
            json!({"parent_id":2,"child_id":id,"x":x,"y":y}),
        )
    }
    fn create(id: u16, ty: &str, text: &str, x: i32) -> Vec<Value> {
        let mut ops = vec![
            op(
                "CreateObject",
                json!({"object_id":id,"object_type":ty,"name":format!("New {ty}")}),
            ),
            op(
                "SetObjectReference",
                json!({"object":{"id":id},"property":"font_attributes","reference":{"id":500}}),
            ),
            property(id, "width", json!(80)),
            property(id, "height", json!(20)),
            op(
                "AddChild",
                json!({"parent":{"id":2},"child":{"id":id},"x":x,"y":20}),
            ),
        ];
        if ty == "OutputString" {
            ops.push(property(id, "value", json!(text)));
        }
        ops
    }
    fn cases() -> Vec<(EvaluationCase, Value, bool)> {
        let defs:Vec<(&str,&str,Vec<Value>,Vec<Invariant>,Vec<u16>,bool)>=vec![
            ("move","Move id 10 to (40,100) in DataMask 2",vec![position(10,40,100)],vec![Invariant::Position(2,10,40,100)],vec![2],false),
            ("resize","Set id 10 width to 100",vec![property(10,"width",json!(100))],vec![Invariant::Property(10,"width".into(),json!(100))],vec![10],false),
            ("property","Set id 10 text to Distance",vec![property(10,"value",json!("Distance"))],vec![Invariant::Property(10,"value".into(),json!("Distance"))],vec![10],false),
            ("reference","Set id 12 variable reference to id 13",vec![op("SetObjectReference",json!({"object":{"id":12},"property":"variable_reference","reference":{"id":13}}))],vec![Invariant::Property(12,"variable_reference".into(),json!(13))],vec![12],false),
            ("add_label","Add Meters label to DataMask 2 using font 500",create(600,"OutputString","Meters",200),vec![Invariant::NewDisplay("OutputString".into(),"Meters".into())],vec![2],false),
            ("add_number","Add a number to DataMask 2 using font 500",create(600,"OutputNumber","",200),vec![Invariant::NewDisplay("OutputNumber".into(),"".into())],vec![2],false),
            ("number_and_unit","Add a number at the top middle on DataMask 2 with a Meters label. Follow existing style.",[create(600,"OutputNumber","",180),create(601,"OutputString","Meters",280)].concat(),vec![Invariant::NewDisplay("OutputNumber".into(),"".into()),Invariant::NewDisplay("OutputString".into(),"Meters".into())],vec![2],false),
            ("nearby_style","Add a Meters label to DataMask 2 matching nearby labels",create(600,"OutputString","Meters",200),vec![Invariant::NewDisplay("OutputString".into(),"Meters".into())],vec![2],false),
            ("reuse_font","Add a number to DataMask 2 reusing existing FontAttributes",create(600,"OutputNumber","",200),vec![Invariant::NewDisplay("OutputNumber".into(),"".into()),Invariant::CreatedType("FontAttributes".into(),0)],vec![2],false),
            ("explicit_target","Modify DataMask 2: move its label 10 to (40,100). The selected mask is unrelated.",vec![position(10,40,100)],vec![Invariant::Position(2,10,40,100)],vec![2],false),
            ("local_rearrangement","Arrange labels 10 and 11 in DataMask 2 at (20,100) and (120,100)",vec![position(10,20,100),position(11,120,100)],vec![Invariant::Position(2,10,20,100),Invariant::Position(2,11,120,100)],vec![2],false),
            ("repair","Set id 10 width to 100; repair any invalid proposal",vec![property(10,"width",json!(100))],vec![Invariant::Property(10,"width".into(),json!(100))],vec![10],true),
            ("macro_reference","Connect DataMask 2 OnActivate to existing Macro 30",vec![op("SetMacroReferences",json!({"object":{"id":2},"macro_refs":[{"event_id":"OnActivate","macro_object":{"id":30}}]}))],vec![Invariant::MacroReference(2,30)],vec![2],false),
            ("multi_mask","Set DataMask 2 and DataMask 3 background_colour to 1",vec![property(2,"background_colour",json!(1)),property(3,"background_colour",json!(1))],vec![Invariant::Property(2,"background_colour".into(),json!(1)),Invariant::Property(3,"background_colour".into(),json!(1))],vec![2,3],false),
        ];
        defs.into_iter()
            .map(|(name, goal, ops, invariants, allowed, repair)| {
                (
                    EvaluationCase {
                        name: name.into(),
                        goal: goal.into(),
                        state: fixture(),
                        allowed_existing: allowed.into_iter().collect(),
                        allowed_roots: if name == "multi_mask" {
                            BTreeSet::from([2, 3])
                        } else {
                            BTreeSet::from([2])
                        },
                        invariants,
                    },
                    json!({"schema_version":2,"operations":ops}),
                    repair,
                )
            })
            .collect()
    }
    struct Replay {
        step: Mutex<usize>,
        tx: Value,
        repair: bool,
        root: u16,
    }
    impl AiProvider for Replay {
        fn run_turn(
            &self,
            request: AiTurnRequest,
        ) -> AiProviderFuture<Result<AiProviderResponse, AiProviderError>> {
            let mut step = self.step.lock().unwrap();
            let n = *step;
            *step += 1;
            // Measure real serialized payload bytes, never invent provider tokens.
            let body = super::super::providers::openai::request_body(&request);
            request
                .conversation
                .debug
                .lock()
                .unwrap()
                .record_request(&body);
            let calls = match n {
                0 => vec![(
                    "get_design_context",
                    json!({"root_id":self.root,"include_selection":false}),
                )],
                1 => vec![
                    (
                        "inspect_design_region",
                        json!({"root_id":self.root,"region":"top","types":["OutputNumber","OutputString"]}),
                    ),
                    (
                        "get_object_schema",
                        json!({"object_types":["OutputNumber","OutputString"]}),
                    ),
                    ("finish_inspection", json!({})),
                ],
                2 if self.repair => vec![(
                    "propose_transaction",
                    json!({"schema_version":2,"operations":[{"type":"SetProperty","data":{}}]}),
                )],
                2 => vec![("propose_transaction", self.tx.clone())],
                3 if self.repair => vec![("propose_transaction", self.tx.clone())],
                _ => vec![],
            };
            let content = if calls.is_empty() {
                vec![AiContent::Text("Staged".into())]
            } else {
                calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, args))| {
                        AiContent::ToolCall(AiToolCall {
                            call_id: format!("{n}_{i}"),
                            name: name.into(),
                            arguments: args,
                        })
                    })
                    .collect()
            };
            Box::pin(async move {
                Ok(AiProviderResponse {
                    content,
                    usage: Default::default(),
                })
            })
        }
    }
    #[test]
    fn offline_three_mode_matrix_uses_invariants() {
        let mut results = vec![];
        for (case, tx, repair) in cases() {
            for mode in [
                AiModelMode::Fast,
                AiModelMode::Strong,
                AiModelMode::Automatic,
            ] {
                let provider = Replay {
                    step: Mutex::new(0),
                    tx: tx.clone(),
                    repair,
                    root: 2,
                };
                let result = evaluate(&provider, &case, mode, "mock replay");
                assert!(
                    result.task_success,
                    "{} {:?}: {}",
                    case.name,
                    mode,
                    serde_json::to_string(&result).unwrap()
                );
                assert!(result.provider_rounds <= if repair { 5 } else { 4 });
                assert_eq!(result.quality.unrelated_objects, 0);
                results.push(result);
            }
        }
        assert_eq!(results.len(), 42);
        assert!(!quality_gate(&results, 14));
        println!(
            "PHASE54_OFFLINE_RESULTS={}",
            serde_json::to_string(&results).unwrap()
        );
    }
    #[test]
    fn representative_2737_object_boundary() {
        let (mut case, mut tx, _) = cases()
            .into_iter()
            .find(|(c, _, _)| c.name == "number_and_unit")
            .unwrap();
        let remap:crate::operations::OperationTransaction=serde_json::from_value(json!({"schema_version":2,"operations":[op("ChangeObjectId",json!({"old_id":2,"new_id":1000})),op("ChangeObjectId",json!({"old_id":3,"new_id":1002}))]})).unwrap();
        let remapped = propose_transaction(&case.state, remap).unwrap();
        case.state.pool = remapped.prepared.pool;
        case.state.object_info = remapped.prepared.object_info;
        case.state.selected = ObjectId::new(1002).ok();
        let mut id = 10000;
        while case.state.pool.objects().len() < 2737 {
            let mut o = crate::default_object(ObjectType::NumberVariable);
            o.mut_id().set_value(id).unwrap();
            case.state.pool.add(o);
            id += 1;
        }
        case.goal="Add a number at the top middle on DataMask 1000 with a 'Meters' label, to indicate meters remaining. Follow the existing style.".into();
        case.allowed_existing = BTreeSet::from([1000]);
        case.allowed_roots = BTreeSet::from([1000]);
        for op in tx["operations"].as_array_mut().unwrap() {
            if op["type"] == "AddChild" {
                op["data"]["parent"]["id"] = json!(1000);
            }
        }
        let provider = Replay {
            step: Mutex::new(0),
            tx,
            repair: false,
            root: 1000,
        };
        let result = evaluate(
            &provider,
            &case,
            AiModelMode::Automatic,
            "mock replay, 2737 objects",
        );
        assert!(result.task_success);
        assert_eq!(result.provider_rounds, 4);
        assert_eq!(result.wrong_target_reads, 0);
        assert_eq!(result.per_model["gpt-5.6-luna"].request_count, 4);
        assert!(!result.per_model.contains_key("gpt-5.6-sol"));
        println!(
            "PHASE54_REPRESENTATIVE={}",
            serde_json::to_string(&result).unwrap()
        );
    }
    #[test]
    #[ignore = "Requires OPENAI_API_KEY; runs 42 paid live model tasks"]
    fn live_three_mode_quality_matrix() {
        let mut results = vec![];
        for (case, _, _) in cases() {
            for mode in [
                AiModelMode::Fast,
                AiModelMode::Strong,
                AiModelMode::Automatic,
            ] {
                let provider = providers::openai::OpenAiProvider::from_environment()
                    .expect("OPENAI_API_KEY required");
                results.push(evaluate(&provider, &case, mode, "live"));
            }
        }
        println!(
            "PHASE54_LIVE_RESULTS={}",
            serde_json::to_string(&results).unwrap()
        );
        assert!(
            quality_gate(&results, 14),
            "Quality gate failed; review reported outcomes before rollout"
        );
    }
}
