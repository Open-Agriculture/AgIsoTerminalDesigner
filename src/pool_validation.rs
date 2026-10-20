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

/// Format diagnostic object identities using the editor's selected radix.
pub fn format_validation_diagnostic(
    diagnostic: &ValidationDiagnostic,
    format: crate::ObjectIdFormat,
) -> String {
    format_validation_diagnostic_with_names(diagnostic, format, &std::collections::HashMap::new())
}

/// Format a diagnostic with project names when a name is available.
///
/// The serialized diagnostic remains ID-based; names are supplied by the
/// owning project at presentation time so they always reflect current metadata.
pub fn format_validation_diagnostic_with_names(
    diagnostic: &ValidationDiagnostic,
    format: crate::ObjectIdFormat,
    names: &std::collections::HashMap<u16, String>,
) -> String {
    let Some(objects) = diagnostic
        .affected_objects
        .as_ref()
        .filter(|ids| !ids.is_empty())
    else {
        return diagnostic.message.clone();
    };
    let objects = objects
        .iter()
        .map(|id| format_object_identity(*id, format, names.get(id).map(String::as_str)))
        .collect::<Vec<_>>()
        .join(", ");
    let subject = if diagnostic
        .affected_objects
        .as_ref()
        .is_some_and(|ids| ids.len() > 1)
    {
        "Objects"
    } else {
        "Object"
    };
    format!("{subject} {objects}: {}", diagnostic.message)
}

/// Format one object's name and ID, falling back to its ID when unnamed.
pub(crate) fn format_object_identity(
    id: u16,
    format: crate::ObjectIdFormat,
    name: Option<&str>,
) -> String {
    match name.filter(|name| !name.trim().is_empty()) {
        Some(name) => format!("‘{name}’ ({})", format.format(id)),
        None => format.format(id),
    }
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
            message: "Cannot reference an object to itself".to_owned(),
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
                message: "Adding this reference would create a circular dependency".to_owned(),
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
            message: "Parent object not found".to_owned(),
            affected_objects: Some(vec![parent_id.value()]),
            code: "E003_PARENT_NOT_FOUND".to_string(),
        })?;

    let child = pool
        .object_by_id(child_id)
        .ok_or_else(|| ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: "Child object not found".to_owned(),
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

/// Check if an object ID is valid (0-65534; 65535 denotes NULL).
pub fn validate_object_id(id: u16) -> Result<ObjectId, ValidationDiagnostic> {
    if id == u16::MAX {
        return Err(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: "Object ID is invalid (must be 0-65534; 65535 denotes NULL)".to_owned(),
            affected_objects: Some(vec![id]),
            code: "E006_INVALID_OBJECT_ID".to_string(),
        });
    }
    ObjectId::new(id).map_err(|_| ValidationDiagnostic {
        severity: ValidationSeverity::Error,
        message: "Object ID is invalid (must be 0-65534; 65535 denotes NULL)".to_owned(),
        affected_objects: Some(vec![id]),
        code: "E006_INVALID_OBJECT_ID".to_string(),
    })
}

/// Check if an object exists in the pool
pub fn validate_object_exists(pool: &ObjectPool, id: ObjectId) -> Result<(), ValidationDiagnostic> {
    pool.object_by_id(id).ok_or_else(|| ValidationDiagnostic {
        severity: ValidationSeverity::Error,
        message: "Object not found in pool".to_owned(),
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

/// Validate references, wire constraints, and compatibility for an editable
/// pool. This incremental contract permits unresolved default references to
/// ID 0 and omits whole-pool cardinality and property-reference type checks.
/// Explicit reference assignments are validated by their operations. Use [`validate_complete_pool_state_for_version`] for project
/// diagnostics and export.
pub fn validate_pool_state(pool: &ObjectPool) -> Vec<ValidationDiagnostic> {
    validate_pool_state_for_version(pool, VtVersion::Version6)
}

/// Validate an editable pool using the project's VT version, permitting
/// unresolved ID 0 defaults as in [`validate_pool_state`]. Whole-pool
/// completeness and property-reference types are checked by
/// [`validate_complete_pool_state_for_version`].
pub fn validate_pool_state_for_version(
    pool: &ObjectPool,
    vt_version: VtVersion,
) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();

    // Check all object references for validity
    for obj in pool.objects() {
        use ag_iso_stack::object_pool::object::Object;
        // String encoding and target byte limits are checked together by
        // vt_strings. Check other record counts without encoding a draft's
        // Unicode text through the historical UTF-16 codec on legacy targets.
        let mut wire_object = matches!(
            obj,
            Object::InputString(_) | Object::OutputString(_) | Object::StringVariable(_)
        )
        .then(|| obj.clone());
        let empty = ag_iso_stack::object_pool::object::VtString::Encoded {
            encoded_bytes: vec![],
        };
        match &mut wire_object {
            Some(Object::InputString(s)) => s.value = empty,
            Some(Object::OutputString(s)) => s.value = empty,
            Some(Object::StringVariable(s)) => s.value = empty,
            _ => {}
        }
        if let Err(message) = wire_object
            .as_ref()
            .unwrap_or(obj)
            .validate_wire_for_version(vt_version)
        {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                message,
                affected_objects: Some(vec![obj.id().value()]),
                code: "VT_WIRE_INVALID".into(),
            });
        }
        for referenced_id in references_for_validation(obj, vt_version) {
            // Object ID zero is a temporary unresolved value in authoring
            // defaults. Complete-pool validation below checks it as a normal
            // ID so incomplete drafts remain editable.
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

    // Fixed-width text fit is advisory: it does not affect whether a pool can
    // be serialized or executed. This uses the preview's own text normalization
    // and wrapping logic so valid wrapping does not create false positives.
    for obj in pool.objects() {
        if let ag_iso_stack::object_pool::object::Object::OutputString(output) = obj {
            let Some(clipping) = crate::output_string_rendering::clipping(output, pool) else {
                continue;
            };
            if !clipping.horizontal && !clipping.vertical {
                continue;
            }
            let dimensions = match (clipping.horizontal, clipping.vertical) {
                (true, true) => "horizontally and vertically",
                (true, false) => "horizontally",
                (false, true) => "vertically",
                (false, false) => unreachable!(),
            };
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Warning,
                message: format!("Output String text is clipped {dimensions}"),
                affected_objects: Some(vec![obj.id().value()]),
                code: "TEXT_CLIPPED".into(),
            });
        }
    }

    diagnostics.extend(crate::vt_compatibility::validate_pool_compatibility(
        pool, vt_version,
    ));

    diagnostics
}

/// Validate a pool that is expected to be complete enough for project
/// diagnostics or IOP export. Incremental validation deliberately omits these
/// whole-pool requirements so drafts remain editable and saveable.
pub fn validate_complete_pool_state_for_version(
    pool: &ObjectPool,
    vt_version: VtVersion,
) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = validate_pool_state_for_version(pool, vt_version);
    diagnostics.extend(validate_complete_pool_requirements(pool, vt_version));
    diagnostics
}

/// Checks only whole-pool requirements and references whose ID 0 cannot be
/// treated as an unresolved authoring placeholder.
pub(crate) fn validate_complete_pool_requirements(
    pool: &ObjectPool,
    vt_version: VtVersion,
) -> Vec<ValidationDiagnostic> {
    use ag_iso_stack::object_pool::{object::Object, ObjectType};

    let mut diagnostics = Vec::new();
    for object in pool.objects() {
        let mut reported = HashSet::new();
        for referenced_id in references_for_validation(object, vt_version) {
            if referenced_id.value() == 0
                && reported.insert(referenced_id)
                && pool.object_by_id(referenced_id).is_none()
            {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: "Object not found in pool".into(),
                    affected_objects: Some(vec![0]),
                    code: "E007_OBJECT_NOT_FOUND".into(),
                });
            }
        }
    }
    let working_sets: Vec<_> = pool
        .objects()
        .iter()
        .filter(|object| object.object_type() == ObjectType::WorkingSet)
        .collect();
    if working_sets.len() != 1 {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!(
                "Object pool must contain exactly one Working Set (found {})",
                working_sets.len()
            ),
            affected_objects: working_sets.first().map(|object| vec![object.id().value()]),
            code: "WORKING_SET_CARDINALITY".into(),
        });
    }
    for object in &working_sets {
        if matches!(object, Object::WorkingSet(ws) if ws.object_refs.is_empty()) {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                message: "Working Set must contain at least one identifier object".into(),
                affected_objects: Some(vec![object.id().value()]),
                code: "WORKING_SET_IDENTIFIER_MISSING".into(),
            });
        }
    }
    let special_controls: Vec<_> = pool
        .objects()
        .iter()
        .filter(|object| object.object_type() == ObjectType::WorkingSetSpecialControls)
        .collect();
    if special_controls.len() > 1 {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!("Object pool may contain at most one Working Set Special Controls object (found {})", special_controls.len()),
            affected_objects: Some(special_controls.iter().map(|object| object.id().value()).collect()),
            code: "WORKING_SET_SPECIAL_CONTROLS_CARDINALITY".into(),
        });
    }

    // Descriptor metadata is the authority for typed object properties. The
    // version table also identifies attributes the target ignores (notably the
    // pre-VT6 unselectable Working Set active mask).
    for object in pool.objects() {
        for descriptor in crate::object_properties::get_property_descriptors(object.object_type()) {
            let crate::object_properties::PropertySemantic::ObjectReference { allowed_types } =
                descriptor.semantic
            else {
                continue;
            };
            if allowed_types.is_empty()
                || !crate::vt_compatibility::supported_property(
                    object.object_type(),
                    descriptor.name.as_str(),
                    vt_version,
                )
                || (object.object_type() == ObjectType::WorkingSet
                    && descriptor.name.as_str() == "active_mask"
                    && vt_version < VtVersion::Version6
                    && matches!(object, Object::WorkingSet(ws) if !ws.selectable))
                || (object.object_type() == ObjectType::FillAttributes
                    && descriptor.name.as_str() == "fill_pattern"
                    && matches!(object, Object::FillAttributes(fill) if fill.fill_type != 3))
            {
                continue;
            }
            let Ok(value) = crate::object_properties::get_object_property(object, descriptor.name)
            else {
                continue;
            };
            let Some(id) = value.as_u64().and_then(|id| u16::try_from(id).ok()) else {
                continue;
            };
            if id == u16::MAX {
                continue;
            }
            if let Some(target) = pool.object_by_id(ObjectId::new(id).expect("u16 object ID")) {
                let versioned_input_attributes = object.object_type() == ObjectType::InputString
                    && descriptor.name.as_str() == "input_attributes"
                    && target.object_type() == ObjectType::ExtendedInputAttributes;
                if !allowed_types.contains(&target.object_type())
                    || (versioned_input_attributes && vt_version < VtVersion::Version4)
                {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message: format!(
                            "{} references {:?}, but this property requires {}",
                            descriptor.name,
                            target.object_type(),
                            allowed_types
                                .iter()
                                .map(|t| format!("{t:?}"))
                                .collect::<Vec<_>>()
                                .join(" or ")
                        ),
                        affected_objects: Some(vec![object.id().value(), id]),
                        code: "PROPERTY_REFERENCE_WRONG_TYPE".into(),
                    });
                }
            }
        }
    }
    diagnostics
}

fn references_for_validation(
    object: &ag_iso_stack::object_pool::object::Object,
    vt_version: VtVersion,
) -> Vec<ObjectId> {
    use ag_iso_stack::object_pool::object::Object;
    let mut references = object.referenced_objects();
    // Ignore reference attributes that the selected VT version or the owning
    // attribute value tells the VT to ignore (ISO tables B.2/B.50).
    if let Object::WorkingSet(ws) = object {
        if vt_version < VtVersion::Version6 && !ws.selectable {
            remove_reference_once(&mut references, ws.active_mask);
        }
    }
    if let Object::FillAttributes(fill) = object {
        if fill.fill_type != 3 {
            if let Some(pattern) = fill.fill_pattern.0 {
                remove_reference_once(&mut references, pattern);
            }
        }
    }
    references
}

fn remove_reference_once(references: &mut Vec<ObjectId>, ignored_id: ObjectId) {
    if let Some(index) = references.iter().position(|id| *id == ignored_id) {
        references.remove(index);
    }
}

/// Return compatibility diagnostics whose violation is new or has worsened
/// relative to an existing repair draft. Matching includes the diagnostic text
/// so replacing one invalid value with another remains a rejected change.
pub(crate) fn new_or_worsened_compatibility_diagnostics(
    baseline: &[ValidationDiagnostic],
    candidate: &[ValidationDiagnostic],
) -> Vec<ValidationDiagnostic> {
    use std::collections::HashMap;
    fn is_compatibility(diagnostic: &ValidationDiagnostic) -> bool {
        diagnostic.code.starts_with("VT_")
    }
    fn key(diagnostic: &ValidationDiagnostic) -> (String, String, Vec<u16>) {
        (
            diagnostic.code.clone(),
            diagnostic.message.clone(),
            diagnostic.affected_objects.clone().unwrap_or_default(),
        )
    }
    let mut remaining = HashMap::new();
    for diagnostic in baseline.iter().filter(|d| is_compatibility(d)) {
        *remaining.entry(key(diagnostic)).or_insert(0usize) += 1;
    }
    candidate
        .iter()
        .filter(|diagnostic| is_compatibility(diagnostic))
        .filter_map(|diagnostic| {
            let count = remaining.entry(key(diagnostic)).or_insert(0);
            if *count > 0 {
                *count -= 1;
                None
            } else {
                Some(diagnostic.clone())
            }
        })
        .collect()
}

/// Preserve the identity of existing violations during object renumbering.
pub(crate) fn remap_compatibility_diagnostic_ids(
    diagnostics: &mut [ValidationDiagnostic],
    id_changes: &[(u16, u16)],
) {
    let mapping: std::collections::HashMap<_, _> = id_changes.iter().copied().collect();
    for diagnostic in diagnostics {
        if let Some(ids) = &mut diagnostic.affected_objects {
            for id in ids {
                if let Some(new_id) = mapping.get(id) {
                    *id = *new_id;
                }
            }
        }
    }
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
