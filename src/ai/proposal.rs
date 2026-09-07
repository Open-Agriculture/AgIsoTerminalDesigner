use super::snapshot::ObjectSummary;
use super::{AiDesignState, ProjectRevision};
use crate::operations::{
    Operation, OperationExecutor, OperationTransaction, PreparedTransaction,
    TransactionDiagnostics, ValidationContext,
};
use ag_iso_stack::object_pool::ObjectId;
use uuid::Uuid;
#[derive(Debug, Clone)]
pub struct AiProposal {
    pub proposal_id: String,
    pub base_revision: ProjectRevision,
    pub transaction: OperationTransaction,
    pub prepared: PreparedTransaction,
    pub summary: AiProposalSummary,
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
    let prepared = OperationExecutor::new().prepare_transaction(
        &state.pool,
        &state.object_info,
        &transaction,
        &ValidationContext {
            vt_version: state.vt_version,
            mask_size: state.mask_size,
        },
    )?;
    let mut summary = AiProposalSummary {
        operation_count: transaction.operations.len(),
        ..Default::default()
    };
    for operation in &transaction.operations {
        match operation {
            Operation::CreateObject { .. } => summary.created += 1,
            Operation::DeleteObject { .. } => summary.deleted += 1,
            Operation::RenameObject { .. } => summary.renamed += 1,
            Operation::SetProperty { .. } | Operation::SetObjectReference { .. } => {
                summary.property_changes += 1
            }
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
        base_revision: state.revision,
        transaction,
        prepared,
        summary,
    })
}
