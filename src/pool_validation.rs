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

/// Checks complete-pool cardinalities, collection constraints, imported
/// property ranges, and references whose ID 0 cannot be treated as an
/// unresolved authoring placeholder.
pub(crate) fn validate_complete_pool_requirements(
    pool: &ObjectPool,
    vt_version: VtVersion,
) -> Vec<ValidationDiagnostic> {
    use ag_iso_stack::object_pool::{object::Object, ObjectType};

    let mut diagnostics = Vec::new();
    for object in pool.objects() {
        // Property ranges also validate imported records that bypassed the
        // authoring operation path.
        for descriptor in crate::object_properties::get_property_descriptors(object.object_type()) {
            let Some((minimum, maximum)) = descriptor.valid_range else {
                continue;
            };
            let Ok(value) = crate::object_properties::get_object_property(object, descriptor.name)
            else {
                continue;
            };
            let number = value.as_f64();
            let min = minimum.as_f64();
            let max = maximum.as_f64();
            if !matches!((number, min, max), (Some(value), Some(min), Some(max)) if value >= min && value <= max)
            {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: format!(
                        "{} is outside the allowed range {} through {}",
                        descriptor.name, minimum, maximum
                    ),
                    affected_objects: Some(vec![object.id().value()]),
                    code: "PROPERTY_OUT_OF_RANGE".into(),
                });
            }
        }

        match object {
            Object::ScaledGraphic(graphic) => {
                let mut reference = graphic.value.0;
                let mut visited = HashSet::new();
                while let Some(id) = reference {
                    let target = pool.object_by_id(id);
                    if !visited.insert(id) {
                        diagnostics.push(ValidationDiagnostic {
                            severity: ValidationSeverity::Error,
                            message: "Scaled Graphic contains a circular Object Pointer chain"
                                .into(),
                            affected_objects: Some(vec![graphic.id.value(), id.value()]),
                            code: "SCALED_GRAPHIC_POINTER_CYCLE".into(),
                        });
                        break;
                    }
                    match target {
                        Some(Object::ObjectPointer(pointer)) => reference = pointer.value.0,
                        Some(Object::PictureGraphic(_) | Object::GraphicData(_)) | None => break,
                        Some(_) => {
                            diagnostics.push(ValidationDiagnostic {
                                severity: ValidationSeverity::Error,
                                message: "Scaled Graphic pointer must resolve to Picture Graphic, Graphic Data, or NULL".into(),
                                affected_objects: Some(vec![graphic.id.value(), id.value()]),
                                code: "SCALED_GRAPHIC_TARGET_INVALID".into(),
                            });
                            break;
                        }
                    }
                }
            }
            Object::Macro(macro_object) if macro_object.commands.is_empty() => {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: "Macro must contain at least one command packet".into(),
                    affected_objects: Some(vec![object.id().value()]),
                    code: "MACRO_EMPTY".into(),
                });
            }
            Object::PictureGraphic(picture) => {
                let data_length = decoded_picture_data_len(picture);
                let problem = match (picture_required_bytes(picture), data_length) {
                    (Err(error), _) | (_, Err(error)) => Some(("PICTURE_DATA_INVALID", error)),
                    (Ok(required), Ok(length)) if required > length => Some((
                        "PICTURE_DATA_TRUNCATED",
                        format!("Picture Graphic pixel data is truncated (need {required} decoded bytes)"),
                    )),
                    _ => None,
                };
                if let Some((code, message)) = problem {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message,
                        affected_objects: Some(vec![object.id().value()]),
                        code: code.into(),
                    });
                }
            }
            Object::GraphicData(graphic) => {
                if !graphic.data.is_empty() {
                    if let Err(message) = validate_graphic_data_png(&graphic.data) {
                        diagnostics.push(ValidationDiagnostic {
                            severity: ValidationSeverity::Error,
                            message,
                            affected_objects: Some(vec![object.id().value()]),
                            code: "GRAPHIC_DATA_PNG_INVALID".into(),
                        });
                    }
                }
            }
            Object::OutputPolygon(polygon) if polygon.points.len() < 3 => {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: "Output Polygon must contain at least three points".into(),
                    affected_objects: Some(vec![object.id().value()]),
                    code: "POLYGON_TOO_FEW_POINTS".into(),
                });
            }
            Object::ColourMap(colour_map)
                if colour_map.colour_map.len() <= u16::MAX as usize
                    && !matches!(colour_map.colour_map.len(), 2 | 16 | 256) =>
            {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: "Colour Map must contain 2, 16 or 256 entries".into(),
                    affected_objects: Some(vec![object.id().value()]),
                    code: "COLOUR_MAP_INVALID_SIZE".into(),
                });
            }
            Object::ColourPalette(palette)
                if (257..=u16::MAX as usize).contains(&palette.colours.len()) =>
            {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: "Colour Palette may contain at most 256 entries".into(),
                    affected_objects: Some(vec![object.id().value()]),
                    code: "COLOUR_PALETTE_TOO_LARGE".into(),
                });
            }
            Object::KeyGroup(group) => {
                if group.objects.len() <= u8::MAX as usize
                    && !(1..=4).contains(&group.objects.len())
                {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message: "Key Group must list between one and four objects".into(),
                        affected_objects: Some(vec![object.id().value()]),
                        code: "KEY_GROUP_INVALID_SIZE".into(),
                    });
                }
                // Object Pointer has one target, so the entry limit also
                // limits the number of Keys after resolving pointers.
            }
            _ => {}
        }

        if let Object::FillAttributes(fill) = object {
            if fill.fill_type == 3 {
                if let Some(target) = fill.fill_pattern.0.and_then(|id| pool.object_by_id(id)) {
                    if let Object::PictureGraphic(picture) = target {
                        let width = usize::from(picture.actual_width);
                        let aligned = match picture.format {
                            ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat::Monochrome => width % 8 == 0,
                            ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat::FourBit => width % 2 == 0,
                            ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat::EightBit => true,
                        };
                        if !aligned {
                            diagnostics.push(ValidationDiagnostic {
                                severity: ValidationSeverity::Error,
                                message: "Fill pattern width must be byte aligned for its Picture Graphic format".into(),
                                affected_objects: Some(vec![object.id().value(), target.id().value()]),
                                code: "FILL_PATTERN_NOT_BYTE_ALIGNED".into(),
                            });
                        }
                    }
                }
            }
        }
    }

    diagnostics.extend(validate_required_references(pool));
    diagnostics.extend(validate_macro_graph(pool, vt_version));
    diagnostics.extend(validate_macro_command_targets(pool, vt_version));

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

fn validate_required_references(pool: &ObjectPool) -> Vec<ValidationDiagnostic> {
    use ag_iso_stack::object_pool::object::Object;
    let mut diagnostics = Vec::new();
    for object in pool.objects() {
        let required: Vec<(&str, Option<ObjectId>)> = match object {
            Object::InputBoolean(value) => vec![("foreground_colour", value.foreground_colour.0)],
            Object::InputString(value) => vec![("font_attributes", value.font_attributes.0)],
            Object::InputNumber(value) => vec![("font_attributes", value.font_attributes.0)],
            Object::OutputString(value) => vec![("font_attributes", value.font_attributes.0)],
            Object::OutputNumber(value) => vec![("font_attributes", value.font_attributes.0)],
            Object::OutputLine(value) => vec![("line_attributes", value.line_attributes.0)],
            Object::OutputRectangle(value) => vec![("line_attributes", value.line_attributes.0)],
            Object::OutputEllipse(value) => vec![("line_attributes", value.line_attributes.0)],
            Object::OutputPolygon(value) => vec![("line_attributes", value.line_attributes.0)],
            _ => Vec::new(),
        };
        for (property, value) in required {
            if value.is_none() {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: format!("Required reference {property} cannot be NULL"),
                    affected_objects: Some(vec![object.id().value()]),
                    code: "REQUIRED_REFERENCE_NULL".into(),
                });
            }
        }
    }
    diagnostics
}

fn validate_macro_command_targets(
    pool: &ObjectPool,
    version: VtVersion,
) -> Vec<ValidationDiagnostic> {
    use ag_iso_stack::object_pool::object::Object;
    let mut diagnostics = Vec::new();
    for object in pool.objects() {
        let Object::Macro(macro_object) = object else {
            continue;
        };
        let targets = match crate::vt_macro_commands::macro_command_targets(
            &macro_object.commands,
            version,
        ) {
            Ok(targets) => targets,
            Err(_) => continue,
        };
        for target in targets {
            let Some(id) = target.object_id else {
                if !target.nullable {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message: format!(
                            "Macro command at offset {} has a NULL target",
                            target.command_offset
                        ),
                        affected_objects: Some(vec![object.id().value()]),
                        code: "MACRO_TARGET_NULL".into(),
                    });
                }
                continue;
            };
            let resolved = pool
                .objects()
                .iter()
                .find(|candidate| candidate.id().value() == id);
            let Some(resolved) = resolved else {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: format!(
                        "Macro command at offset {} references missing object {id}",
                        target.command_offset
                    ),
                    affected_objects: Some(vec![object.id().value(), id]),
                    code: "MACRO_TARGET_NOT_FOUND".into(),
                });
                continue;
            };
            if !target.expected_types.is_empty()
                && !target.expected_types.contains(&resolved.object_type())
            {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    message: format!(
                        "Macro command at offset {} targets {:?}; expected {}",
                        target.command_offset,
                        resolved.object_type(),
                        target
                            .expected_types
                            .iter()
                            .map(|kind| format!("{kind:?}"))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    ),
                    affected_objects: Some(vec![object.id().value(), id]),
                    code: "MACRO_TARGET_WRONG_TYPE".into(),
                });
            }
        }
    }
    diagnostics
}

fn validate_macro_graph(pool: &ObjectPool, version: VtVersion) -> Vec<ValidationDiagnostic> {
    use ag_iso_stack::object_pool::object::Object;
    use ag_iso_stack::object_pool::object_attributes::Event;
    use std::collections::{HashMap, HashSet};
    let macros: HashSet<u16> = pool
        .objects()
        .iter()
        .filter_map(|object| matches!(object, Object::Macro(_)).then_some(object.id().value()))
        .collect();
    let mut edges: HashMap<u16, HashSet<u16>> = HashMap::new();
    for object in pool.objects() {
        let Object::Macro(macro_object) = object else {
            continue;
        };
        if let Ok(targets) =
            crate::vt_macro_commands::macro_command_targets(&macro_object.commands, version)
        {
            for command_target in targets {
                let Some(target) = command_target.object_id else {
                    continue;
                };
                if command_target.command == 0xBE || command_target.command == 0xBC {
                    if macros.contains(&target) {
                        edges.entry(object.id().value()).or_default().insert(target);
                    }
                    continue;
                }
                if let Some(target_object) = pool
                    .objects()
                    .iter()
                    .find(|candidate| candidate.id().value() == target)
                {
                    if let Some(references) = crate::operations::macro_refs(target_object) {
                        let event = match command_target.command {
                            0xA0 => Some(
                                if macro_object.commands.get(command_target.command_offset + 3)
                                    == Some(&0)
                                {
                                    Event::OnHide
                                } else {
                                    Event::OnShow
                                },
                            ),
                            0xA1 => Some(
                                if macro_object.commands.get(command_target.command_offset + 3)
                                    == Some(&0)
                                {
                                    Event::OnDisable
                                } else {
                                    Event::OnEnable
                                },
                            ),
                            0xA5 => Some(Event::OnChangeChildLocation),
                            0xA6 => Some(Event::OnChangeSize),
                            0xA7 => Some(Event::OnChangeBackgroundColour),
                            0xA8 | 0xB3 => Some(Event::OnChangeValue),
                            0xA9 => Some(Event::OnChangeEndPoint),
                            0xAA => Some(Event::OnChangeFontAttributes),
                            0xAB => Some(Event::OnChangeLineAttributes),
                            0xAC => Some(Event::OnChangeFillAttributes),
                            0xAD => Some(Event::OnChangeActiveMask),
                            0xAE => Some(Event::OnChangeSoftKeyMask),
                            0xAF | 0xB5 => Some(Event::OnChangeAttribute),
                            0xB0 => Some(Event::OnChangePriority),
                            0xB4 => Some(Event::OnChangeChildPosition),
                            _ => None,
                        };
                        let primary_operand = match command_target.command {
                            0xA5 | 0xB4 | 0xAD => command_target.operand_offset == 1,
                            0xAC => command_target.operand_offset == 1,
                            0xAE => command_target.operand_offset == 2,
                            _ => command_target.operand_offset == 1,
                        };
                        if !primary_operand {
                            continue;
                        }
                        let Some(event) = event else { continue };
                        for reference in references {
                            let event_macro = u16::from(reference.macro_id);
                            if reference.event_id == event && macros.contains(&event_macro) {
                                edges
                                    .entry(object.id().value())
                                    .or_default()
                                    .insert(event_macro);
                            }
                        }
                    }
                }
            }
        }
    }
    // Use iterative tri-colour DFS so a large imported pool cannot overflow
    // the native or wasm call stack.
    let mut state = HashMap::<u16, u8>::new();
    let mut cycle_path = None;
    for start in macros {
        if state.get(&start).copied().unwrap_or(0) != 0 {
            continue;
        }
        state.insert(start, 1);
        let mut active = vec![start];
        let mut active_index = HashMap::from([(start, 0usize)]);
        let children: Vec<u16> = edges
            .get(&start)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
        let mut frames = vec![(start, children, 0usize)];
        while let Some((node, children, next)) = frames.last_mut() {
            if *next == children.len() {
                let finished = *node;
                frames.pop();
                active.pop();
                active_index.remove(&finished);
                state.insert(finished, 2);
                continue;
            }
            let child = children[*next];
            *next += 1;
            match state.get(&child).copied().unwrap_or(0) {
                1 => {
                    let index = active_index[&child];
                    let mut path = active[index..].to_vec();
                    path.push(child);
                    cycle_path = Some(path);
                    break;
                }
                2 => {}
                _ => {
                    state.insert(child, 1);
                    active_index.insert(child, active.len());
                    active.push(child);
                    let children: Vec<u16> = edges
                        .get(&child)
                        .map(|set| set.iter().copied().collect())
                        .unwrap_or_default();
                    frames.push((child, children, 0));
                }
            }
        }
        if cycle_path.is_some() {
            break;
        }
    }
    cycle_path
        .map(|cycle| {
            vec![ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                message: "Macro references form a recursive execution loop".into(),
                affected_objects: Some(cycle),
                code: "MACRO_EXECUTION_CYCLE".into(),
            }]
        })
        .unwrap_or_default()
}

fn picture_required_bytes(
    picture: &ag_iso_stack::object_pool::object::PictureGraphic,
) -> Result<usize, String> {
    let width = usize::from(picture.actual_width);
    let height = usize::from(picture.actual_height);
    let row_bytes = match picture.format {
        ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat::Monochrome => {
            width.div_ceil(8)
        }
        ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat::FourBit => {
            width.div_ceil(2)
        }
        ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat::EightBit => width,
    };
    row_bytes
        .checked_mul(height)
        .ok_or_else(|| "Picture Graphic dimensions overflow".into())
}

fn decoded_picture_data_len(
    picture: &ag_iso_stack::object_pool::object::PictureGraphic,
) -> Result<usize, String> {
    use ag_iso_stack::object_pool::object_attributes::DataCodeType;
    match picture.options.data_code_type {
        DataCodeType::Raw => Ok(picture.data.len()),
        DataCodeType::RunLength => {
            if picture.data.len() % 2 != 0 {
                return Err("Picture Graphic has a truncated run-length pair".into());
            }
            let mut length = 0usize;
            for pair in picture.data.chunks_exact(2) {
                length = length
                    .checked_add(usize::from(pair[0]))
                    .ok_or_else(|| "Picture Graphic decoded data length overflows".to_owned())?;
            }
            Ok(length)
        }
    }
}

fn validate_graphic_data_png(bytes: &[u8]) -> Result<(), String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("Graphic Data is not a valid PNG: {error}"))?;
    let info = reader.info();
    let bits_per_pixel = info.color_type.samples() * info.bit_depth as usize;
    if bits_per_pixel > 32 {
        return Err("Graphic Data PNG exceeds the 32-bit RGBA maximum".into());
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "Graphic Data PNG dimensions overflow".to_owned())?;
    if size > png::Limits::default().bytes {
        return Err("Graphic Data PNG exceeds the decoder memory limit".into());
    }
    let mut output = vec![0; size];
    reader
        .next_frame(&mut output)
        .map_err(|error| format!("Graphic Data PNG cannot be decoded: {error}"))?;
    Ok(())
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
