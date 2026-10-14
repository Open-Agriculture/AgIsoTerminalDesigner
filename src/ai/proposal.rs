use super::snapshot::ObjectSummary;
use super::{AiDesignState, ProjectRevision};
use crate::operations::{
    Operation, OperationTransaction, PreparedTransaction, TransactionDiagnostics,
};
use ag_iso_stack::object_pool::ObjectId;
use uuid::Uuid;
#[derive(Debug, Clone)]
pub struct AiProposal {
    pub proposal_id: String,
    pub canonical_pool: ag_iso_stack::object_pool::ObjectPool,
    pub component_intents: Vec<crate::components::ComponentIntent>,
    pub components: crate::components::ComponentSource,
    pub graphics: crate::graphics::GraphicLibrary,
    pub source_map: crate::components::MaterializedSourceMap,
    pub base_revision: ProjectRevision,
    pub transaction: OperationTransaction,
    pub prepared: PreparedTransaction,
    pub summary: AiProposalSummary,
    /// Number of visual-review corrections included in this staged proposal.
    pub refinement_round: u8,
    pub soft_key_layout: crate::SoftKeyLayout,
}
#[derive(Debug, Clone, Default)]
pub struct AiProposalSummary {
    pub operation_count: usize,
    pub affected_objects: Vec<ObjectSummary>,
    pub created: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub property_changes: usize,
    pub structural_changes: usize,
}
pub fn propose_transaction(
    state: &AiDesignState,
    transaction: OperationTransaction,
) -> Result<AiProposal, TransactionDiagnostics> {
    let editor_project = crate::EditorProject::from_ai_source(state);
    let mut prepared = editor_project.prepare_transaction(&transaction)?;
    let canonical_pool = prepared.pool.clone();
    let materialized = crate::components::materialize_source_with_graphics(
        &canonical_pool,
        &state.components,
        state.vt_version,
        &prepared.object_info,
        &state.graphics,
        &state.object_id_ranges,
    )
    .map_err(crate::editor_project::component_transaction_error)?;
    prepared.pool = materialized.pool;
    let mut summary = AiProposalSummary {
        operation_count: transaction.operations.len(),
        ..Default::default()
    };
    for operation in &transaction.operations {
        match operation {
            Operation::CreateObject { .. } => summary.created += 1,
            Operation::DeleteObject { .. } => summary.deleted += 1,
            Operation::RenameObject { .. } => summary.renamed += 1,
            Operation::SetProperty { .. }
            | Operation::SetObjectReference { .. }
            | Operation::SetObjectPointerAllowedReferences { .. } => summary.property_changes += 1,
            Operation::AddChild { .. }
            | Operation::RemoveChild { .. }
            | Operation::SetChildPosition { .. }
            | Operation::SetChildren { .. }
            | Operation::SetObjectList { .. }
            | Operation::SetMacroReferences { .. } => summary.structural_changes += 1,
            _ => {}
        }
    }
    summary.affected_objects = prepared
        .applied
        .affected_objects
        .iter()
        .filter_map(|id| {
            let object = prepared.pool.object_by_id(ObjectId::new(*id).ok()?)?;
            Some(ObjectSummary {
                id: *id,
                name: prepared
                    .object_info
                    .get(&object.id())
                    .and_then(|i| i.name.clone())
                    .unwrap_or_else(|| format!("Object_{}", id)),
                object_type: format!("{:?}", object.object_type()),
            })
        })
        .collect();
    Ok(AiProposal {
        proposal_id: Uuid::new_v4().to_string(),
        canonical_pool,
        component_intents: vec![],
        components: state.components.clone(),
        graphics: state.graphics.clone(),
        source_map: materialized.source_map,
        base_revision: state.revision,
        transaction,
        prepared,
        summary,
        refinement_round: 0,
        soft_key_layout: state.soft_key_layout,
    })
}

/// Prepare a corrective transaction against an already staged candidate.  The
/// returned proposal deliberately retains the original revision, so Apply still
/// performs the normal stale check against the canonical project.
pub fn propose_refinement(
    state: &AiDesignState,
    base: &AiProposal,
    correction: OperationTransaction,
) -> Result<AiProposal, TransactionDiagnostics> {
    if !base.component_intents.is_empty() {
        return Err(crate::editor_project::component_transaction_error(crate::components::ComponentError::new("COMPONENT_EDIT_POLICY", "Use component overrides to refine component proposals; apply before adding primitive edits")));
    }
    let candidate = AiDesignState {
        canonical_pool: Some(base.canonical_pool.clone()),
        components: base.components.clone(),
        graphics: base.graphics.clone(),
        object_id_ranges: state.object_id_ranges.clone(),
        revision: state.revision,
        pool: base.prepared.pool.clone(),
        object_info: base.prepared.object_info.clone(),
        vt_version: state.vt_version,
        mask_size: state.mask_size,
        soft_key_size: state.soft_key_size,
        soft_key_layout: base.soft_key_layout,
        selected: state.selected,
    };
    // First validate against P1. This is the important validation boundary for
    // a visual correction; it may reference objects made by the initial edit.
    let corrected_candidate = propose_transaction(&candidate, correction.clone())?;
    // Flattening preserves normal one-transaction apply/undo semantics.
    let mut operations = base.transaction.operations.clone();
    let correction_description = correction.description.clone();
    operations.extend(correction.operations);
    let combined = OperationTransaction {
        schema_version: base.transaction.schema_version,
        description: correction_description.or_else(|| base.transaction.description.clone()),
        operations,
    };
    // Reprepare the flattened sequence from P0 so its AppliedTransaction has a
    // single, exact undo/redo record. The earlier candidate preparation above
    // ensures correction references are interpreted against P1.
    let mut refined = propose_transaction(state, combined)?;
    refined.refinement_round = base.refinement_round.saturating_add(1);
    let _ = corrected_candidate; // documents the staged validation boundary.
    Ok(refined)
}

/// Stage source commands through exactly the same service used by the editor.
pub fn propose_component_intents(
    state: &AiDesignState,
    mut intents: Vec<crate::components::ComponentIntent>,
) -> Result<AiProposal, TransactionDiagnostics> {
    if intents.is_empty() || intents.len() > 256 {
        return Err(crate::editor_project::component_transaction_error(
            crate::components::ComponentError::new(
                "COMPONENT_INTENT_INVALID",
                "Provide 1..256 component commands",
            ),
        ));
    }
    for intent in &mut intents {
        if let crate::components::ComponentIntent::InstantiateComponent { instance_id, .. } = intent
        {
            if instance_id.is_none() {
                *instance_id = Some(Default::default());
            }
        }
    }
    let mut project = crate::EditorProject::from_ai_source(state);
    crate::components::ComponentService::apply_intents(&mut project, &intents)
        .map_err(crate::editor_project::component_transaction_error)?;
    let candidate = project.ai_design_state();
    let mut proposal = propose_transaction(
        &candidate,
        OperationTransaction::new(Some("Component actions".into())),
    )?;
    proposal.base_revision = state.revision;
    proposal.summary.operation_count = intents.len();
    proposal.component_intents = intents;
    Ok(proposal)
}
