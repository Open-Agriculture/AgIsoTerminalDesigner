//! Safe object-ID remapping for ISO 11783-6 Macro command streams.

use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::ObjectType;
use std::collections::HashMap;

use crate::vt_macro_commands::{command_spec, graphics_subcommand_length, validate_macro_commands};

/// Describes how a changed object attribute stores its value.
///
/// `LocalObjectId16` identifies a two-byte reference into this object pool.
/// `ForeignReference` is preserved because an external-pool reference is not
/// part of the ID mapping. `Unknown` blocks reassignment rather than risking
/// corruption of a value whose meaning is not known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacroAttributeValueKind {
    Numeric,
    LocalObjectId16 { value_offset: usize },
    ForeignReference,
    Unknown,
}

/// Classify the numeric value fields that contain object references for the
/// built-in ISO 11783-6 object types.
///
/// `aid` is `Some(AID)` for Change Attribute and `None` for Change Numeric
/// Value. The returned offset is relative to the first byte of the four-byte
/// value field. Unknown object types should be represented by the caller as
/// [`MacroAttributeValueKind::Unknown`]. Reference AIDs follow the object
/// tables in ISO 11783-6:2018 Annex B; Change Numeric Value layouts follow
/// Annex F.22. `version` restricts object and AID availability; unassigned
/// and read-only AIDs return `Unknown` and block reassignment.
pub fn macro_attribute_value_kind(
    object_type: ObjectType,
    aid: Option<u8>,
    version: VtVersion,
) -> MacroAttributeValueKind {
    use MacroAttributeValueKind::{ForeignReference, LocalObjectId16, Numeric};
    use ObjectType::*;
    if !crate::vt_compatibility::supported_object_type(object_type, version)
        || (object_type == Button && aid == Some(6) && version < VtVersion::Version4)
    {
        return MacroAttributeValueKind::Unknown;
    }

    match (object_type, aid) {
        // Pool-local scalar references (Annex B object tables).
        (DataMask | AlarmMask, Some(2))
        | (InputBoolean, Some(3 | 4))
        | (InputString, Some(4 | 5 | 7))
        | (InputNumber, Some(4 | 6))
        | (InputList | OutputList, Some(3))
        | (OutputString | OutputNumber, Some(4 | 6))
        | (OutputLine, Some(1))
        | (OutputRectangle, Some(1 | 5))
        | (OutputEllipse, Some(1 | 7))
        | (OutputPolygon, Some(3 | 4))
        | (FillAttributes, Some(3))
        | (OutputMeter, Some(11))
        | (OutputLinearBarGraph, Some(9 | 10))
        | (OutputArchedBarGraph, Some(11 | 12))
        | (WindowMask, Some(3))
        | (KeyGroup, Some(2))
        | (GraphicsContext, Some(12 | 13 | 14))
        | (ScaledGraphic, Some(5))
        | (AuxiliaryControlDesignatorType2, Some(2)) => LocalObjectId16 { value_offset: 0 },
        // External Object Pointer AID 1 is a local fallback object and AID 2
        // is a local External Reference NAME object. AID 3 names an object in
        // another Working Set's pool and must remain unchanged.
        (ExternalObjectPointer, Some(1 | 2)) => LocalObjectId16 { value_offset: 0 },
        (ExternalObjectPointer, Some(3)) => ForeignReference,

        // Change Numeric Value has object-specific payload positions in F.22.
        (ObjectPointer, None) => LocalObjectId16 { value_offset: 0 },
        (ExternalObjectPointer, None) => LocalObjectId16 { value_offset: 0 },
        // Animation's value is a one-byte list index. The F.22 Animation line
        // does not make that value an Object ID, even though list entries are.
        (Animation, None) => Numeric,
        (ScaledGraphic, None) => LocalObjectId16 { value_offset: 0 },
        (
            InputBoolean | InputNumber | InputList | OutputNumber | OutputList | OutputMeter
            | OutputLinearBarGraph | OutputArchedBarGraph | NumberVariable,
            None,
        ) => Numeric,
        (_, Some(aid)) if writable_attribute(object_type, aid) => Numeric,
        _ => MacroAttributeValueKind::Unknown,
    }
}

// Assigned, writable AIDs from Annex B and Tables J.2/J.4/J.6. Bracketed
// (read-only) AIDs and unassigned values are deliberately excluded.
fn writable_attribute(object_type: ObjectType, aid: u8) -> bool {
    use ObjectType::*;
    match object_type {
        DataMask => (1..=2).contains(&aid),
        AlarmMask => (1..=4).contains(&aid),
        SoftKeyMask => aid == 1,
        Key | KeyGroup => (1..=2).contains(&aid),
        Button => (1..=6).contains(&aid),
        InputBoolean | OutputLine | FontAttributes => (1..=4).contains(&aid),
        InputString => (1..=8).contains(&aid),
        InputNumber | OutputArchedBarGraph => (1..=13).contains(&aid),
        InputList | OutputList => (1..=3).contains(&aid),
        OutputString | OutputEllipse => (1..=7).contains(&aid),
        OutputNumber | OutputMeter | OutputLinearBarGraph => (1..=11).contains(&aid),
        OutputRectangle | OutputPolygon | ScaledGraphic => (1..=5).contains(&aid),
        PictureGraphic | LineAttributes | FillAttributes | WindowMask | ExternalObjectPointer => {
            (1..=3).contains(&aid)
        }
        GraphicsContext => (1..=17).contains(&aid) && !matches!(aid, 5 | 6),
        Animation => (1..=9).contains(&aid),
        ExternalObjectDefinition
        | ExternalReferenceName
        | ColourPalette
        | AuxiliaryFunctionType2
        | AuxiliaryInputType2 => aid == 1,
        AuxiliaryControlDesignatorType2 => aid == 2,
        _ => false,
    }
}

/// Remap references in a Macro packet stream without changing the input.
///
/// `attribute_kind` receives the target object ID and either `Some(aid)` for
/// Change Attribute or `None` for Change Numeric Value. It should return
/// `Unknown` when the target object or attribute cannot be resolved. All
/// changes are made to a temporary copy and returned only after the complete
/// stream has passed framing, version, and representability checks. Command
/// eligibility and packet framing follow ISO 11783-6:2018 Table A.5, Annex F,
/// and the Macro padding rule in Table B.56. Attribute values follow F.22 and
/// F.38; Graphics Context references follow F.56/Table F.1.
pub fn remap_macro_object_ids<F>(
    bytes: &[u8],
    mapping: &HashMap<u16, u16>,
    target_version: VtVersion,
    attribute_kind: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u16, Option<u8>) -> MacroAttributeValueKind,
{
    remap_macro_ids(bytes, mapping, target_version, attribute_kind, None, None)
}

/// Validate every local reference against the original pool before rewriting.
pub(crate) fn remap_macro_object_ids_checked(
    bytes: &[u8],
    mapping: &HashMap<u16, u16>,
    version: VtVersion,
    object_types: &HashMap<u16, ObjectType>,
) -> Result<Vec<u8>, String> {
    remap_macro_ids(
        bytes,
        mapping,
        version,
        |id, aid| {
            object_types
                .get(&id)
                .map_or(MacroAttributeValueKind::Unknown, |ty| {
                    macro_attribute_value_kind(*ty, aid, version)
                })
        },
        Some(object_types),
        None,
    )
}

/// Validate local IDs and typed attribute-reference payloads after redirection.
pub(crate) fn validate_macro_reference_values(
    bytes: &[u8],
    version: VtVersion,
    object_types: &HashMap<u16, ObjectType>,
) -> Result<(), String> {
    use crate::object_properties::{get_property_descriptors, PropertyKey, PropertySemantic};
    // An empty mapping validates the candidate stream without rewriting bytes.
    remap_macro_object_ids_checked(bytes, &HashMap::new(), version, object_types)?;
    let mut offset = 0;
    while offset < bytes.len() {
        let command = bytes[offset];
        let spec = command_spec(command).expect("checked command framing");
        if matches!(command, 0xA8 | 0xAF) {
            let owner = u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]);
            let object_type = object_types[&owner];
            let aid = (command == 0xAF).then(|| bytes[offset + 3]);
            if let MacroAttributeValueKind::LocalObjectId16 { value_offset } =
                macro_attribute_value_kind(object_type, aid, version)
            {
                let value_at = offset + 4 + value_offset;
                let target = u16::from_le_bytes([bytes[value_at], bytes[value_at + 1]]);
                if target != u16::MAX {
                    let property = match (object_type, aid) {
                        (_, None) => Some(PropertyKey::Value),
                        (ObjectType::WindowMask, Some(3)) | (ObjectType::KeyGroup, Some(2)) => {
                            Some(PropertyKey::Name)
                        }
                        (ObjectType::OutputLinearBarGraph, Some(10))
                        | (ObjectType::OutputArchedBarGraph, Some(12)) => {
                            Some(PropertyKey::TargetValueVariableReference)
                        }
                        (ObjectType::AuxiliaryControlDesignatorType2, Some(2)) => {
                            Some(PropertyKey::AuxiliaryObjectId)
                        }
                        (ObjectType::ExternalObjectPointer, Some(1)) => {
                            Some(PropertyKey::DefaultObjectId)
                        }
                        (ObjectType::ExternalObjectPointer, Some(2)) => {
                            Some(PropertyKey::ExternalReferenceNameId)
                        }
                        (_, Some(aid)) => crate::preview_commands::attribute_key(object_type, aid),
                    };
                    if let Some(descriptor) = get_property_descriptors(object_type)
                        .into_iter()
                        .find(|descriptor| Some(descriptor.name) == property)
                    {
                        if let PropertySemantic::ObjectReference { allowed_types } =
                            descriptor.semantic
                        {
                            let target_type = object_types[&target];
                            if !allowed_types.is_empty() && !allowed_types.contains(&target_type) {
                                return Err(format!("Macro attribute reference on object {owner} targets incompatible {target_type:?} object {target}"));
                            }
                        }
                    }
                }
            }
        }
        offset += frame_length(bytes, offset, command, spec.length)?;
    }
    Ok(())
}

/// Inspect local IDs with the same semantic layouts used for safe remapping.
/// Unknown attribute payload semantics return an error instead of guessing IDs.
pub(crate) fn macro_object_references(
    bytes: &[u8],
    version: VtVersion,
    object_types: &HashMap<u16, ObjectType>,
) -> Result<Vec<u16>, String> {
    let references = std::cell::RefCell::new(std::collections::BTreeSet::new());
    remap_macro_ids(
        bytes,
        &HashMap::new(),
        version,
        |id, aid| {
            object_types
                .get(&id)
                .map_or(MacroAttributeValueKind::Unknown, |ty| {
                    macro_attribute_value_kind(*ty, aid, version)
                })
        },
        None,
        Some(&references),
    )?;
    Ok(references.into_inner().into_iter().collect())
}

struct IdRemapping<'a> {
    ids: &'a HashMap<u16, u16>,
    object_types: Option<&'a HashMap<u16, ObjectType>>,
    references: Option<&'a std::cell::RefCell<std::collections::BTreeSet<u16>>>,
}

impl IdRemapping<'_> {
    fn get(&self, id: &u16) -> Option<&u16> {
        self.ids.get(id)
    }
    fn validate(&self, id: u16, macro_target: bool) -> Result<(), String> {
        if let Some(references) = self.references {
            references.borrow_mut().insert(id);
        }
        if let Some(types) = self.object_types {
            let ty = types
                .get(&id)
                .ok_or_else(|| format!("Macro references missing local object {id}"))?;
            if macro_target && *ty != ObjectType::Macro {
                return Err(format!("Execute Macro target {id} is not a Macro object"));
            }
        }
        Ok(())
    }
}

fn remap_macro_ids<F>(
    bytes: &[u8],
    mapping: &HashMap<u16, u16>,
    target_version: VtVersion,
    mut attribute_kind: F,
    object_types: Option<&HashMap<u16, ObjectType>>,
    references: Option<&std::cell::RefCell<std::collections::BTreeSet<u16>>>,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u16, Option<u8>) -> MacroAttributeValueKind,
{
    let mapping = &IdRemapping {
        ids: mapping,
        object_types,
        references,
    };
    validate_macro_commands(bytes, target_version)?;
    let mut remapped = bytes.to_vec();
    let mut offset = 0;

    while offset < bytes.len() {
        let command = bytes[offset];
        let spec = command_spec(command).expect("validated command has a specification");
        let frame_length = frame_length(bytes, offset, command, spec.length)?;
        let original_target_id = if matches!(command, 0xA8 | 0xAF) {
            Some(u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]))
        } else {
            None
        };

        match command {
            // Commands with one or more explicit Object ID fields.
            0xA0..=0xA2 | 0xA6..=0xA7 | 0xA9..=0xAB | 0xB0 | 0xB3 | 0xB6..=0xB7 | 0xBA => {
                remap_u16(&mut remapped, offset + 1, mapping)?
            }
            0xAC => {
                remap_u16(&mut remapped, offset + 1, mapping)?;
                remap_u16(&mut remapped, offset + 5, mapping)?;
            }
            0xAD => {
                remap_u16(&mut remapped, offset + 1, mapping)?;
                remap_u16(&mut remapped, offset + 3, mapping)?;
            }
            0xAE => {
                // Byte 2 is the Mask Type discriminator, not an object ID.
                remap_u16(&mut remapped, offset + 2, mapping)?;
                remap_u16(&mut remapped, offset + 4, mapping)?;
            }
            0xA5 | 0xB4 => {
                remap_u16(&mut remapped, offset + 1, mapping)?;
                remap_u16(&mut remapped, offset + 3, mapping)?;
            }
            0xA8 => {
                remap_u16(&mut remapped, offset + 1, mapping)?;
                remap_attribute_value(
                    &mut remapped,
                    offset,
                    original_target_id.expect("Change Numeric Value has an object ID"),
                    mapping,
                    &mut attribute_kind,
                    None,
                    4,
                )?;
            }
            0xAF => {
                remap_u16(&mut remapped, offset + 1, mapping)?;
                remap_attribute_value(
                    &mut remapped,
                    offset,
                    original_target_id.expect("Change Attribute has an object ID"),
                    mapping,
                    &mut attribute_kind,
                    Some(bytes[offset + 3]),
                    4,
                )?;
            }
            0xB1 => {
                remap_u16(&mut remapped, offset + 1, mapping)?;
                remap_u16(&mut remapped, offset + 4, mapping)?;
            }
            0xB5 => {
                remap_u16(&mut remapped, offset + 1, mapping)?;
                remap_u16(&mut remapped, offset + 3, mapping)?;
                remap_u16(&mut remapped, offset + 6, mapping)?;
            }
            0xB8 => remap_graphics_context(&mut remapped, offset, frame_length, mapping)?,
            // Lock/Unlock Mask has a command byte before its mask Object ID.
            0xBD => remap_u16(&mut remapped, offset + 2, mapping)?,
            // Execute Macro carries an 8-bit ID through VT4. On VT5+ it can
            // be represented by the extended command when the new ID is wide.
            0xBE => remap_execute_macro(&mut remapped, offset, mapping, target_version)?,
            0xBC => {
                mapping.validate(
                    u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]),
                    true,
                )?;
                remap_u16(&mut remapped, offset + 1, mapping)?;
            }
            // The Working Set name in Select Active Working Set is an ECU NAME,
            // not an object-pool ID. Audio and volume commands have no IDs.
            0x90 | 0xA3..=0xA4 => {}
            _ => {
                return Err(format!(
                    "macro command 0x{command:02X} at offset {offset} has no safe ID remapping rule"
                ));
            }
        }
        offset += frame_length;
    }

    Ok(remapped)
}

fn frame_length(bytes: &[u8], offset: usize, command: u8, fixed: usize) -> Result<usize, String> {
    match command {
        0xB3 => {
            let string_length = u16::from_le_bytes([bytes[offset + 3], bytes[offset + 4]]) as usize;
            Ok((5 + string_length).max(8))
        }
        0xB8 => {
            let subcommand_length = graphics_subcommand_length(bytes, offset + 3)?;
            Ok((3 + subcommand_length).max(8))
        }
        _ => Ok(fixed),
    }
}

fn remap_u16(bytes: &mut [u8], offset: usize, mapping: &IdRemapping<'_>) -> Result<(), String> {
    let current = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    // FFFF is the standard NULL Object ID and is not an object to reassign.
    if current == u16::MAX {
        return Ok(());
    }
    mapping.validate(current, false)?;
    if let Some(&new) = mapping.get(&current) {
        if new == u16::MAX {
            return Err(format!(
                "Object ID mapping for {current} uses reserved ID 65535"
            ));
        }
        bytes[offset..offset + 2].copy_from_slice(&new.to_le_bytes());
    }
    Ok(())
}

fn remap_attribute_value<F>(
    bytes: &mut [u8],
    command_offset: usize,
    original_target_id: u16,
    mapping: &IdRemapping<'_>,
    attribute_kind: &mut F,
    aid: Option<u8>,
    value_offset: usize,
) -> Result<(), String>
where
    F: FnMut(u16, Option<u8>) -> MacroAttributeValueKind,
{
    match attribute_kind(original_target_id, aid) {
        MacroAttributeValueKind::Numeric | MacroAttributeValueKind::ForeignReference => Ok(()),
        MacroAttributeValueKind::LocalObjectId16 {
            value_offset: reference_offset,
        } => {
            let offset = value_offset.checked_add(reference_offset).ok_or_else(|| {
                format!("attribute reference offset overflows at {command_offset}")
            })?;
            if offset + 2 > 8 {
                return Err(format!(
                    "attribute reference is outside command payload at offset {command_offset}"
                ));
            }
            remap_u16(bytes, command_offset + offset, mapping)
        }
        MacroAttributeValueKind::Unknown => Err(format!(
            "cannot determine whether value is an object reference for command 0x{:02X} at offset {command_offset}",
            bytes[command_offset]
        )),
    }
}

fn remap_graphics_context(
    bytes: &mut [u8],
    command_offset: usize,
    frame_length: usize,
    mapping: &IdRemapping<'_>,
) -> Result<(), String> {
    // Byte 2-3 identifies the Graphics Context object itself.
    remap_u16(bytes, command_offset + 1, mapping)?;
    let subcommand_offset = command_offset + 3;
    let subcommand = bytes[subcommand_offset];
    if matches!(subcommand, 4 | 5 | 6 | 18 | 19 | 20) {
        // These subcommands carry an Object ID in their first two parameters.
        if subcommand_offset + 3 > command_offset + frame_length {
            return Err(format!(
                "truncated Graphics Context reference at offset {command_offset}"
            ));
        }
        remap_u16(bytes, subcommand_offset + 1, mapping)?;
    }
    Ok(())
}

fn remap_execute_macro(
    bytes: &mut [u8],
    offset: usize,
    mapping: &IdRemapping<'_>,
    version: VtVersion,
) -> Result<(), String> {
    let old_id = u16::from(bytes[offset + 1]);
    mapping.validate(old_id, true)?;
    let Some(&new_id) = mapping.get(&old_id) else {
        return Ok(());
    };
    if new_id == u16::MAX {
        return Err(format!(
            "Execute Macro at offset {offset} maps to reserved ID 65535"
        ));
    }
    if new_id <= u8::MAX as u16 {
        bytes[offset + 1] = new_id as u8;
        return Ok(());
    }
    if version < VtVersion::Version5 {
        return Err(format!(
            "Execute Macro at offset {offset} cannot reference ID {new_id} on VT{}",
            u8::from(version)
        ));
    }
    bytes[offset] = 0xBC;
    bytes[offset + 1..offset + 3].copy_from_slice(&new_id.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod redirected_reference_tests {
    use super::*;

    #[test]
    fn redirected_attribute_payloads_require_existing_compatible_targets() {
        let types = HashMap::from([
            (1, ObjectType::OutputPolygon),
            (2, ObjectType::LineAttributes),
            (3, ObjectType::FontAttributes),
        ]);
        let valid = [0xAF, 1, 0, 3, 2, 0, 0xFF, 0xFF];
        assert!(validate_macro_reference_values(&valid, VtVersion::Version6, &types).is_ok());
        let mut wrong_type = valid;
        wrong_type[4] = 3;
        assert!(validate_macro_reference_values(&wrong_type, VtVersion::Version6, &types).is_err());
        for (object_type, aid) in [
            (ObjectType::OutputLinearBarGraph, 10),
            (ObjectType::OutputArchedBarGraph, 12),
        ] {
            let types = HashMap::from([
                (1, object_type),
                (2, ObjectType::NumberVariable),
                (3, ObjectType::FontAttributes),
            ]);
            let mut packet = [0xAF, 1, 0, aid, 2, 0, 0xFF, 0xFF];
            assert!(validate_macro_reference_values(&packet, VtVersion::Version6, &types).is_ok());
            packet[4] = 3;
            assert!(validate_macro_reference_values(&packet, VtVersion::Version6, &types).is_err());
        }
        let pointer_types = HashMap::from([(1, ObjectType::ObjectPointer)]);
        let missing = [0xA8, 1, 0, 0xFF, 9, 0, 0xFF, 0xFF];
        assert!(
            validate_macro_reference_values(&missing, VtVersion::Version6, &pointer_types).is_err()
        );
    }
}
