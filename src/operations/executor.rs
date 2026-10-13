//! Authoritative, atomic transaction preparation engine.

use super::operation::{ObjectTarget, Operation, OperationContext, OperationError};
use super::transaction::{
    AppliedTransaction, OperationTransaction, CURRENT_OPERATION_SCHEMA_VERSION,
};
use crate::object_info::ObjectInfo;
use ag_iso_stack::object_pool::{
    object_attributes::ObjectRef, vt_version::VtVersion, ObjectId, ObjectPool,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const MAX_OPERATIONS_PER_TRANSACTION: usize = 256;
pub const MAX_DESCRIPTION_LENGTH: usize = 1_024;
pub const MAX_NAME_LENGTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub operation_index: Option<usize>,
    pub operation_type: Option<String>,
    pub object_id: Option<u16>,
    pub object_name: Option<String>,
    pub property: Option<String>,
    pub related_objects: Vec<u16>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransactionDiagnostics(pub Vec<TransactionDiagnostic>);

impl TransactionDiagnostics {
    fn single(diagnostic: TransactionDiagnostic) -> Self {
        Self(vec![diagnostic])
    }
    /// Compatibility conversion for existing UI callers still using OperationError.
    pub fn legacy_operation_error(&self) -> OperationError {
        let d = self.0.first();
        match d.map(|d| d.code.as_str()) {
            Some("UNSUPPORTED_SCHEMA_VERSION") => OperationError::UnsupportedSchemaVersion {
                expected: CURRENT_OPERATION_SCHEMA_VERSION,
                actual: d.and_then(|d| d.object_id).unwrap_or(0) as u32,
            },
            Some("OBJECT_NAME_NOT_FOUND") => OperationError::ObjectNameNotFound(
                d.and_then(|d| d.object_name.clone()).unwrap_or_default(),
            ),
            Some("DUPLICATE_OBJECT_NAME") => OperationError::DuplicateObjectName(
                d.and_then(|d| d.object_name.clone()).unwrap_or_default(),
            ),
            Some("AMBIGUOUS_OBJECT_NAME") => OperationError::AmbiguousObjectName(
                d.and_then(|d| d.object_name.clone()).unwrap_or_default(),
            ),
            Some("OBJECT_ALREADY_EXISTS") => {
                OperationError::ObjectAlreadyExists(d.and_then(|d| d.object_id).unwrap_or(0))
            }
            Some("OBJECT_NOT_FOUND") => {
                OperationError::ObjectNotFound(d.and_then(|d| d.object_id).unwrap_or(0))
            }
            Some("CIRCULAR_REFERENCE") => OperationError::CircularReference {
                parent: d.and_then(|d| d.object_id).unwrap_or(0),
                child: d
                    .and_then(|d| d.related_objects.get(1).copied())
                    .unwrap_or(0),
            },
            Some("DELETE_REFERENCED_OBJECT") => {
                OperationError::DeleteReferencedObject(d.and_then(|d| d.object_id).unwrap_or(0))
            }
            Some("REFERENCE_PROPERTY_REQUIRES_REFERENCE_OPERATION") => {
                OperationError::ReferencePropertyRequiresReferenceOperation {
                    object_id: d.and_then(|d| d.object_id).unwrap_or(0),
                    property: d.and_then(|d| d.property.clone()).unwrap_or_default(),
                }
            }
            Some("PROPERTY_IS_NOT_OBJECT_REFERENCE") => {
                OperationError::PropertyIsNotObjectReference {
                    object_id: d.and_then(|d| d.object_id).unwrap_or(0),
                    property: d.and_then(|d| d.property.clone()).unwrap_or_default(),
                }
            }
            Some("INVALID_OBJECT_ORDER") => OperationError::InvalidObjectOrder,
            _ => OperationError::PropertyError {
                object_id: d.and_then(|d| d.object_id).unwrap_or(0),
                property: d
                    .and_then(|d| d.property.clone())
                    .unwrap_or_else(|| "transaction".into()),
                reason: d
                    .map(|d| d.message.clone())
                    .unwrap_or_else(|| "Transaction preparation failed".into()),
            },
        }
    }
}

pub(crate) fn containment_transaction_error(
    error: crate::layout_containment::ContainmentError,
) -> TransactionDiagnostics {
    TransactionDiagnostics(vec![TransactionDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: error.code().into(),
        message: error.message,
        operation_index: None,
        operation_type: None,
        object_id: Some(error.child_id),
        object_name: None,
        property: None,
        related_objects: vec![error.parent_id, error.child_id],
    }])
}

#[derive(Debug, Clone, Copy)]
pub struct ValidationContext {
    pub vt_version: VtVersion,
    pub mask_size: u16,
}
impl Default for ValidationContext {
    fn default() -> Self {
        Self {
            vt_version: VtVersion::Version6,
            mask_size: 500,
        }
    }
}

/// Fully validated candidate state. It never mutates canonical project state.
#[derive(Debug, Clone)]
pub struct PreparedTransaction {
    pub transaction: OperationTransaction,
    pub pool: ObjectPool,
    pub object_info: HashMap<ObjectId, ObjectInfo>,
    pub applied: AppliedTransaction,
    pub diagnostics: Vec<TransactionDiagnostic>,
}

pub struct OperationExecutor;

/// Tracks reference provenance through a batch so a new placement cannot reuse
/// a legacy placement's overflow allowance, even when the old one is repaired.
struct PlacementProvenance(HashMap<u16, Vec<(ObjectRef, bool)>>);

impl PlacementProvenance {
    fn new(pool: &ObjectPool) -> Self {
        Self(
            pool.objects()
                .iter()
                .filter_map(|object| {
                    super::object_refs(object).map(|refs| {
                        (
                            object.id().value(),
                            refs.iter().map(|r| (r.clone(), false)).collect(),
                        )
                    })
                })
                .collect(),
        )
    }

    fn update(&mut self, operation: &Operation, inverse: &Operation, pool: &ObjectPool) {
        match (operation, inverse) {
            (Operation::AddChild { .. }, Operation::RemoveChild { parent_id, .. }) => {
                if let Some(reference) = ObjectId::new(*parent_id)
                    .ok()
                    .and_then(|id| pool.object_by_id(id))
                    .and_then(super::object_refs)
                    .and_then(|refs| refs.last())
                {
                    self.0
                        .entry(*parent_id)
                        .or_default()
                        .push((reference.clone(), true));
                }
            }
            (
                Operation::RemoveChild {
                    parent_id,
                    child_id,
                },
                _,
            ) => {
                if let Some(refs) = self.0.get_mut(parent_id) {
                    if let Some(index) = refs
                        .iter()
                        .position(|(reference, _)| reference.id.value() == *child_id)
                    {
                        refs.remove(index);
                    }
                }
            }
            (Operation::ChangeObjectId { old_id, new_id }, _) => {
                if let Some(refs) = self.0.remove(old_id) {
                    self.0.insert(*new_id, refs);
                }
                for refs in self.0.values_mut() {
                    for (reference, _) in refs {
                        if reference.id.value() == *old_id {
                            reference.id =
                                ObjectId::new(*new_id).expect("applied object ID is valid");
                        }
                    }
                }
            }
            _ => {}
        }
        let mut updated = HashMap::new();
        for object in pool.objects() {
            let Some(refs) = super::object_refs(object) else {
                continue;
            };
            let old = self.0.remove(&object.id().value()).unwrap_or_default();
            let mut matched = vec![false; old.len()];
            let mut origins = vec![None; refs.len()];
            // Preserve unchanged references across reorder. When a replacement
            // list adds repeated children, its first existing occurrences retain
            // their identity and the additional occurrences are new.
            let mut old_counts = HashMap::<ObjectId, usize>::new();
            let mut new_counts = HashMap::<ObjectId, usize>::new();
            for (reference, _) in &old {
                *old_counts.entry(reference.id).or_default() += 1;
            }
            for reference in refs {
                *new_counts.entry(reference.id).or_default() += 1;
            }
            for (index, reference) in refs.iter().enumerate() {
                if old_counts.get(&reference.id) != new_counts.get(&reference.id) {
                    continue;
                }
                if let Some(previous) = old
                    .iter()
                    .enumerate()
                    .position(|(i, (r, _))| !matched[i] && r == reference)
                {
                    matched[previous] = true;
                    origins[index] = Some(old[previous].1);
                }
            }
            let origins = refs
                .iter()
                .enumerate()
                .map(|(index, reference)| {
                    let is_new = origins[index].unwrap_or_else(|| {
                        if let Some(previous) = old
                            .iter()
                            .enumerate()
                            .position(|(i, (r, _))| !matched[i] && r.id == reference.id)
                        {
                            matched[previous] = true;
                            old[previous].1
                        } else {
                            true
                        }
                    });
                    (reference.clone(), is_new)
                })
                .collect();
            updated.insert(object.id().value(), origins);
        }
        self.0 = updated;
    }

    fn new_references(&self) -> HashSet<(u16, usize)> {
        self.0
            .iter()
            .flat_map(|(parent, refs)| {
                refs.iter()
                    .enumerate()
                    .filter_map(move |(index, (_, is_new))| is_new.then_some((*parent, index)))
            })
            .collect()
    }
}

impl OperationExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Prepare the final candidate atomically, including parent containment.
    /// Existing placement overflow may remain or decrease; new placements must
    /// fit fully. No intermediate operation state is committed or validated as
    /// the resulting layout.
    pub fn prepare_transaction(
        &self,
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, ObjectInfo>,
        transaction: &OperationTransaction,
        context: &ValidationContext,
    ) -> Result<PreparedTransaction, TransactionDiagnostics> {
        if transaction.schema_version != CURRENT_OPERATION_SCHEMA_VERSION {
            return Err(TransactionDiagnostics::single(TransactionDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "UNSUPPORTED_SCHEMA_VERSION".into(),
                message: format!(
                    "Unsupported schema version {}; supported version is {}",
                    transaction.schema_version, CURRENT_OPERATION_SCHEMA_VERSION
                ),
                operation_index: None,
                operation_type: None,
                object_id: Some(transaction.schema_version as u16),
                object_name: None,
                property: None,
                related_objects: vec![],
            }));
        }
        if transaction.operations.len() > MAX_OPERATIONS_PER_TRANSACTION
            || transaction
                .description
                .as_ref()
                .is_some_and(|d| d.len() > MAX_DESCRIPTION_LENGTH)
        {
            return Err(TransactionDiagnostics::single(TransactionDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "TRANSACTION_TOO_LARGE".into(),
                message: format!(
                    "Transaction exceeds limits (max {} operations and {} description bytes)",
                    MAX_OPERATIONS_PER_TRANSACTION, MAX_DESCRIPTION_LENGTH
                ),
                operation_index: None,
                operation_type: None,
                object_id: None,
                object_name: None,
                property: None,
                related_objects: vec![],
            }));
        }
        let started = std::time::Instant::now();
        let mut candidate_pool = pool.clone();
        let mut candidate_info = object_info.clone();
        let mut placements = PlacementProvenance::new(pool);
        let mut op_context = OperationContext {
            vt_version: context.vt_version,
            ..Default::default()
        };
        let mut forwards = Vec::with_capacity(transaction.operations.len());
        for (index, operation) in transaction.operations.iter().enumerate() {
            if let Some(name) = operation_name(operation) {
                if name.len() > MAX_NAME_LENGTH {
                    return Err(TransactionDiagnostics::single(operation_diagnostic(
                        index,
                        operation,
                        OperationError::DuplicateObjectName(name.to_owned()),
                        "TRANSACTION_TOO_LARGE",
                        "Object name exceeds maximum length",
                    )));
                }
            }
            match operation.apply(&mut candidate_pool, &mut candidate_info, &mut op_context) {
                Ok(inverse) => {
                    placements.update(operation, &inverse, &candidate_pool);
                    let forward = match (operation, &inverse) {
                        (Operation::CreateObject { .. }, Operation::DeleteObject { object_id }) => {
                            let id = ObjectId::new(*object_id).map_err(|_| {
                                TransactionDiagnostics::single(operation_diagnostic(
                                    index,
                                    operation,
                                    OperationError::ObjectNotFound(*object_id),
                                    "OBJECT_NOT_FOUND",
                                    "Allocated object ID is invalid",
                                ))
                            })?;
                            Operation::RestoreObject {
                                object: candidate_pool.object_by_id(id).cloned().ok_or_else(
                                    || {
                                        TransactionDiagnostics::single(operation_diagnostic(
                                            index,
                                            operation,
                                            OperationError::ObjectNotFound(*object_id),
                                            "OBJECT_NOT_FOUND",
                                            "Created object disappeared",
                                        ))
                                    },
                                )?,
                                info: candidate_info.get(&id).cloned(),
                            }
                        }
                        _ => operation.clone(),
                    };
                    forwards.push(forward);
                    op_context.inverse_operations.push(inverse);
                }
                Err(error) => {
                    return Err(TransactionDiagnostics::single(operation_diagnostic(
                        index, operation, error, "", "",
                    )))
                }
            }
        }
        crate::layout_containment::validate_new_placements(
            &candidate_pool,
            context.mask_size,
            &placements.new_references(),
        )
        .map_err(containment_transaction_error)?;
        let id_changes: Vec<_> = transaction
            .operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::ChangeObjectId { old_id, new_id } => Some((*old_id, *new_id)),
                _ => None,
            })
            .collect();
        crate::layout_containment::validate_change_with_id_changes(
            pool,
            &candidate_pool,
            context.mask_size,
            context.mask_size,
            &id_changes,
        )
        .map_err(containment_transaction_error)?;
        let diagnostics: Vec<_> = crate::pool_validation::validate_pool_state_for_version(
            &candidate_pool,
            context.vt_version,
        )
        .into_iter()
        .map(|d| TransactionDiagnostic {
            severity: match d.severity {
                crate::pool_validation::ValidationSeverity::Error => DiagnosticSeverity::Error,
                crate::pool_validation::ValidationSeverity::Warning => DiagnosticSeverity::Warning,
                crate::pool_validation::ValidationSeverity::Info => DiagnosticSeverity::Info,
            },
            code: if d.code.is_empty() {
                "INVALID_RESULTING_POOL".into()
            } else {
                d.code
            },
            message: d.message,
            operation_index: None,
            operation_type: None,
            object_id: None,
            object_name: None,
            property: None,
            related_objects: d.affected_objects.unwrap_or_default(),
        })
        .collect();
        if diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
        {
            return Err(TransactionDiagnostics(diagnostics));
        }
        let mut affected: Vec<_> = op_context
            .affected_objects
            .iter()
            .map(|id| id.value())
            .collect();
        affected.sort_unstable();
        Ok(PreparedTransaction {
            transaction: transaction.clone(),
            pool: candidate_pool,
            object_info: candidate_info,
            applied: AppliedTransaction {
                transaction_id: Uuid::new_v4().to_string(),
                description: transaction.description.clone(),
                forward_operations: forwards,
                inverse_operations: op_context.inverse_operations,
                affected_objects: affected,
                execution_time_ms: started.elapsed().as_millis(),
            },
            diagnostics,
        })
    }

    /// Validation cannot diverge from execution: it prepares and discards the candidate.
    pub fn validate_transaction(
        &self,
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, ObjectInfo>,
        transaction: &OperationTransaction,
        context: &ValidationContext,
    ) -> Result<(), TransactionDiagnostics> {
        self.prepare_transaction(pool, object_info, transaction, context)
            .map(|_| ())
    }

    /// Compatibility facade. Preparation is the only validation/execution implementation.
    pub fn execute(
        &self,
        transaction: OperationTransaction,
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, ObjectInfo>,
    ) -> Result<
        (
            ObjectPool,
            HashMap<ObjectId, ObjectInfo>,
            AppliedTransaction,
        ),
        OperationError,
    > {
        let prepared = self
            .prepare_transaction(
                pool,
                object_info,
                &transaction,
                &ValidationContext::default(),
            )
            .map_err(|d| d.legacy_operation_error())?;
        Ok((prepared.pool, prepared.object_info, prepared.applied))
    }
}

fn operation_name(operation: &Operation) -> Option<&str> {
    match operation {
        Operation::CreateObject { name, .. } | Operation::RenameObject { name, .. } => Some(name),
        _ => None,
    }
}
fn operation_type(operation: &Operation) -> String {
    match operation {
        Operation::CreateObject { .. } => "CreateObject",
        Operation::RestoreObject { .. } => "RestoreObject",
        Operation::DeleteObject { .. } => "DeleteObject",
        Operation::SetProperty { .. } => "SetProperty",
        Operation::SetObjectReference { .. } => "SetObjectReference",
        Operation::RestoreObjectReference { .. } => "RestoreObjectReference",
        Operation::ChangeObjectId { .. } => "ChangeObjectId",
        Operation::ReorderObjects { .. } => "ReorderObjects",
        Operation::AddChild { .. } => "AddChild",
        Operation::RemoveChild { .. } => "RemoveChild",
        Operation::SetChildPosition { .. } => "SetChildPosition",
        Operation::SetChildren { .. } => "SetChildren",
        Operation::SetObjectList { .. } => "SetObjectList",
        Operation::SetMacroReferences { .. } => "SetMacroReferences",
        Operation::SetObjectLabels { .. } => "SetObjectLabels",
        Operation::RenameObject { .. } => "RenameObject",
    }
    .into()
}
fn primary_target(operation: &Operation) -> Option<&ObjectTarget> {
    match operation {
        Operation::SetProperty { object, .. }
        | Operation::SetObjectReference { object, .. }
        | Operation::SetObjectList { object, .. }
        | Operation::SetMacroReferences { object, .. }
        | Operation::RenameObject { object, .. } => Some(object),
        Operation::AddChild { parent, .. } | Operation::SetChildren { parent, .. } => Some(parent),
        _ => None,
    }
}
fn operation_diagnostic(
    index: usize,
    operation: &Operation,
    error: OperationError,
    forced_code: &str,
    forced_message: &str,
) -> TransactionDiagnostic {
    let (mut code, mut message, object_id, property, related) = match error {
        OperationError::ObjectNotFound(id) => (
            "OBJECT_NOT_FOUND",
            format!("Object {} was not found", id),
            Some(id),
            None,
            vec![],
        ),
        OperationError::ObjectNameNotFound(n) => (
            "OBJECT_NAME_NOT_FOUND",
            format!("Object named {n:?} was not found"),
            None,
            None,
            vec![],
        ),
        OperationError::DuplicateObjectName(n) => (
            "DUPLICATE_OBJECT_NAME",
            format!("Object name {n:?} is already in use"),
            None,
            None,
            vec![],
        ),
        OperationError::AmbiguousObjectName(n) => (
            "AMBIGUOUS_OBJECT_NAME",
            format!("Object name {n:?} is ambiguous"),
            None,
            None,
            vec![],
        ),
        OperationError::ObjectAlreadyExists(id) => (
            "OBJECT_ALREADY_EXISTS",
            format!("Object {} already exists", id),
            Some(id),
            None,
            vec![],
        ),
        OperationError::CircularReference { parent, child } => (
            "CIRCULAR_REFERENCE",
            format!(
                "Adding {} → {} would create a circular reference",
                parent, child
            ),
            Some(parent),
            None,
            vec![parent, child],
        ),
        OperationError::InvalidReference {
            parent_type,
            child_type,
        } => (
            "INVALID_CHILD_TYPE",
            format!("Invalid reference from {parent_type} to {child_type}"),
            None,
            None,
            vec![],
        ),
        OperationError::DeleteReferencedObject(id) => (
            "DELETE_REFERENCED_OBJECT",
            format!("Object {} is still referenced", id),
            Some(id),
            None,
            vec![],
        ),
        OperationError::ReferencePropertyRequiresReferenceOperation {
            object_id,
            property,
        } => (
            "REFERENCE_PROPERTY_REQUIRES_REFERENCE_OPERATION",
            "Reference properties require SetObjectReference".into(),
            Some(object_id),
            Some(property),
            vec![],
        ),
        OperationError::PropertyIsNotObjectReference {
            object_id,
            property,
        } => (
            "PROPERTY_IS_NOT_OBJECT_REFERENCE",
            "Property is not an object reference".into(),
            Some(object_id),
            Some(property),
            vec![],
        ),
        OperationError::InvalidObjectOrder => (
            "INVALID_OBJECT_ORDER",
            "Object order must contain every object exactly once".into(),
            None,
            None,
            vec![],
        ),
        OperationError::PropertyError {
            object_id,
            property,
            reason,
        } => (
            "INVALID_PROPERTY_VALUE",
            reason,
            Some(object_id),
            Some(property),
            vec![],
        ),
        OperationError::UnsupportedSchemaVersion { expected, actual } => (
            "UNSUPPORTED_SCHEMA_VERSION",
            format!("Unsupported schema version {actual}; supported version is {expected}"),
            Some(actual as u16),
            None,
            vec![],
        ),
    };
    if !forced_code.is_empty() {
        code = forced_code;
        message = forced_message.into();
    }
    let object_name = primary_target(operation)
        .and_then(|t| match t {
            ObjectTarget::Name { name } => Some(name.clone()),
            _ => None,
        })
        .or_else(|| operation_name(operation).map(str::to_owned));
    TransactionDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: code.into(),
        message,
        operation_index: Some(index),
        operation_type: Some(operation_type(operation)),
        object_id,
        object_name,
        property,
        related_objects: related,
    }
}
impl Default for OperationExecutor {
    fn default() -> Self {
        Self::new()
    }
}
