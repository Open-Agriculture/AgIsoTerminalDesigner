//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//!
//! Consolidated pool and graph validation logic
//! Single source of truth for constraint checking across the application
//! Used by UI, operations engine, and AI

use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectId, ObjectPool};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

/// Validation severity for diagnostic messages
/// Controls behavior: Error blocks execution, Warning/Info allowed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationSeverity {
    /// Blocks transaction execution
    Error,
    /// Allows execution but reports issue
    Warning,
    /// Informational only
    Info,
}

/// A single validation diagnostic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationDiagnostic {
    pub severity: ValidationSeverity,
    pub message: String,
    pub affected_objects: Option<Vec<u16>>, // Store as u16 for serialization
    pub code: String,                       // e.g., "E001_CIRCULAR_REF"
}

/// Check if adding a reference from parent_id to child_id would create a circular dependency
/// Performs breadth-first search to detect cycles
pub fn would_create_circular_reference(
    pool: &ObjectPool,
    parent_id: ObjectId,
    child_id: ObjectId,
) -> Result<(), ValidationDiagnostic> {
    // Self-reference is always circular
    if parent_id == child_id {
        return Err(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!("Object {} cannot reference itself", parent_id.value()),
            affected_objects: Some(vec![parent_id.value()]),
            code: "E001_SELF_REFERENCE".to_string(),
        });
    }

    // Check if child already references parent (directly or indirectly)
    // Use breadth-first search for efficiency
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(child_id);

    while let Some(current_id) = queue.pop_front() {
        // If we've already visited this object, skip it (prevents infinite loops)
        if !visited.insert(current_id) {
            continue;
        }

        // If we find the parent in the descendants of child, it's circular
        if current_id == parent_id {
            let cycle_path = build_cycle_path(pool, parent_id, child_id);
            return Err(ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                message: format!(
                    "Adding reference {} → {} would create a circular dependency",
                    parent_id.value(),
                    child_id.value()
                ),
                affected_objects: Some(cycle_path),
                code: "E002_CIRCULAR_REFERENCE".to_string(),
            });
        }

        // Add all children of current object to the queue
        if let Some(obj) = pool.object_by_id(current_id) {
            for child_ref_id in obj.referenced_objects() {
                if !visited.contains(&child_ref_id) {
                    queue.push_back(child_ref_id);
                }
            }
        }
    }

    Ok(())
}

/// Validate parent-child relationship according to object types and VT version
/// Uses get_allowed_child_refs() as source of truth
pub fn validate_parent_child_relationship(
    pool: &ObjectPool,
    parent_id: ObjectId,
    child_id: ObjectId,
) -> Result<(), ValidationDiagnostic> {
    validate_parent_child_relationship_for_version(pool, parent_id, child_id, VtVersion::Version6)
}

/// Validate a relationship using the project's authoritative VT version.
pub fn validate_parent_child_relationship_for_version(
    pool: &ObjectPool,
    parent_id: ObjectId,
    child_id: ObjectId,
    vt_version: VtVersion,
) -> Result<(), ValidationDiagnostic> {
    let parent = pool
        .object_by_id(parent_id)
        .ok_or_else(|| ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!("Parent object {} not found", parent_id.value()),
            affected_objects: Some(vec![parent_id.value()]),
            code: "E003_PARENT_NOT_FOUND".to_string(),
        })?;

    let child = pool
        .object_by_id(child_id)
        .ok_or_else(|| ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!("Child object {} not found", child_id.value()),
            affected_objects: Some(vec![child_id.value()]),
            code: "E004_CHILD_NOT_FOUND".to_string(),
        })?;

    let allowed = crate::allowed_object_relationships::get_allowed_child_refs(
        parent.object_type(),
        vt_version,
    );
    if !allowed.contains(&child.object_type()) {
        return Err(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!(
                "{} cannot contain {} for VT {:?}",
                format!("{:?}", parent.object_type()),
                format!("{:?}", child.object_type()),
                vt_version
            ),
            affected_objects: Some(vec![parent_id.value(), child_id.value()]),
            code: "INVALID_CHILD_TYPE".to_owned(),
        });
    }

    Ok(())
}

/// Check if an object ID is valid (1-65534; 0 and 65535 are reserved as NULL)
pub fn validate_object_id(id: u16) -> Result<ObjectId, ValidationDiagnostic> {
    if id == 0 || id == u16::MAX {
        return Err(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!("Object ID {} is invalid (must be 1-65534)", id),
            affected_objects: None,
            code: "E006_INVALID_OBJECT_ID".to_string(),
        });
    }
    ObjectId::new(id).map_err(|_| ValidationDiagnostic {
        severity: ValidationSeverity::Error,
        message: format!("Object ID {} is invalid (must be 1-65534)", id),
        affected_objects: None,
        code: "E006_INVALID_OBJECT_ID".to_string(),
    })
}

/// Check if an object exists in the pool
pub fn validate_object_exists(pool: &ObjectPool, id: ObjectId) -> Result<(), ValidationDiagnostic> {
    pool.object_by_id(id).ok_or_else(|| ValidationDiagnostic {
        severity: ValidationSeverity::Error,
        message: format!("Object {} not found in pool", id.value()),
        affected_objects: Some(vec![id.value()]),
        code: "E007_OBJECT_NOT_FOUND".to_string(),
    })?;
    Ok(())
}

/// Check if a parent-child reference is still valid
/// (both objects exist and relationship is allowed)
pub fn validate_reference_integrity(
    pool: &ObjectPool,
    parent_id: ObjectId,
    child_id: ObjectId,
) -> Result<(), ValidationDiagnostic> {
    validate_object_exists(pool, parent_id)?;
    validate_object_exists(pool, child_id)?;
    validate_parent_child_relationship(pool, parent_id, child_id)?;
    Ok(())
}

/// Get all objects that reference the given object
pub fn get_referencing_objects(pool: &ObjectPool, object_id: ObjectId) -> Vec<ObjectId> {
    pool.objects()
        .iter()
        .filter(|object| {
            object.id() != object_id
                && (object.referenced_objects().contains(&object_id)
                    || crate::operations::operation::macro_refs(object).is_some_and(|refs| {
                        refs.iter()
                            .any(|macro_ref| u16::from(macro_ref.macro_id) == object_id.value())
                    }))
        })
        .map(|object| object.id())
        .collect()
}

/// Comprehensive pool validation
/// Returns all validation diagnostics (errors, warnings, info)
pub fn validate_pool_state(pool: &ObjectPool) -> Vec<ValidationDiagnostic> {
    validate_pool_state_for_version(pool, VtVersion::Version6)
}

/// Comprehensive pool validation using the project's VT version.
pub fn validate_pool_state_for_version(
    pool: &ObjectPool,
    vt_version: VtVersion,
) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();

    // Check all object references for validity
    for obj in pool.objects() {
        for referenced_id in obj.referenced_objects() {
            // Default editor objects use ObjectId(0) as an unresolved required
            // reference until the user assigns one. It is not a pool object.
            if referenced_id.value() == 0 {
                continue;
            }
            // `referenced_objects` also contains property references (for
            // example OutputNumber → NumberVariable), not only child-list
            // edges. Structural operations validate their child edge against
            // the allowed-child table at application time; globally we can
            // authoritatively validate the invariant shared by all reference
            // kinds: the target exists.
            if let Err(diag) = validate_object_exists(pool, referenced_id) {
                diagnostics.push(diag);
            }
        }
    }

    diagnostics
}

/// Build the cycle path for diagnostic reporting
/// Returns the sequence of object IDs that form the cycle
fn build_cycle_path(pool: &ObjectPool, parent_id: ObjectId, child_id: ObjectId) -> Vec<u16> {
    let mut path = vec![child_id.value(), parent_id.value()];
    let mut current = parent_id;

    // Try to trace back to child to show the full cycle
    for _ in 0..100 {
        // Prevent infinite loops
        if let Some(obj) = pool.object_by_id(current) {
            let refs = obj.referenced_objects();
            if let Some(next_id) = refs.first() {
                if *next_id == child_id {
                    return path;
                }
                current = *next_id;
                path.push(current.value());
            } else {
                break;
            }
        } else {
            break;
        }
    }

    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_object_id_valid() {
        assert!(validate_object_id(1).is_ok());
        assert!(validate_object_id(100).is_ok());
        assert!(validate_object_id(65534).is_ok());
    }

    #[test]
    fn test_validate_object_id_invalid() {
        assert!(validate_object_id(0).is_err());
        assert!(validate_object_id(65535).is_err());
    }
}
