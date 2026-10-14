//! Validation for command streams stored in ISO 11783-6 Macro objects.

use ag_iso_stack::object_pool::vt_version::VtVersion;

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
            _ => {
                require(bytes, command_offset, spec.length, spec.name)?;
                offset += spec.length;
            }
        }
    }
    Ok(())
}

struct CommandSpec {
    name: &'static str,
    minimum_version: VtVersion,
    length: usize,
}

const VT2: VtVersion = VtVersion::Version2;
const VT4: VtVersion = VtVersion::Version4;
const VT5: VtVersion = VtVersion::Version5;
const VT6: VtVersion = VtVersion::Version6;

/// Annex F command bytes and data lengths. Responses and commands marked
/// “No” in the Annex F Allowed in a Macro field are intentionally absent.
fn command_spec(command: u8) -> Option<CommandSpec> {
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

fn graphics_subcommand_length(bytes: &[u8], offset: usize) -> Result<usize, String> {
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

#[cfg(test)]
mod tests {
    use super::validate_macro_commands;

    #[test]
    fn accepts_mixed_fixed_and_length_prefixed_macro_commands() {
        let mut commands = vec![0xA0; 8];
        commands.extend_from_slice(&[0xB3, 1, 0, 2, 0, b'o', b'k', 0xFF]);
        commands.extend_from_slice(&[0xB8, 1, 0, 2, 7, 0xFF, 0xFF, 0xFF]);
        commands.extend_from_slice(&[0xBE; 8]);

        assert!(validate_macro_commands(&commands, VtVersion::Version4).is_ok());
    }

    #[test]
    fn reports_truncated_variable_command_at_its_offset() {
        let commands = [0xA0, 0, 0, 0, 0, 0, 0, 0, 0xB3, 1, 0, 3, 0, b'o', b'k'];

        let error = validate_macro_commands(&commands, VtVersion::Version3).unwrap_err();
        assert!(error.contains("offset 8"));
        assert!(error.contains("truncated Change String Value"));
    }

    use ag_iso_stack::object_pool::vt_version::VtVersion;
}
