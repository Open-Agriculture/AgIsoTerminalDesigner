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
