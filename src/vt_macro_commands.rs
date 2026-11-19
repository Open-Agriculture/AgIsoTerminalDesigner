//! Validation for command streams stored in ISO 11783-6 Macro objects.

use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::ObjectType;

/// A local object operand found in a Macro command. `expected_types` is empty
/// when the command permits any object type; nullable operands are reported
/// with `object_id == None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroCommandTarget {
    pub command: u8,
    pub command_offset: usize,
    pub operand_offset: usize,
    pub object_id: Option<u16>,
    pub nullable: bool,
    pub expected_types: Vec<ObjectType>,
}

/// Decode local object operands from a validated Macro command stream.
/// This metadata is shared by whole-pool validation and local preview.
pub fn macro_command_targets(
    bytes: &[u8],
    version: VtVersion,
) -> Result<Vec<MacroCommandTarget>, String> {
    validate_macro_commands(bytes, version)?;
    let mut targets = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let command = bytes[offset];
        let target = |relative: usize, expected_types: Vec<ObjectType>| {
            let id = u16::from_le_bytes([bytes[offset + relative], bytes[offset + relative + 1]]);
            MacroCommandTarget {
                command,
                command_offset: offset,
                operand_offset: relative,
                object_id: (id != u16::MAX).then_some(id),
                nullable: false,
                expected_types,
            }
        };
        match command {
            0xA0 | 0xA1 | 0xA6 | 0xA7 | 0xAF | 0xB0 | 0xB5 | 0xBD => {
                targets.push(target(1, vec![]));
            }
            0xA2 => targets.push(target(
                1,
                vec![
                    ObjectType::InputBoolean,
                    ObjectType::InputString,
                    ObjectType::InputNumber,
                    ObjectType::InputList,
                ],
            )),
            0xA5 => {
                targets.push(target(1, vec![]));
                targets.push(target(3, vec![]));
            }
            0xA8 => targets.push(target(
                1,
                vec![
                    ObjectType::InputBoolean,
                    ObjectType::InputNumber,
                    ObjectType::InputList,
                    ObjectType::OutputNumber,
                    ObjectType::OutputList,
                    ObjectType::OutputMeter,
                    ObjectType::OutputLinearBarGraph,
                    ObjectType::OutputArchedBarGraph,
                    ObjectType::NumberVariable,
                    ObjectType::ObjectPointer,
                    ObjectType::ExternalObjectPointer,
                    ObjectType::Animation,
                    ObjectType::ScaledGraphic,
                ],
            )),
            0xA9 => targets.push(target(1, vec![ObjectType::OutputLine])),
            0xAA => targets.push(target(1, vec![ObjectType::FontAttributes])),
            0xAB => {
                targets.push(target(1, vec![ObjectType::LineAttributes]));
            }
            0xAC => {
                targets.push(target(1, vec![ObjectType::FillAttributes]));
                let mut pattern = target(5, vec![ObjectType::PictureGraphic]);
                pattern.nullable = true;
                targets.push(pattern);
            }
            0xAD => {
                targets.push(target(1, vec![ObjectType::WorkingSet]));
                let mut mask = target(3, vec![ObjectType::DataMask, ObjectType::AlarmMask]);
                mask.nullable = true;
                targets.push(mask);
            }
            0xAE => {
                let mut mask = target(2, vec![ObjectType::DataMask, ObjectType::AlarmMask]);
                mask.nullable = true;
                targets.push(mask);
                let mut soft_key_mask = target(4, vec![ObjectType::SoftKeyMask]);
                soft_key_mask.nullable = true;
                targets.push(soft_key_mask);
            }
            0xB1 => {
                targets.push(target(
                    1,
                    vec![
                        ObjectType::InputList,
                        ObjectType::OutputList,
                        ObjectType::Animation,
                        ObjectType::ExternalObjectDefinition,
                    ],
                ));
                let mut item = target(4, vec![]);
                item.nullable = true;
                targets.push(item);
            }
            0xB3 => targets.push(target(
                1,
                vec![
                    ObjectType::InputString,
                    ObjectType::OutputString,
                    ObjectType::StringVariable,
                ],
            )),
            0xB4 => {
                targets.push(target(1, vec![]));
                targets.push(target(3, vec![]));
            }
            0xB6 | 0xB7 => targets.push(target(1, vec![ObjectType::OutputPolygon])),
            0xB8 => {
                targets.push(target(1, vec![ObjectType::GraphicsContext]));
                let subcommand = bytes[offset + 3];
                if subcommand == 18 {
                    targets.push(target(4, vec![]));
                }
                let operand = match subcommand {
                    4 => Some((4, ObjectType::LineAttributes)),
                    5 => Some((4, ObjectType::FillAttributes)),
                    6 => Some((4, ObjectType::FontAttributes)),
                    19 | 20 => Some((4, ObjectType::PictureGraphic)),
                    _ => None,
                };
                if let Some((relative, kind)) = operand {
                    let mut target = target(relative, vec![kind]);
                    target.nullable = matches!(subcommand, 4 | 5 | 6);
                    targets.push(target);
                }
            }
            0xBC => targets.push(target(1, vec![ObjectType::Macro])),
            0xBE => {
                let id = u16::from(bytes[offset + 1]);
                targets.push(MacroCommandTarget {
                    command,
                    command_offset: offset,
                    operand_offset: 1,
                    object_id: Some(id),
                    nullable: false,
                    expected_types: vec![ObjectType::Macro],
                });
            }
            _ => {}
        }
        let spec = command_spec(command).expect("validated command has a spec");
        if command == 0xB3 {
            let count = u16::from_le_bytes([bytes[offset + 3], bytes[offset + 4]]) as usize;
            offset += (5 + count).max(8);
        } else if command == 0xB8 {
            offset += (3 + graphics_subcommand_length(bytes, offset + 3)?).max(8);
        } else {
            offset += spec.length;
        }
    }
    Ok(targets)
}

/// Validate the framing and version availability of a Macro command stream.
///
/// Command offsets in errors are zero-based offsets into `bytes`. Only
/// commands permitted in a Macro by Annex F are accepted; command payloads
/// are inspected for framing, not executed or semantically applied.
pub fn validate_macro_commands(bytes: &[u8], version: VtVersion) -> Result<(), String> {
    if version < VtVersion::Version2 {
        return Err(format!(
            "macro command stream at offset 0 requires VT2 or later (target is VT{})",
            u8::from(version)
        ));
    }

    let mut offset = 0;
    while offset < bytes.len() {
        let command_offset = offset;
        let command = bytes[offset];
        let Some(spec) = command_spec(command) else {
            return Err(format!(
                "unsupported or response-only macro command 0x{command:02X} at offset {command_offset}"
            ));
        };
        if version < spec.minimum_version {
            return Err(format!(
                "{} command 0x{command:02X} at offset {command_offset} requires VT{} or later (target is VT{})",
                spec.name,
                u8::from(spec.minimum_version),
                u8::from(version)
            ));
        }

        match command {
            0xB3 => {
                require(bytes, command_offset, 5, spec.name)?;
                let string_bytes =
                    u16::from_le_bytes([bytes[command_offset + 3], bytes[command_offset + 4]])
                        as usize;
                let command_length = 5usize.checked_add(string_bytes).ok_or_else(|| {
                    format!("{} length overflows at offset {command_offset}", spec.name)
                })?;
                let frame_length = command_length.max(8);
                require(bytes, command_offset, frame_length, spec.name)?;
                if version < VtVersion::Version4
                    && string_bytes >= 2
                    && bytes[command_offset + 5..command_offset + 7] == [0xFF, 0xFE]
                {
                    return Err(format!("Change String Value WideString at offset {command_offset} requires VT4 or later"));
                }
                validate_padding(
                    bytes,
                    command_offset + command_length,
                    command_offset + frame_length,
                )?;
                offset += frame_length;
            }
            0xB8 => {
                require(bytes, command_offset, 4, spec.name)?;
                let subcommand_offset = command_offset + 3;
                let subcommand_length = graphics_subcommand_length(bytes, subcommand_offset)?;
                let command_length = 3usize.checked_add(subcommand_length).ok_or_else(|| {
                    format!("{} length overflows at offset {command_offset}", spec.name)
                })?;
                let frame_length = command_length.max(8);
                require(bytes, command_offset, frame_length, spec.name)?;
                validate_padding(
                    bytes,
                    command_offset + command_length,
                    command_offset + frame_length,
                )?;
                offset += frame_length;
            }
            0xBE => {
                require(bytes, command_offset, spec.length, spec.name)?;
                validate_padding(bytes, command_offset + 2, command_offset + spec.length)?;
                offset += spec.length;
            }
            _ => {
                require(bytes, command_offset, spec.length, spec.name)?;
                offset += spec.length;
            }
        }
    }
    Ok(())
}

pub(crate) struct CommandSpec {
    pub(crate) name: &'static str,
    pub(crate) minimum_version: VtVersion,
    pub(crate) length: usize,
}

const VT2: VtVersion = VtVersion::Version2;
const VT4: VtVersion = VtVersion::Version4;
const VT5: VtVersion = VtVersion::Version5;
const VT6: VtVersion = VtVersion::Version6;

/// Annex F command bytes and data lengths. Responses and commands marked
/// “No” in the Annex F Allowed in a Macro field are intentionally absent.
pub(crate) fn command_spec(command: u8) -> Option<CommandSpec> {
    let (name, minimum_version, length) = match command {
        0x90 => ("Select Active Working Set", VT6, 9),
        0xA0 => ("Hide/Show Object", VT2, 8),
        0xA1 => ("Enable/Disable Object", VT2, 8),
        0xA2 => ("Select Input Object", VT2, 8),
        0xA3 => ("Control Audio Signal", VT2, 8),
        0xA4 => ("Set Audio Volume", VT2, 8),
        0xA5 => ("Change Child Location", VT2, 8),
        0xA6 => ("Change Size", VT2, 8),
        0xA7 => ("Change Background Colour", VT2, 8),
        0xA8 => ("Change Numeric Value", VT2, 8),
        0xA9 => ("Change End Point", VT2, 8),
        0xAA => ("Change Font Attributes", VT2, 8),
        0xAB => ("Change Line Attributes", VT2, 8),
        0xAC => ("Change Fill Attributes", VT2, 8),
        0xAD => ("Change Active Mask", VT2, 8),
        0xAE => ("Change Soft Key Mask", VT2, 8),
        0xAF => ("Change Attribute", VT2, 8),
        0xB0 => ("Change Priority", VT2, 8),
        0xB1 => ("Change List Item", VT2, 8),
        0xB3 => ("Change String Value", VT2, 0),
        0xB4 => ("Change Child Position", VT2, 9),
        0xB5 => ("Change Object Label", VT4, 8),
        0xB6 => ("Change Polygon Point", VT4, 8),
        0xB7 => ("Change Polygon Scale", VT4, 8),
        0xB8 => ("Graphics Context", VT4, 0),
        0xBA => ("Select Colour Map or Palette", VT4, 8),
        0xBC => ("Execute Extended Macro", VT5, 8),
        0xBD => ("Lock/Unlock Mask", VT4, 8),
        0xBE => ("Execute Macro", VT4, 8),
        _ => return None,
    };
    Some(CommandSpec {
        name,
        minimum_version,
        length,
    })
}

pub(crate) fn graphics_subcommand_length(bytes: &[u8], offset: usize) -> Result<usize, String> {
    require(bytes, offset, 1, "Graphics Context sub-command")?;
    let subcommand = bytes[offset];
    let length = match subcommand {
        0 | 1 | 7 | 8 | 9 | 10 | 11 | 14 | 15 | 17 => 5,
        2 | 3 => 2,
        4 | 5 | 6 | 18 | 19 | 20 => 3,
        12 => {
            require(bytes, offset, 2, "Graphics Context Draw Polygon")?;
            let point_count = bytes[offset + 1] as usize;
            2usize
                .checked_add(point_count.checked_mul(4).ok_or_else(|| {
                    format!("Graphics Context polygon length overflows at offset {offset}")
                })?)
                .ok_or_else(|| {
                    format!("Graphics Context polygon length overflows at offset {offset}")
                })?
        }
        13 => {
            require(bytes, offset, 3, "Graphics Context Draw Text")?;
            3usize
                .checked_add(bytes[offset + 2] as usize)
                .ok_or_else(|| {
                    format!("Graphics Context text length overflows at offset {offset}")
                })?
        }
        16 => 9,
        _ => {
            return Err(format!(
                "invalid Graphics Context sub-command {subcommand} at offset {offset}"
            ));
        }
    };
    require(bytes, offset, length, "Graphics Context sub-command")?;
    Ok(length)
}

fn validate_padding(bytes: &[u8], start: usize, end: usize) -> Result<(), String> {
    if let Some((relative, _)) = bytes[start..end]
        .iter()
        .enumerate()
        .find(|(_, byte)| **byte != 0xFF)
    {
        return Err(format!(
            "invalid command padding at offset {}: expected FF",
            start + relative
        ));
    }
    Ok(())
}

fn require(bytes: &[u8], offset: usize, length: usize, name: &str) -> Result<(), String> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("{name} length overflows at offset {offset}"))?;
    if end > bytes.len() {
        return Err(format!(
            "truncated {name} at offset {offset}: need {length} bytes, have {}",
            bytes.len().saturating_sub(offset)
        ));
    }
    Ok(())
}
