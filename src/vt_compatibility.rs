//! ISO 11783-6:2018 version compatibility rules shared by authoring and validation.
//!
//! The version boundaries in this module follow Annex A.1 and A.3. Optional VT
//! capabilities are deliberately not treated as mandatory compatibility errors.

use ag_iso_stack::object_pool::{
    object::Object,
    object_attributes::{Event, FontSize, FontType, VerticalAlignment},
    vt_version::VtVersion,
    ObjectId, ObjectPool, ObjectType,
};
use std::collections::{HashSet, VecDeque};

/// Describe the supported targets in the designer's VT2–VT6 range.
///
/// Returns `None` when no supported target satisfies `supports`. This does not
/// imply support for optional terminal hardware capabilities.
pub fn availability_description(mut supports: impl FnMut(VtVersion) -> bool) -> Option<String> {
    let versions: Vec<u8> = (2..=6)
        .filter(|number| supports(VtVersion::try_from(*number).unwrap()))
        .collect();
    let first = *versions.first()?;
    if versions == (first..=6).collect::<Vec<_>>() {
        Some(format!("Available from VT{first}"))
    } else if versions.len() == 1 {
        Some(format!("Available only in VT{first}"))
    } else {
        Some(format!(
            "Available in {}",
            versions
                .iter()
                .map(|number| format!("VT{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Whether an object type is available in object pools for a VT version.
///
/// Annex A.1 footnotes are represented as introduction boundaries. Optional
/// objects remain available from their introduction version; a particular VT
/// may still omit optional support.
pub fn supported_object_type(object_type: ObjectType, version: VtVersion) -> bool {
    if matches!(
        object_type,
        ObjectType::AuxiliaryFunctionType1 | ObjectType::AuxiliaryInputType1
    ) {
        // Type 1 auxiliary objects are used by VT2 only; VT3 and later prohibit
        // Working Sets from including them (ISO 11783-6:2018, H.2).
        return version == VtVersion::Version2;
    }
    let since = match object_type {
        ObjectType::AuxiliaryFunctionType2
        | ObjectType::AuxiliaryInputType2
        | ObjectType::AuxiliaryControlDesignatorType2 => VtVersion::Version3,
        ObjectType::OutputList
        | ObjectType::ExtendedInputAttributes
        | ObjectType::ColourMap
        | ObjectType::ObjectLabelReferenceList
        | ObjectType::GraphicsContext
        | ObjectType::WindowMask
        | ObjectType::KeyGroup => VtVersion::Version4,
        ObjectType::ExternalObjectDefinition
        | ObjectType::ExternalReferenceName
        | ObjectType::ExternalObjectPointer
        | ObjectType::Animation => VtVersion::Version5,
        ObjectType::ColourPalette
        | ObjectType::GraphicData
        | ObjectType::WorkingSetSpecialControls
        | ObjectType::ScaledGraphic => VtVersion::Version6,
        _ => VtVersion::Version2,
    };
    version >= since
}

/// Whether an event ID is available to a macro at this VT version.
pub fn supported_event(object: &Object, event: Event, version: VtVersion) -> bool {
    let generation_support = match event {
        Event::Reserved | Event::UseExtendedMacro => false,
        Event::OnPointingEventPress | Event::OnPointingEventRelease => {
            version >= VtVersion::Version5
        }
        _ => true,
    };
    if !generation_support {
        return false;
    }
    matches!(
        event,
        Event::ProprietaryEvent1
            | Event::ProprietaryEvent2
            | Event::ProprietaryEvent3
            | Event::ProprietaryEvent4
            | Event::ProprietaryEvent5
            | Event::ProprietaryEvent6
            | Event::ProprietaryEvent7
            | Event::ProprietaryEvent8
            | Event::ProprietaryEvent9
            | Event::ProprietaryEvent10
            | Event::ProprietaryEvent11
            | Event::ProprietaryEvent12
            | Event::ProprietaryEvent13
            | Event::ProprietaryEvent14
            | Event::ProprietaryEvent15
    ) || crate::possible_events::get_possible_events(object).contains(&event)
}

/// Whether a named property is part of the requested VT version.
///
/// This is intended for authoring choice filtering. It does not indicate that
/// an existing serialized value should be hidden from repair editors.
pub fn supported_property(_object_type: ObjectType, property: &str, version: VtVersion) -> bool {
    match property {
        "vertical_justification"
        | "justification.vertical"
        | "wrap_on_hyphen"
        | "proportional"
        | "truncate"
        | "real_time_editing"
        | "suppress_border"
        | "transparent_background"
        | "disabled"
        | "no_border" => version >= VtVersion::Version4,
        "macro_id_16bit" | "extended_macro_reference" => version >= VtVersion::Version5,
        "wide_characters" | "utf16" | "extended_input_attributes" => version >= VtVersion::Version4,
        "alpha" | "colour_palette" | "scaled_graphic" => version >= VtVersion::Version6,
        "animation" => version >= VtVersion::Version5,
        // Existing properties remain editable on unsupported object types so
        // users can repair the object. Object availability has its own diagnostic.
        _ => true,
    }
}

/// Whether a specific flag can be enabled for the selected object at this VT version.
///
/// This covers version-defined flag introductions, not negotiated VT capabilities.
pub fn supported_property_flag(
    object_type: ObjectType,
    property: &str,
    flag: &str,
    version: VtVersion,
) -> bool {
    let introduced_in_v4 = match (object_type, property, flag) {
        (
            ObjectType::Button,
            "options",
            "suppress_border" | "transparent_background" | "disabled" | "no_border",
        ) => true,
        (ObjectType::InputString, "options", "wrap_on_hyphen")
        | (ObjectType::OutputString, "options", "wrap_on_hyphen")
        | (ObjectType::FontAttributes, "font_style", "proportional")
        | (ObjectType::InputNumber, "options", "truncate")
        | (ObjectType::OutputNumber, "options", "truncate")
        | (ObjectType::InputNumber, "options2", "real_time_editing")
        | (ObjectType::InputList, "options", "real_time_editing") => true,
        _ => false,
    };
    !introduced_in_v4 || version >= VtVersion::Version4
}

/// Whether a font encoding value is defined for the selected VT generation.
pub fn supported_font_type(font_type: FontType, version: VtVersion) -> bool {
    match (version >= VtVersion::Version4, font_type) {
        (_, FontType::Latin1 | FontType::Latin9) => true,
        (true, FontType::Latin2 | FontType::Latin4 | FontType::Cyrillic | FontType::Greek) => true,
        (true, FontType::Proprietary(_)) => true,
        (false, FontType::Proprietary(255)) => true,
        _ => false,
    }
}

/// Whether proportional font sizes are defined for this VT version.
pub fn supported_font_size(font_size: FontSize, version: VtVersion) -> bool {
    version >= VtVersion::Version4 || !matches!(font_size, FontSize::Proportional(_))
}

/// Whether a vertical text alignment value is available at this VT version.
pub fn supported_vertical_alignment(alignment: VerticalAlignment, version: VtVersion) -> bool {
    version >= VtVersion::Version4 || alignment == VerticalAlignment::Top
}

/// VT2/VT3 accept only canonical Boolean values; VT4+ defines all nonzero
/// values as TRUE for Input Boolean objects.
pub fn supported_input_boolean_value(value: u8, version: VtVersion) -> bool {
    version >= VtVersion::Version4 || value <= 1
}

/// Return the earliest version that supports an object type.
pub fn object_type_since(object_type: ObjectType) -> VtVersion {
    [
        VtVersion::Version2,
        VtVersion::Version3,
        VtVersion::Version4,
        VtVersion::Version5,
        VtVersion::Version6,
    ]
    .into_iter()
    .find(|version| supported_object_type(object_type, *version))
    .unwrap_or(VtVersion::Version6)
}

/// Validate compatibility rules beyond generic object/reference integrity.
pub fn validate_pool_compatibility(
    pool: &ObjectPool,
    version: VtVersion,
) -> Vec<crate::pool_validation::ValidationDiagnostic> {
    use crate::pool_validation::{ValidationDiagnostic, ValidationSeverity};
    let mut diagnostics = Vec::new();
    diagnostics.extend(crate::vt_strings::string_export_diagnostics(pool, version));
    let mut push = |object_id: u16, code: &str, property: &str, message: String| {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            message: format!("Property {property}: {message}"),
            affected_objects: Some(vec![object_id]),
            code: code.to_owned(),
        });
    };

    for object in pool.objects() {
        let id = object.id().value();
        let object_type = object.object_type();
        if !supported_object_type(object_type, version) {
            let requirement = if matches!(
                object_type,
                ObjectType::AuxiliaryFunctionType1 | ObjectType::AuxiliaryInputType1
            ) {
                "Type 1 auxiliary objects are valid only in VT2".to_owned()
            } else {
                format!(
                    "{:?} requires VT{} or later",
                    object_type,
                    u8::from(object_type_since(object_type))
                )
            };
            push(id, "VT_OBJECT_UNSUPPORTED", "object_type", requirement);
        }

        if let Object::InputBoolean(value) = object {
            if !supported_input_boolean_value(value.value as u8, version) {
                push(
                    id,
                    "VT_ATTRIBUTE_VALUE_UNSUPPORTED",
                    "value",
                    format!(
                        "value {} must be 0 or 1 for VT{}",
                        value.value as u8,
                        u8::from(version)
                    ),
                );
            }
        }

        // Annex A.2 event availability follows event IDs: 27 and 28 are
        // reserved before VT5; 255 is only an in-band macro reference marker.
        if let Some(references) = crate::operations::macro_refs(object) {
            for reference in references {
                let macro_id = u16::from(reference.macro_id);
                if macro_id > u8::MAX as u16 && version < VtVersion::Version5 {
                    push(
                        id,
                        "VT_MACRO_ID_UNSUPPORTED",
                        "macro_refs.macro_id",
                        format!("macro ID {macro_id} requires VT5 or later"),
                    );
                }
                if !supported_event(object, reference.event_id, version) {
                    push(
                        id,
                        "VT_EVENT_UNSUPPORTED",
                        "macro_refs.event_id",
                        format!(
                            "event {:?} is unavailable in VT{}",
                            reference.event_id,
                            u8::from(version)
                        ),
                    );
                }
                if macro_id > 65534 {
                    push(
                        id,
                        "VT_MACRO_ID_INVALID",
                        "macro_refs.macro_id",
                        format!("macro ID {macro_id} is reserved"),
                    );
                } else if !pool.objects().iter().any(|candidate| {
                    candidate.id().value() == macro_id && matches!(candidate, Object::Macro(_))
                }) {
                    push(
                        id,
                        "VT_MACRO_REFERENCE_INVALID",
                        "macro_refs.macro_id",
                        format!("macro ID {macro_id} does not identify a Macro object"),
                    );
                }
            }
        }

        if let Object::Macro(macro_object) = object {
            if macro_object.id.value() > u16::from(u8::MAX) && version < VtVersion::Version5 {
                push(
                    id,
                    "VT_MACRO_ID_UNSUPPORTED",
                    "id",
                    format!("macro IDs above 255 require VT5 or later (found {})", id),
                );
            }
            if let Err(error) =
                crate::vt_macro_commands::validate_macro_commands(&macro_object.commands, version)
            {
                push(id, "VT_MACRO_COMMAND_INVALID", "commands", error);
            }
        }

        if let Object::FontAttributes(font) = object {
            if !supported_font_type(font.font_type, version) {
                push(
                    id,
                    "VT_FONT_TYPE_UNSUPPORTED",
                    "font_type",
                    format!(
                        "font encoding {:?} is unavailable in VT{}",
                        font.font_type,
                        u8::from(version)
                    ),
                );
            }
            // The codec derives the proportional style bit from font_size,
            // as does the editor's mode selector. Validate that authoritative
            // mode so choosing a fixed size can repair an imported font.
            if !supported_font_size(font.font_size, version) {
                push(
                    id,
                    "VT_ATTRIBUTE_UNSUPPORTED",
                    "font_size",
                    "proportional font sizes require VT4 or later".into(),
                );
            }
            if let FontSize::Proportional(height) = font.font_size {
                if height < 8 {
                    push(
                        id,
                        "VT_ATTRIBUTE_VALUE_INVALID",
                        "font_size",
                        format!(
                            "proportional font height {height} is below the minimum of 8 pixels"
                        ),
                    );
                }
            }
        }

        let vertical_alignment = match object {
            Object::InputString(value) => Some(value.justification.vertical),
            Object::InputNumber(value) => Some(value.justification.vertical),
            Object::OutputString(value) => Some(value.justification.vertical),
            Object::OutputNumber(value) => Some(value.justification.vertical),
            _ => None,
        };
        if version < VtVersion::Version4
            && vertical_alignment
                .is_some_and(|alignment| !supported_vertical_alignment(alignment, version))
        {
            push(
                id,
                "VT_ATTRIBUTE_UNSUPPORTED",
                "justification.vertical",
                "vertical middle and bottom justification require VT4 or later".into(),
            );
        }

        if version < VtVersion::Version4 {
            match object {
                Object::Button(value) => {
                    for (flag, enabled) in [
                        ("suppress_border", value.options.suppress_border),
                        (
                            "transparent_background",
                            value.options.transparent_background,
                        ),
                        ("disabled", value.options.disabled),
                        ("no_border", value.options.no_border),
                    ] {
                        if enabled
                            && !supported_property_flag(object_type, "options", flag, version)
                        {
                            push(
                                id,
                                "VT_FLAG_UNSUPPORTED",
                                &format!("options.{flag}"),
                                format!("flag {flag} requires VT4 or later"),
                            );
                        }
                    }
                }
                Object::InputString(value) if value.options.wrap_on_hyphen => {
                    push(
                        id,
                        "VT_FLAG_UNSUPPORTED",
                        "options.wrap_on_hyphen",
                        "wrap on hyphen requires VT4 or later".into(),
                    );
                }
                Object::OutputString(value) if value.options.wrap_on_hyphen => {
                    push(
                        id,
                        "VT_FLAG_UNSUPPORTED",
                        "options.wrap_on_hyphen",
                        "wrap on hyphen requires VT4 or later".into(),
                    );
                }
                Object::InputNumber(value) => {
                    if value.options.truncate {
                        push(
                            id,
                            "VT_FLAG_UNSUPPORTED",
                            "options.truncate",
                            "truncate requires VT4 or later".into(),
                        );
                    }
                    if value.options2.real_time_editing {
                        push(
                            id,
                            "VT_FLAG_UNSUPPORTED",
                            "options2.real_time_editing",
                            "real-time editing requires VT4 or later".into(),
                        );
                    }
                }
                Object::InputList(value) if value.options.real_time_editing => {
                    push(
                        id,
                        "VT_FLAG_UNSUPPORTED",
                        "options.real_time_editing",
                        "real-time editing requires VT4 or later".into(),
                    );
                }
                Object::OutputNumber(value) if value.options.truncate => {
                    push(
                        id,
                        "VT_FLAG_UNSUPPORTED",
                        "options.truncate",
                        "truncate requires VT4 or later".into(),
                    );
                }
                _ => {}
            }
        }
    }
    diagnostics.extend(validate_hierarchy(pool, version));
    diagnostics
}

/// Validate structural references against every ancestor's containment set.
/// Context is part of the visited key so a shared DAG node is checked under
/// each distinct set of ancestor constraints; cycles terminate as the sets can
/// only shrink while descending.
fn validate_hierarchy(
    pool: &ObjectPool,
    version: VtVersion,
) -> Vec<crate::pool_validation::ValidationDiagnostic> {
    use crate::pool_validation::{ValidationDiagnostic, ValidationSeverity};

    let mut diagnostics = Vec::new();
    let mut incoming = HashSet::new();
    for object in pool.objects() {
        for child_id in structural_children(object) {
            incoming.insert(child_id.value());
            if let Some(Object::ObjectPointer(pointer)) = pool.object_by_id(child_id) {
                if let Some(target) = pointer.value.0 {
                    incoming.insert(target.value());
                }
            }
        }
    }

    let mut starts: Vec<_> = pool
        .objects()
        .iter()
        .filter(|object| !incoming.contains(&object.id().value()))
        .map(|object| object.id())
        .collect();
    let mut visited: HashSet<(u16, Option<Vec<u8>>)> = HashSet::new();
    let mut work = VecDeque::new();

    let mut traverse = |start: ObjectId, work: &mut VecDeque<(ObjectId, Option<Vec<u8>>)>| {
        work.push_back((start, None));
        while let Some((parent_id, inherited)) = work.pop_front() {
            let Some(parent) = pool.object_by_id(parent_id) else {
                continue;
            };
            let state_key = (
                parent_id.value(),
                inherited.as_ref().map(|types| {
                    let mut types: Vec<_> = types.iter().copied().collect();
                    types.sort_unstable();
                    types
                }),
            );
            if !visited.insert(state_key) {
                continue;
            }

            // Soft-key labels and mask contents begin separate presentation
            // branches. A mask containing a Key must not narrow the objects
            // referenced by that Key to only Key-legal types. The same applies
            // when a mask appears beneath an invalid parent edge: its own
            // descendants are still validated in the mask's hierarchy.
            let inherited = if matches!(
                parent.object_type(),
                ObjectType::WorkingSet
                    | ObjectType::Key
                    | ObjectType::DataMask
                    | ObjectType::AlarmMask
                    | ObjectType::SoftKeyMask
                    | ObjectType::WindowMask
            ) {
                None
            } else {
                inherited
            };
            // A pointer forwards the containing position, including through
            // other pointers. It does not establish its own containment set.
            if let Object::ObjectPointer(pointer) = parent {
                if let Some(target_id) = pointer.value.0 {
                    if let Some(target) = pool.object_by_id(target_id) {
                        if inherited.as_ref().is_some_and(|allowed| {
                            !allowed.contains(&u8::from(target.object_type()))
                        }) {
                            diagnostics.push(ValidationDiagnostic {
                                severity: ValidationSeverity::Error,
                                message: format!(
                                    "Property value: {:?} is not allowed under this VT{} hierarchy",
                                    target.object_type(),
                                    u8::from(version)
                                ),
                                affected_objects: Some(vec![parent_id.value(), target_id.value()]),
                                code: "VT_INVALID_POINTER_TARGET_TYPE".into(),
                            });
                        }
                        work.push_back((target_id, inherited));
                    }
                }
                continue;
            }
            let local: HashSet<u8> = crate::allowed_object_relationships::get_allowed_child_refs(
                parent.object_type(),
                version,
            )
            .into_iter()
            .map(u8::from)
            .collect();
            let effective: HashSet<u8> = match inherited {
                Some(ancestor_types) => {
                    let ancestor_types: HashSet<u8> = ancestor_types.into_iter().collect();
                    local.intersection(&ancestor_types).copied().collect()
                }
                None => local,
            };
            for child_id in structural_children(parent) {
                let Some(child) = pool.object_by_id(child_id) else {
                    continue;
                };
                let child_type = u8::from(child.object_type());
                if !effective.contains(&child_type) {
                    diagnostics.push(ValidationDiagnostic {
                        severity: ValidationSeverity::Error,
                        message: format!(
                            "Property children: {:?} is not allowed under this VT{} hierarchy",
                            child.object_type(),
                            u8::from(version)
                        ),
                        affected_objects: Some(vec![parent_id.value(), child_id.value()]),
                        code: "VT_INVALID_CHILD_TYPE".to_owned(),
                    });
                }
                let mut next_context: Vec<_> = effective.iter().copied().collect();
                next_context.sort_unstable();
                let next_context = Some(next_context);
                work.push_back((child_id, next_context.clone()));
            }
        }
    };

    for start in starts.drain(..) {
        traverse(start, &mut work);
    }
    // Cyclic structural graphs have no root; validate each remaining component.
    for object in pool.objects() {
        // Already visited paths terminate immediately; this also starts each
        // structural component whose members form a cycle.
        traverse(object.id(), &mut work);
    }

    // Different ancestor paths can expose the same violation. Keep diagnostics
    // stable and singular for transaction baseline comparison.
    let mut seen = HashSet::new();
    diagnostics.retain(|diagnostic| {
        seen.insert((
            diagnostic.code.clone(),
            diagnostic.message.clone(),
            diagnostic.affected_objects.clone(),
        ))
    });
    diagnostics
}

fn structural_children(object: &Object) -> Vec<ObjectId> {
    crate::operations::object_refs(object)
        .into_iter()
        .flatten()
        .map(|reference| reference.id)
        .chain(match object {
            Object::SoftKeyMask(value) => value.objects.clone(),
            Object::KeyGroup(value) => value.objects.clone(),
            Object::WindowMask(value) => value
                .objects
                .iter()
                .filter_map(|reference| reference.0)
                .collect(),
            Object::InputList(value) => value
                .list_items
                .iter()
                .filter_map(|reference| reference.0)
                .collect(),
            Object::OutputList(value) => value
                .list_items
                .iter()
                .filter_map(|reference| reference.0)
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}
