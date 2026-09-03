//! Atomic transaction execution engine
//! Validates, executes operations, and collects inverses

use super::operation::{Operation, OperationContext, OperationError};
use super::transaction::{AppliedTransaction, OperationTransaction};
use crate::object_info::ObjectInfo;
use crate::pool_validation::{ValidationDiagnostic, ValidationSeverity};
use ag_iso_stack::object_pool::ObjectPool;
use std::collections::HashMap;
use uuid::Uuid;

/// Executes transactions with validation and atomicity
pub struct OperationExecutor;

impl OperationExecutor {
    /// Create a new executor
    pub fn new() -> Self {
        Self
    }

    /// Validate a transaction without executing it
    /// Returns diagnostics with Error, Warning, or Info severity
    /// Errors block execution; warnings/info do not
    pub fn validate(
        &self,
        transaction: &OperationTransaction,
        pool: &ObjectPool,
        _object_info: &HashMap<ObjectId, ObjectInfo>,
    ) -> Vec<ValidationDiagnostic> {
        // Perform pre-execution validation
        let mut diagnostics = Vec::new();

        for op in &transaction.operations {
            // Basic validation for each operation type
            match op {
                Operation::CreateObject { .. } => {
                    // Validate object type is known
                    // This will be expanded as we implement operation validation
                }
                Operation::DeleteObject { object_id, .. } => {
                    // Check object exists
                    if pool
                        .object_by_id(
                            ag_iso_stack::object_pool::ObjectId::new(*object_id)
                                .unwrap_or_default(),
                        )
                        .is_none()
                    {
                        diagnostics.push(ValidationDiagnostic {
                            severity: ValidationSeverity::Error,
                            message: format!("Object {} not found for deletion", object_id),
                            affected_objects: None,
                            code: "E008_DELETE_NOT_FOUND".to_string(),
                        });
                    }
                }
                Operation::SetProperty { .. } => {
                    // Validate property exists and is writable
                }
                Operation::ChangeObjectId { old_id, new_id } => {
                    if pool
                        .object_by_id(ObjectId::new(*old_id).unwrap_or_default())
                        .is_none()
                    {
                        diagnostics.push(ValidationDiagnostic {
                            severity: ValidationSeverity::Error,
                            message: format!("Object {} not found for ID change", old_id),
                            affected_objects: Some(vec![*old_id]),
                            code: "E009_CHANGE_ID_NOT_FOUND".to_string(),
                        });
                    }
                    if pool
                        .object_by_id(ObjectId::new(*new_id).unwrap_or_default())
                        .is_some()
                    {
                        diagnostics.push(ValidationDiagnostic {
                            severity: ValidationSeverity::Error,
                            message: format!("Object {} already exists", new_id),
                            affected_objects: Some(vec![*new_id]),
                            code: "E010_DUPLICATE_ID".to_string(),
                        });
                    }
                }
                Operation::ReorderObjects { .. } => {}
                Operation::AddChild { .. } => {
                    // Validate both objects exist and relationship is allowed
                }
                Operation::RemoveChild { .. } => {
                    // Validate both objects exist
                }
                Operation::SetChildPosition { .. } => {
                    // Validate position values
                }
                Operation::SetChildren { .. } => {
                    // Full validation is performed against the evolving working
                    // pool while the operation is applied.
                }
                Operation::SetObjectList { .. } => {
                    // Full validation is performed against the evolving working
                    // pool while the operation is applied.
                }
                Operation::SetMacroReferences { .. } => {
                    // Macro targets are validated while the operation is applied.
                }
                Operation::SetObjectLabels { .. } => {
                    // Label targets are validated while the operation is applied.
                }
                Operation::RenameObject { .. } => {
                    // Validate name is not empty
                }
            }
        }

        diagnostics
    }

    /// Execute a transaction atomically
    /// - Resolves temporary handles to ObjectIds
    /// - Clones pool and metadata ONCE per transaction
    /// - Validates before application
    /// - Applies operations, collecting inverses
    /// - Validates resulting state
    /// - Returns error if any step fails (no partial application)
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
        let start_time = std::time::Instant::now();
        let transaction_id = Uuid::new_v4().to_string();

        // Step 1: Validate all operations
        let diagnostics = self.validate(&transaction, pool, object_info);
        let has_errors = diagnostics
            .iter()
            .any(|d| d.severity == ValidationSeverity::Error);

        if has_errors {
            // Return first error encountered
            if let Some(diag) = diagnostics
                .iter()
                .find(|d| d.severity == ValidationSeverity::Error)
            {
                return Err(OperationError::PropertyError {
                    object_id: 0,
                    property: diag.code.clone(),
                    reason: diag.message.clone(),
                });
            }
        }

        // Step 2: Clone pool and metadata ONCE for atomicity
        let mut working_pool = pool.clone();
        let mut working_object_info = object_info.clone();
        let mut context = OperationContext::default();

        // Step 3: Resolve temporary handles
        self.resolve_handles(&transaction, &working_pool, &mut context)?;

        // Step 4: Apply operations, collecting inverses and concrete forward
        // operations. Concretizing CreateObject preserves allocated IDs and
        // editor metadata across redo.
        let mut forward_operations = Vec::with_capacity(transaction.operations.len());
        for op in &transaction.operations {
            match op.apply(&mut working_pool, &mut working_object_info, &mut context) {
                Ok(inverse) => {
                    let applied_forward = match (op, &inverse) {
                        (
                            Operation::CreateObject {
                                handle,
                                object_type,
                                name,
                                ..
                            },
                            Operation::DeleteObject { object_id, .. },
                        ) => {
                            let id = ObjectId::new(*object_id)
                                .map_err(|_| OperationError::ObjectNotFound(*object_id))?;
                            let object = working_pool
                                .object_by_id(id)
                                .ok_or(OperationError::ObjectNotFound(*object_id))?;
                            Operation::CreateObject {
                                handle: handle.clone(),
                                object_id: Some(*object_id),
                                object_type: object_type.clone(),
                                name: name.clone(),
                                captured_object: Some(serde_json::to_value(object).map_err(
                                    |error| OperationError::PropertyError {
                                        object_id: *object_id,
                                        property: "CreateObject".to_owned(),
                                        reason: error.to_string(),
                                    },
                                )?),
                                captured_info: working_object_info.get(&id).cloned(),
                            }
                        }
                        _ => op.clone(),
                    };
                    forward_operations.push(applied_forward);
                    context.inverse_operations.push(inverse);
                }
                Err(e) => {
                    // Operation failed; return error without modifying original pool
                    return Err(e);
                }
            }
        }

        // Step 5: Validate resulting pool state
        let final_diagnostics = crate::pool_validation::validate_pool_state(&working_pool);
        let has_errors = final_diagnostics
            .iter()
            .any(|d| d.severity == ValidationSeverity::Error);

        if has_errors {
            return Err(OperationError::PropertyError {
                object_id: 0,
                property: "VALIDATION_FAILED_POST_EXECUTION".to_string(),
                reason: "Pool validation failed after execution".to_string(),
            });
        }

        // Step 6: All good; return result
        let elapsed = start_time.elapsed();
        let applied = AppliedTransaction {
            transaction_id,
            description: transaction.description.clone(),
            forward_operations,
            inverse_operations: context.inverse_operations,
            affected_objects: {
                let mut ids: Vec<_> = context
                    .affected_objects
                    .iter()
                    .map(|id| id.value())
                    .collect();
                ids.sort_unstable();
                ids
            },
            execution_time_ms: elapsed.as_millis(),
        };

        Ok((working_pool, working_object_info, applied))
    }

    /// Resolve temporary handles to actual ObjectIds
    fn resolve_handles(
        &self,
        transaction: &OperationTransaction,
        pool: &ObjectPool,
        context: &mut OperationContext,
    ) -> Result<(), OperationError> {
        let mut reserved: std::collections::HashSet<_> =
            pool.objects().iter().map(|object| object.id()).collect();
        for op in &transaction.operations {
            if let Operation::CreateObject {
                handle, object_id, ..
            } = op
            {
                if let Some(h) = handle {
                    if context.handle_map.contains_key(h) {
                        return Err(OperationError::ResolveHandleFailed { handle: h.clone() });
                    }
                    let id = if let Some(value) = object_id {
                        ObjectId::new(*value).map_err(|_| OperationError::ResolveHandleFailed {
                            handle: h.clone(),
                        })?
                    } else {
                        (1..u16::MAX)
                            .filter_map(|value| ObjectId::new(value).ok())
                            .find(|id| !reserved.contains(id))
                            .ok_or_else(|| OperationError::ResolveHandleFailed {
                                handle: h.clone(),
                            })?
                    };
                    if !reserved.insert(id) {
                        return Err(OperationError::ObjectAlreadyExists(id.value()));
                    }
                    context.handle_map.insert(h.clone(), id);
                }
            }
        }
        Ok(())
    }
}

impl Default for OperationExecutor {
    fn default() -> Self {
        Self::new()
    }
}

// re-export for convenience
use ag_iso_stack::object_pool::ObjectId;
