//! Request-local orchestration state, independent of provider model identifiers.
use super::{AiDesignState, AiToolCall, ObjectSummary};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiModelRole {
    #[default]
    Fast,
    Strong,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiModelMode {
    #[default]
    Automatic,
    Fast,
    Strong,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiTaskPhase {
    #[default]
    Inspect,
    Propose,
    Repair,
    Review,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiEscalationReason {
    RepeatedValidationFailure,
    InspectionLoop,
    ModelRequestedHelp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AiFailureClass {
    ModelDecision,
    DomainValidation,
    ProviderTransport,
    StaleProject,
    ToolExecution,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct AiTaskState {
    pub goal: String,
    pub explicit_target: Option<ObjectSummary>,
    pub target_root: Option<u16>,
    pub inspected_roots: BTreeSet<u16>,
    pub relevant_objects: BTreeSet<u16>,
    pub style_examples: BTreeSet<u16>,
    pub loaded_schemas: BTreeSet<String>,
    pub phase: AiTaskPhase,
    pub inspection_rounds: usize,
    pub proposal_attempts: usize,
    pub validation_failures: usize,
    pub first_proposal_valid: Option<bool>,
    pub escalation: Option<AiEscalationReason>,
    pub harness_hint: Option<String>,
    #[serde(skip)]
    pub observations: Vec<Value>,
    #[serde(skip)]
    pub latest_candidate: Option<Value>,
    #[serde(skip)]
    pub diagnostics: Vec<Value>,
    #[serde(skip)]
    stagnant_rounds: usize,
    #[serde(skip)]
    warned: bool,
}
/// Recognize only explicit, unambiguous ID forms. Multiple IDs stay unbound.
pub fn explicit_ids(goal: &str) -> BTreeSet<u16> {
    let normalized = goal.to_ascii_lowercase().replace('=', " ");
    let words: Vec<_> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    words
        .windows(2)
        .filter(|w| matches!(w[0], "id" | "datamask" | "mask"))
        .filter_map(|w| w[1].parse().ok())
        .collect()
}
pub fn explicit_target(state: &AiDesignState, goal: &str) -> Option<ObjectSummary> {
    let ids = explicit_ids(goal);
    if ids.len() != 1 {
        return None;
    }
    let id = ag_iso_stack::object_pool::ObjectId::new(*ids.first()?).ok()?;
    super::snapshot::summary(&state.pool, &state.object_info, id).map(|mut s| {
        s.name = s.name.chars().take(128).collect();
        s
    })
}
impl AiTaskState {
    pub fn new(state: &AiDesignState, goal: String) -> Self {
        let explicit_target = explicit_target(state, &goal);
        Self {
            target_root: explicit_target.as_ref().map(|s| s.id),
            explicit_target,
            goal: goal.chars().take(4096).collect(),
            ..Default::default()
        }
    }
    pub fn progress(&self) -> (usize, usize, usize, usize) {
        (
            self.inspected_roots.len(),
            self.relevant_objects.len(),
            self.loaded_schemas.len(),
            self.proposal_attempts,
        )
    }
    pub fn observe(&mut self, call: &AiToolCall, result: &Value) {
        if call.name == "propose_transaction" {
            self.proposal_attempts += 1;
            self.first_proposal_valid
                .get_or_insert(result["success"] == true);
            self.latest_candidate = Some(call.arguments.clone());
            if result["success"] != true {
                self.validation_failures += 1;
                self.phase = AiTaskPhase::Repair;
                self.diagnostics.push(if result.to_string().len() <= 8192 {result.clone()} else {json!({"diagnostic_excerpt":result.to_string().chars().take(2048).collect::<String>(),"truncated":true})});
                if self.diagnostics.len() > 2 {
                    self.diagnostics.remove(0);
                }
            } else {
                self.phase = AiTaskPhase::Review;
            }
            return;
        }
        if result["success"] != true {
            return;
        }
        if let Some(root) = call.arguments["root_id"].as_u64() {
            self.inspected_roots.insert(root as u16);
        }
        if call.name == "get_object_schema" {
            if let Ok(r) = serde_json::from_value::<super::tools::GetObjectSchemaRequest>(
                call.arguments.clone(),
            ) {
                if let Ok(types) = r.types() {
                    self.loaded_schemas.extend(types);
                }
            }
        }
        fn collect(v: &Value, ids: &mut BTreeSet<u16>) {
            match v {
                Value::Object(m) => {
                    if let Some(id) = m
                        .get("id")
                        .and_then(Value::as_u64)
                        .and_then(|id| u16::try_from(id).ok())
                    {
                        if ids.len() < 80 {
                            ids.insert(id);
                        }
                    }
                    for v in m.values() {
                        collect(v, ids);
                    }
                }
                Value::Array(a) => {
                    for v in a {
                        collect(v, ids);
                    }
                }
                _ => {}
            }
        }
        collect(result, &mut self.relevant_objects);
        if call.name == "inspect_design_region" {
            collect(&result["region"]["objects"], &mut self.style_examples);
        }
        let observation = json!({"tool":call.name,"arguments":call.arguments,"result":result});
        if !self.observations.contains(&observation) && observation.to_string().len() <= 16_384 {
            self.observations.push(observation);
            while self
                .observations
                .iter()
                .map(|v| v.to_string().len())
                .sum::<usize>()
                > 32_768
            {
                self.observations.remove(0);
            }
        }
    }
    pub fn inspection_finished(
        &mut self,
        before: (usize, usize, usize, usize),
    ) -> Option<&'static str> {
        if self.phase != AiTaskPhase::Inspect {
            return None;
        }
        self.inspection_rounds += 1;
        self.stagnant_rounds = if before == self.progress() {
            self.stagnant_rounds + 1
        } else {
            0
        };
        if self.inspection_rounds >= 2 && !self.warned {
            self.warned = true;
            return Some("You already have local context. Call finish_inspection and proceed to proposal unless one specific missing fact blocks you.");
        }
        None
    }
    pub fn stuck(&self) -> bool {
        self.phase == AiTaskPhase::Inspect
            && self.warned
            && (self.inspection_rounds > 4 || self.stagnant_rounds >= 3)
    }
    pub fn repeated_validation_failure(&self) -> bool {
        self.diagnostics.len() == 2
            && self.diagnostics[0].get("diagnostics") == self.diagnostics[1].get("diagnostics")
    }
    pub fn handoff(&self) -> Value {
        // Complete JSON observations only. Oversized candidates remain available locally;
        // the handoff explicitly reports omission rather than sending broken JSON.
        let candidate = self
            .latest_candidate
            .as_ref()
            .filter(|v| v.to_string().len() <= 16_384);
        json!({"task":self,"observations":self.observations,"latest_candidate":candidate,
            "candidate_omitted":self.latest_candidate.is_some() && candidate.is_none(),"diagnostics":self.diagnostics,
            "instruction":"Continue from these observations. Do not restart exploration unless a specific missing fact prevents repair."})
    }
}
