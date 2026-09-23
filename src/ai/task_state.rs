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
    HarnessNormalization,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct AiTaskState {
    pub goal: String,
    pub requested_theme: Value,
    pub resolved_theme: Value,
    pub continuation_instructions: Vec<String>,
    pub expected_revision: u64,
    pub is_complete: bool,
    pub checkpoint: bool,
    pub screen_is_empty: bool,
    pub provider_rounds_used: usize,
    pub tool_calls_used: usize,
    pub normalization_count: usize,
    pub round_reason: String,
    #[serde(skip)]
    pub schema_definitions: std::collections::BTreeMap<String, Value>,
    #[serde(skip)]
    pub object_fingerprints: std::collections::BTreeMap<u16, Value>,
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
            expected_revision: state.revision,
            target_root: explicit_target.as_ref().map(|s| s.id).or_else(|| {
                explicit_ids(&goal)
                    .is_empty()
                    .then(|| super::snapshot::active_mask(&state.pool).map(|id| id.value()))
                    .flatten()
            }),
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
        if matches!(
            call.name.as_str(),
            "propose_transaction" | "propose_component_actions"
        ) {
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
        if call.name == "get_object_schema" || call.name == "finish_inspection" {
            let schemas = result["schemas"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| result.get("schema").cloned().into_iter().collect());
            for schema in schemas {
                if let Some(name) = schema["object_type"]
                    .as_str()
                    .or_else(|| schema["type"].as_str())
                {
                    self.loaded_schemas.insert(name.to_owned());
                    self.schema_definitions.insert(name.to_owned(), schema);
                }
            }
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
        // Exact schema definitions are retained once, not duplicated as observations.
        if matches!(
            call.name.as_str(),
            "get_object_schema" | "finish_inspection"
        ) {
            return;
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
        json!({"task":self,"schemas":self.schema_definitions,"observations":self.observations,"latest_candidate":candidate,
            "candidate_omitted":self.latest_candidate.is_some() && candidate.is_none(),"diagnostics":self.diagnostics,
            "instruction":"Continue from these observations. Do not restart exploration unless a specific missing fact prevents repair."})
    }
}

/// Conservative continuation recognition; arbitrary new instructions replace the task.
pub fn continuation_instruction(message: &str) -> Option<String> {
    let text = message.trim();
    let lower = text.to_ascii_lowercase();
    for prefix in ["continue", "go on", "keep going", "proceed", "resume"] {
        if lower == prefix || lower == format!("{prefix}.") {
            return Some(String::new());
        }
        if lower.starts_with(&format!("{prefix},")) {
            return Some(text[prefix.len() + 1..].trim().to_owned());
        }
    }
    None
}
impl AiTaskState {
    pub fn refresh_target(&mut self, state: &AiDesignState) {
        if let Some(root) = self
            .target_root
            .and_then(|id| ag_iso_stack::object_pool::ObjectId::new(id).ok())
        {
            self.screen_is_empty = state
                .pool
                .object_by_id(root)
                .is_some_and(|o| format!("{:?}", o.object_type()).ends_with("Mask"))
                && super::snapshot::context_layout(&state.pool, &state.object_info, Some(root), 1)
                    .next()
                    .is_none();
            if self.screen_is_empty {
                self.inspected_roots.insert(root.value());
                self.relevant_objects.insert(root.value());
                self.harness_hint = Some("Target is empty. Use defaults and finish_inspection with a minimal design vocabulary; no further local exploration is needed.".into());
            }
        }
    }
    pub fn capture_fingerprints(&mut self, state: &AiDesignState) {
        for id in self
            .relevant_objects
            .iter()
            .copied()
            .chain(self.target_root)
        {
            if let Some(value) = ag_iso_stack::object_pool::ObjectId::new(id)
                .ok()
                .and_then(|id| {
                    super::snapshot::object_snapshot_sections(
                        &state.pool,
                        &state.object_info,
                        id,
                        false,
                    )
                })
            {
                self.object_fingerprints
                    .insert(id, serde_json::to_value(value).unwrap());
            }
        }
    }
    pub fn revalidate(&mut self, state: &AiDesignState) {
        if self.expected_revision == state.revision {
            return;
        }
        if let Ok(theme) = super::visual_intent::resolve_theme(&state.pool, &self.requested_theme) {
            self.resolved_theme = theme;
        }
        let changed: BTreeSet<_> = self
            .object_fingerprints
            .iter()
            .filter_map(|(id, old)| {
                let current = ag_iso_stack::object_pool::ObjectId::new(*id)
                    .ok()
                    .and_then(|id| {
                        super::snapshot::object_snapshot_sections(
                            &state.pool,
                            &state.object_info,
                            id,
                            false,
                        )
                    })
                    .and_then(|v| serde_json::to_value(v).ok());
                (current.as_ref() != Some(old)).then_some(*id)
            })
            .collect();
        if !changed.is_empty() {
            self.inspected_roots.retain(|id| !changed.contains(id));
            self.relevant_objects.retain(|id| !changed.contains(id));
            self.style_examples.retain(|id| !changed.contains(id));
            self.observations.retain(|v| {
                fn mentions(value: &Value, ids: &BTreeSet<u16>) -> bool {
                    match value {
                        Value::Object(map) => map.iter().any(|(key, v)| {
                            ((key == "id" || key == "root_id")
                                && v.as_u64().is_some_and(|id| ids.contains(&(id as u16))))
                                || mentions(v, ids)
                        }),
                        Value::Array(items) => items.iter().any(|v| mentions(v, ids)),
                        _ => false,
                    }
                }
                !mentions(v, &changed)
            });
            self.phase = AiTaskPhase::Inspect;
            self.inspection_rounds = 0;
            self.stagnant_rounds = 0;
            self.warned = false;
            self.harness_hint = Some(format!("Project changed. Reinspect changed objects {changed:?}; reuse other facts and schemas. Revalidate the retained candidate before proposing."));
        }
        self.explicit_target = explicit_target(state, &self.goal);
        self.expected_revision = state.revision;
        self.refresh_target(state);
    }
}
