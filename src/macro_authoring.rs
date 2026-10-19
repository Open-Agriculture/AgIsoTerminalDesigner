//! Readable command forms for ISO 11783-6 Macro command streams.
//!
//! Only commands with a known, lossless typed layout are accepted. This makes
//! the text form useful for authoring while rejecting streams that would
//! otherwise need opaque bytes to reproduce.

use ag_iso_stack::object_pool::vt_version::VtVersion;

use crate::vt_macro_commands::{command_spec, graphics_subcommand_length, validate_macro_commands};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroInstruction {
    /// Canonical kebab-case command name.
    pub name: String,
    /// Readable positional argument tokens accepted by [`encode_macro_instruction`].
    pub arguments: Vec<String>,
}

const COMMANDS: &[(u8, &str)] = &[
    (0x90, "select-active-working-set"),
    (0xA0, "hide-show-object"),
    (0xA1, "enable-disable-object"),
    (0xA2, "select-input-object"),
    (0xA3, "control-audio-signal"),
    (0xA4, "set-audio-volume"),
    (0xA5, "change-child-location"),
    (0xA6, "change-size"),
    (0xA7, "change-background-colour"),
    (0xA8, "change-numeric-value"),
    (0xA9, "change-end-point"),
    (0xAA, "change-font-attributes"),
    (0xAB, "change-line-attributes"),
    (0xAC, "change-fill-attributes"),
    (0xAD, "change-active-mask"),
    (0xAE, "change-soft-key-mask"),
    (0xAF, "change-attribute"),
    (0xB0, "change-priority"),
    (0xB1, "change-list-item"),
    (0xB3, "change-string-value"),
    (0xB3, "change-string-value-wide"),
    (0xB4, "change-child-position"),
    (0xB5, "change-object-label"),
    (0xB6, "change-polygon-point"),
    (0xB7, "change-polygon-scale"),
    (0xB8, "graphics-context"),
    (0xBA, "select-colour-map-or-palette"),
    (0xBC, "execute-extended-macro"),
    (0xBD, "lock-unlock-mask"),
    (0xBE, "execute-macro"),
];

/// Encode one named Macro instruction into its exact command bytes.
///
/// `change-numeric-value` accepts `OBJECT VALUE`, where the
/// object is u16 and the value is u32. `hide-show-object` accepts `OBJECT
/// hide|show`, `enable-disable-object` accepts `OBJECT disable|enable`, and
/// `select-input-object` accepts `OBJECT set-focus|activate-for-input`.
/// `change-string-value` accepts `OBJECT TEXT` encoded in ISO-8859-1;
/// `change-string-value-wide` accepts `OBJECT TEXT` encoded as UTF-16LE with a
/// BOM and requires VT4 or later. Fixed
/// instructions use their semantic numeric fields; nullable object IDs are
/// written as `null`. `graphics-context` accepts named subcommands and typed
/// coordinates, dimensions, palette indexes, text, and object IDs.
pub fn encode_macro_instruction(
    name: &str,
    arguments: &[String],
    version: VtVersion,
) -> Result<Vec<u8>, String> {
    let command = COMMANDS
        .iter()
        .find_map(|(command, candidate)| (*candidate == name).then_some(*command))
        .ok_or_else(|| format!("Unsupported Macro instruction '{name}'"))?;
    let spec = command_spec(command).expect("command names and specs share the same table");
    if version < spec.minimum_version {
        return Err(format!(
            "{name} requires VT{} or later (target is VT{})",
            u8::from(spec.minimum_version),
            u8::from(version)
        ));
    }
    if name == "change-string-value-wide" && version < VtVersion::Version4 {
        return Err("change-string-value-wide requires VT4 or later".into());
    }

    let mut bytes = vec![command];
    match command {
        0x90 => {
            require_arguments(name, arguments, 1)?;
            let working_set_name = parse_u64(&arguments[0], "working set NAME")?;
            bytes.extend_from_slice(&working_set_name.to_le_bytes());
        }
        0xA0 | 0xA1 => {
            require_arguments(name, arguments, 2)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            let state = match (command, arguments[1].as_str()) {
                (0xA0, "hide") | (0xA1, "disable") => 0,
                (0xA0, "show") | (0xA1, "enable") => 1,
                _ => return Err(format!("Invalid state '{}' for {name}", arguments[1])),
            };
            bytes.push(state);
            bytes.extend_from_slice(&[0xFF; 4]);
        }
        0xA2 => {
            require_arguments(name, arguments, 2)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            bytes.push(match arguments[1].as_str() {
                "activate-for-input" if version >= VtVersion::Version4 => 0,
                "set-focus" => 0xFF,
                "activate-for-input" => {
                    return Err("activate-for-input requires VT4 or later".into())
                }
                value => return Err(format!("Invalid Select Input Object option '{value}'")),
            });
            bytes.extend_from_slice(&[0xFF; 4]);
        }
        0xA8 => {
            require_arguments(name, arguments, 2)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            bytes.push(0xFF);
            bytes.extend_from_slice(&parse_u32(&arguments[1], "numeric value")?.to_le_bytes());
        }
        0xA3 => {
            require_arguments(name, arguments, 4)?;
            bytes.push(parse_u8(&arguments[0], "activation count")?);
            push_u16(&mut bytes, parse_u16(&arguments[1], "frequency")?);
            push_u16(&mut bytes, parse_u16(&arguments[2], "on time")?);
            push_u16(&mut bytes, parse_u16(&arguments[3], "off time")?);
        }
        0xA4 => {
            require_arguments(name, arguments, 1)?;
            let volume = parse_u8(&arguments[0], "volume percentage")?;
            if volume > 100 {
                return Err("volume percentage must be between 0 and 100".into());
            }
            bytes.push(volume);
            bytes.extend_from_slice(&[0xFF; 6]);
        }
        0xA5 => {
            require_arguments(name, arguments, 4)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "parent object ID")?);
            push_u16(&mut bytes, parse_u16(&arguments[1], "child object ID")?);
            bytes.push(encode_child_location(&arguments[2], "X location")?);
            bytes.push(encode_child_location(&arguments[3], "Y location")?);
            bytes.push(0xFF);
        }
        0xA6 => {
            require_arguments(name, arguments, 3)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            push_u16(&mut bytes, parse_u16(&arguments[1], "width")?);
            push_u16(&mut bytes, parse_u16(&arguments[2], "height")?);
            bytes.push(0xFF);
        }
        0xA7 => {
            require_arguments(name, arguments, 2)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            bytes.push(parse_u8(&arguments[1], "colour index")?);
            bytes.extend_from_slice(&[0xFF; 4]);
        }
        0xA9 => {
            require_arguments(name, arguments, 4)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            push_u16(&mut bytes, parse_u16(&arguments[1], "width")?);
            push_u16(&mut bytes, parse_u16(&arguments[2], "height")?);
            bytes.push(parse_u8(&arguments[3], "line direction")?);
        }
        0xAA => {
            require_arguments(name, arguments, 5)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            for (value, label) in arguments[1..5]
                .iter()
                .zip(["colour", "size", "type", "style"])
            {
                bytes.push(parse_u8(value, label)?);
            }
            bytes.push(0xFF);
        }
        0xAB => {
            require_arguments(name, arguments, 4)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            bytes.push(parse_u8(&arguments[1], "colour index")?);
            bytes.push(parse_u8(&arguments[2], "line width")?);
            push_u16(&mut bytes, parse_u16(&arguments[3], "line art object ID")?);
            bytes.push(0xFF);
        }
        0xAC => {
            require_arguments(name, arguments, 4)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            bytes.push(parse_u8(&arguments[1], "fill type")?);
            bytes.push(parse_u8(&arguments[2], "colour index")?);
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[3], "pattern object ID")?,
            );
            bytes.push(0xFF);
        }
        0xAD => {
            require_arguments(name, arguments, 2)?;
            push_u16(
                &mut bytes,
                parse_u16(&arguments[0], "working set object ID")?,
            );
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[1], "active mask object ID")?,
            );
            bytes.extend_from_slice(&[0xFF; 3]);
        }
        0xAE => {
            require_arguments(name, arguments, 3)?;
            bytes.push(match arguments[0].as_str() {
                "data" => 1,
                "alarm" => 2,
                other => return Err(format!("mask type must be data or alarm, got '{other}'")),
            });
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[1], "mask object ID")?,
            );
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[2], "soft key mask object ID")?,
            );
            bytes.extend_from_slice(&[0xFF; 2]);
        }
        0xAF => {
            require_arguments(name, arguments, 3)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            bytes.push(parse_u8(&arguments[1], "attribute ID")?);
            bytes.extend_from_slice(&parse_u32(&arguments[2], "attribute value")?.to_le_bytes());
        }
        0xB0 => {
            require_arguments(name, arguments, 2)?;
            push_u16(
                &mut bytes,
                parse_u16(&arguments[0], "alarm mask object ID")?,
            );
            bytes.push(parse_u8(&arguments[1], "priority")?);
            bytes.extend_from_slice(&[0xFF; 4]);
        }
        0xB1 => {
            require_arguments(name, arguments, 3)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            bytes.push(parse_u8(&arguments[1], "list index")?);
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[2], "list item object ID")?,
            );
            bytes.extend_from_slice(&[0xFF; 2]);
        }
        0xB4 => {
            require_arguments(name, arguments, 4)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "parent object ID")?);
            push_u16(&mut bytes, parse_u16(&arguments[1], "child object ID")?);
            push_i16(&mut bytes, parse_i16(&arguments[2], "X position")?);
            push_i16(&mut bytes, parse_i16(&arguments[3], "Y position")?);
        }
        0xB5 => {
            require_arguments(name, arguments, 4)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "object ID")?);
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[1], "string variable object ID")?,
            );
            bytes.push(parse_u8(&arguments[2], "font type")?);
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[3], "graphic object ID")?,
            );
        }
        0xB6 => {
            require_arguments(name, arguments, 4)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "polygon object ID")?);
            bytes.push(parse_u8(&arguments[1], "point index")?);
            push_u16(&mut bytes, parse_u16(&arguments[2], "X position")?);
            push_u16(&mut bytes, parse_u16(&arguments[3], "Y position")?);
        }
        0xB7 => {
            require_arguments(name, arguments, 3)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "polygon object ID")?);
            push_u16(&mut bytes, parse_u16(&arguments[1], "width")?);
            push_u16(&mut bytes, parse_u16(&arguments[2], "height")?);
            bytes.push(0xFF);
        }
        0xBA => {
            require_arguments(name, arguments, 1)?;
            push_u16(
                &mut bytes,
                parse_nullable_u16(&arguments[0], "colour map or palette object ID")?,
            );
            bytes.extend_from_slice(&[0xFF; 5]);
        }
        0xBC => {
            require_arguments(name, arguments, 1)?;
            push_u16(&mut bytes, parse_u16(&arguments[0], "macro object ID")?);
            bytes.extend_from_slice(&[0xFF; 5]);
        }
        0xBD => {
            require_arguments(name, arguments, 3)?;
            bytes.push(match arguments[0].as_str() {
                "lock" => 1,
                "unlock" => 0,
                other => return Err(format!("lock state must be lock or unlock, got '{other}'")),
            });
            push_u16(&mut bytes, parse_u16(&arguments[1], "mask object ID")?);
            push_u16(
                &mut bytes,
                parse_u16(&arguments[2], "lock timeout in milliseconds")?,
            );
            bytes.extend_from_slice(&[0xFF; 2]);
        }
        0xBE => {
            require_arguments(name, arguments, 1)?;
            bytes.push(parse_u8(&arguments[0], "macro ID")?);
            bytes.extend_from_slice(&[0xFF; 6]);
        }
        0xB8 => encode_graphics_context(name, arguments, &mut bytes)?,
        0xB3 => {
            require_arguments(name, arguments, 2)?;
            let object = parse_u16(&arguments[0], "object ID")?;
            let text = if name == "change-string-value-wide" {
                let mut text = vec![0xFF, 0xFE];
                for unit in arguments[1].encode_utf16() {
                    text.extend_from_slice(&unit.to_le_bytes());
                }
                text
            } else {
                encode_latin1(&arguments[1])?
            };
            let length = u16::try_from(text.len())
                .map_err(|_| "Change String Value text exceeds 65535 bytes".to_owned())?;
            push_u16(&mut bytes, object);
            bytes.extend_from_slice(&length.to_le_bytes());
            bytes.extend_from_slice(&text);
            pad_variable_command(&mut bytes);
        }
        _ => {
            return Err(format!(
                "Macro instruction '{name}' has no supported typed argument representation"
            ));
        }
    }
    validate_macro_commands(&bytes, version)?;
    Ok(bytes)
}

/// Decode a complete Macro stream into named instructions.
///
/// Commands with an unsupported opcode, unsupported framing, invalid text
/// string, or unavailable target version return an explicit error. Commands
/// without a lossless typed representation are rejected.
pub fn decode_macro_commands(
    bytes: &[u8],
    version: VtVersion,
) -> Result<Vec<MacroInstruction>, String> {
    validate_macro_commands(bytes, version)?;
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let command = bytes[offset];
        let name = COMMANDS
            .iter()
            .find_map(|(candidate, name)| (*candidate == command).then_some(*name))
            .ok_or_else(|| {
                format!("Unsupported Macro instruction 0x{command:02X} at offset {offset}")
            })?;
        let spec = command_spec(command).expect("command names and specs share the same table");
        let length = match command {
            0xB3 => {
                let data_length =
                    u16::from_le_bytes([bytes[offset + 3], bytes[offset + 4]]) as usize;
                (5 + data_length).max(8)
            }
            0xB8 => {
                let subcommand_length = graphics_subcommand_length(bytes, offset + 3)?;
                (3 + subcommand_length).max(8)
            }
            _ => spec.length,
        };
        let instruction = match command {
            0x90 => MacroInstruction {
                name: name.to_owned(),
                arguments: vec![
                    u64::from_le_bytes(bytes[offset + 1..offset + 9].try_into().unwrap())
                        .to_string(),
                ],
            },
            0xA0 | 0xA1 => {
                require_reserved_ff(bytes, offset, 4, name)?;
                let state = match (command, bytes[offset + 3]) {
                    (0xA0, 0) => "hide",
                    (0xA0, 1) => "show",
                    (0xA1, 0) => "disable",
                    (0xA1, 1) => "enable",
                    (_, state) => return Err(format!("Invalid {name} state {state} at offset {offset}")),
                };
                MacroInstruction {
                    name: name.to_owned(),
                    arguments: vec![read_u16(&bytes[offset + 1..offset + 3]).to_string(), state.into()],
                }
            }
            0xA2 => {
                require_reserved_ff(bytes, offset, 4, name)?;
                let option = match bytes[offset + 3] {
                    0 => {
                        if version < VtVersion::Version4 {
                            return Err(format!("activate-for-input at offset {offset} requires VT4 or later"));
                        }
                        "activate-for-input"
                    }
                    0xFF => "set-focus",
                    value => return Err(format!("Invalid Select Input Object option {value} at offset {offset}")),
                };
                MacroInstruction {
                    name: name.to_owned(),
                    arguments: vec![read_u16(&bytes[offset + 1..offset + 3]).to_string(), option.into()],
                }
            }
            0xA8 => {
                if bytes[offset + 3] != 0xFF {
                    return Err(format!(
                        "Invalid Change Numeric Value reserved byte at offset {}",
                        offset + 3
                    ));
                }
                MacroInstruction {
                    name: name.to_owned(),
                    arguments: vec![
                        read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                        u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap())
                            .to_string(),
                    ],
                }
            }
            0xA3 => MacroInstruction {
                name: name.to_owned(),
                arguments: vec![
                    bytes[offset + 1].to_string(),
                    read_u16(&bytes[offset + 2..offset + 4]).to_string(),
                    read_u16(&bytes[offset + 4..offset + 6]).to_string(),
                    read_u16(&bytes[offset + 6..offset + 8]).to_string(),
                ],
            },
            0xA4 => {
                require_reserved_ff(bytes, offset, 2, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![bytes[offset + 1].to_string()] }
            }
            0xA5 => {
                require_reserved_ff(bytes, offset, 7, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                    read_u16(&bytes[offset + 3..offset + 5]).to_string(),
                    (i16::from(bytes[offset + 5]) - 127).to_string(),
                    (i16::from(bytes[offset + 6]) - 127).to_string(),
                ] }
            }
            0xA6 => {
                require_reserved_ff(bytes, offset, 7, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                    read_u16(&bytes[offset + 3..offset + 5]).to_string(),
                    read_u16(&bytes[offset + 5..offset + 7]).to_string(),
                ] }
            }
            0xA7 => {
                require_reserved_ff(bytes, offset, 4, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(), bytes[offset + 3].to_string()
                ] }
            }
            0xA9 => MacroInstruction { name: name.to_owned(), arguments: vec![
                read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                read_u16(&bytes[offset + 3..offset + 5]).to_string(),
                read_u16(&bytes[offset + 5..offset + 7]).to_string(),
                bytes[offset + 7].to_string(),
            ] },
            0xAA => {
                require_reserved_ff(bytes, offset, 7, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                    bytes[offset + 3].to_string(), bytes[offset + 4].to_string(),
                    bytes[offset + 5].to_string(), bytes[offset + 6].to_string(),
                ] }
            }
            0xAB => {
                require_reserved_ff(bytes, offset, 7, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(), bytes[offset + 3].to_string(),
                    bytes[offset + 4].to_string(), read_u16(&bytes[offset + 5..offset + 7]).to_string()
                ] }
            }
            0xAC => {
                require_reserved_ff(bytes, offset, 7, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(), bytes[offset + 3].to_string(),
                    bytes[offset + 4].to_string(), format_nullable_u16(read_u16(&bytes[offset + 5..offset + 7]))
                ] }
            }
            0xAD => {
                require_reserved_ff(bytes, offset, 5, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                    format_nullable_u16(read_u16(&bytes[offset + 3..offset + 5]))
                ] }
            }
            0xAE => {
                require_reserved_ff(bytes, offset, 6, name)?;
                let mask_type = match bytes[offset + 1] {
                    1 => "data",
                    2 => "alarm",
                    value => return Err(format!("Invalid Change Soft Key Mask type {value} at offset {offset}")),
                };
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    mask_type.into(), format_nullable_u16(read_u16(&bytes[offset + 2..offset + 4])),
                    format_nullable_u16(read_u16(&bytes[offset + 4..offset + 6]))
                ] }
            }
            0xAF => MacroInstruction { name: name.to_owned(), arguments: vec![
                read_u16(&bytes[offset + 1..offset + 3]).to_string(), bytes[offset + 3].to_string(),
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()).to_string()
            ] },
            0xB0 => {
                require_reserved_ff(bytes, offset, 4, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(), bytes[offset + 3].to_string()
                ] }
            }
            0xB1 => {
                require_reserved_ff(bytes, offset, 6, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(), bytes[offset + 3].to_string(),
                    format_nullable_u16(read_u16(&bytes[offset + 4..offset + 6]))
                ] }
            }
            0xB3 => {
                let data_length = u16::from_le_bytes([bytes[offset + 3], bytes[offset + 4]]) as usize;
                let text_bytes = &bytes[offset + 5..offset + 5 + data_length];
                let (instruction_name, text) = if text_bytes.starts_with(&[0xFF, 0xFE]) {
                    if version < VtVersion::Version4 {
                        return Err(format!("Wide Change String Value at offset {offset} requires VT4 or later"));
                    }
                    if (text_bytes.len() - 2) % 2 != 0 {
                        return Err(format!("Invalid UTF-16LE Change String Value length at offset {offset}"));
                    }
                    let units = text_bytes[2..]
                        .chunks_exact(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>();
                    let text = String::from_utf16(&units).map_err(|_| {
                        format!("Invalid UTF-16LE Change String Value text at offset {offset}")
                    })?;
                    ("change-string-value-wide", text)
                } else {
                    let text = text_bytes.iter().copied().map(char::from).collect::<String>();
                    ("change-string-value", text)
                };
                MacroInstruction {
                    name: instruction_name.to_owned(),
                    arguments: vec![
                        read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                        text,
                    ],
                }
            }
            0xB4 => MacroInstruction { name: name.to_owned(), arguments: vec![
                read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                read_u16(&bytes[offset + 3..offset + 5]).to_string(),
                i16::from_le_bytes(bytes[offset + 5..offset + 7].try_into().unwrap()).to_string(),
                i16::from_le_bytes(bytes[offset + 7..offset + 9].try_into().unwrap()).to_string(),
            ] },
            0xB5 => MacroInstruction { name: name.to_owned(), arguments: vec![
                read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                format_nullable_u16(read_u16(&bytes[offset + 3..offset + 5])), bytes[offset + 5].to_string(),
                format_nullable_u16(read_u16(&bytes[offset + 6..offset + 8]))
            ] },
            0xB6 => MacroInstruction { name: name.to_owned(), arguments: vec![
                read_u16(&bytes[offset + 1..offset + 3]).to_string(), bytes[offset + 3].to_string(),
                read_u16(&bytes[offset + 4..offset + 6]).to_string(),
                read_u16(&bytes[offset + 6..offset + 8]).to_string()
            ] },
            0xB7 => {
                require_reserved_ff(bytes, offset, 7, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    read_u16(&bytes[offset + 1..offset + 3]).to_string(),
                    read_u16(&bytes[offset + 3..offset + 5]).to_string(),
                    read_u16(&bytes[offset + 5..offset + 7]).to_string()
                ] }
            }
            0xB8 => decode_graphics_context(name, &bytes[offset..offset + length], offset)?,
            0xBA => {
                require_reserved_ff(bytes, offset, 3, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![format_nullable_u16(read_u16(&bytes[offset + 1..offset + 3]))] }
            }
            0xBC => {
                require_reserved_ff(bytes, offset, 3, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![read_u16(&bytes[offset + 1..offset + 3]).to_string()] }
            }
            0xBD => {
                require_reserved_ff(bytes, offset, 6, name)?;
                let state = match bytes[offset + 1] {
                    0 => "unlock",
                    1 => "lock",
                    value => return Err(format!("Invalid Lock/Unlock Mask state {value} at offset {offset}")),
                };
                MacroInstruction { name: name.to_owned(), arguments: vec![
                    state.into(),
                    read_u16(&bytes[offset + 2..offset + 4]).to_string(),
                    read_u16(&bytes[offset + 4..offset + 6]).to_string(),
                ] }
            }
            0xBE => {
                require_reserved_ff(bytes, offset, 2, name)?;
                MacroInstruction { name: name.to_owned(), arguments: vec![bytes[offset + 1].to_string()] }
            }
            _ => return Err(format!("Macro instruction '{name}' at offset {offset} has no supported typed argument representation")),
        };
        result.push(instruction);
        offset += length;
    }
    Ok(result)
}

fn require_arguments(name: &str, arguments: &[String], expected: usize) -> Result<(), String> {
    if arguments.len() != expected {
        return Err(format!(
            "{name} requires {expected} arguments, received {}",
            arguments.len()
        ));
    }
    Ok(())
}

fn encode_graphics_context(
    name: &str,
    arguments: &[String],
    bytes: &mut Vec<u8>,
) -> Result<(), String> {
    if arguments.len() < 2 {
        return Err("graphics-context requires CONTEXT and SUBCOMMAND".into());
    }
    push_u16(
        bytes,
        parse_u16(&arguments[0], "graphics context object ID")?,
    );
    let subcommand = match arguments[1].as_str() {
        "set-cursor" => 0,
        "move-cursor" => 1,
        "set-foreground-colour" => 2,
        "set-background-colour" => 3,
        "set-line-attributes" => 4,
        "set-fill-attributes" => 5,
        "set-font-attributes" => 6,
        "erase-rectangle" => 7,
        "draw-point" => 8,
        "draw-line" => 9,
        "draw-rectangle" => 10,
        "draw-closed-ellipse" => 11,
        "draw-polygon" => 12,
        "draw-text" => 13,
        "pan-viewport" => 14,
        "zoom-viewport" => 15,
        "pan-and-zoom-viewport" => 16,
        "change-viewport-size" => 17,
        "draw-vt-object" => 18,
        "copy-canvas-to-picture-graphic" => 19,
        "copy-viewport-to-picture-graphic" => 20,
        subcommand => {
            return Err(format!(
                "Unsupported graphics-context sub-command '{subcommand}'"
            ))
        }
    };
    bytes.push(subcommand);
    match subcommand {
        0 | 1 => {
            if arguments.len() != 4 {
                return Err(format!("{} requires X and Y values", arguments[1]));
            }
            for (index, argument) in arguments[2..].iter().enumerate() {
                let value = argument.parse::<i16>().map_err(|_| {
                    format!(
                        "'{}' is not a signed 16-bit {} coordinate",
                        argument,
                        if index == 0 { "X" } else { "Y" }
                    )
                })?;
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        2 | 3 => {
            if arguments.len() != 3 {
                return Err(format!("{} requires one colour index", arguments[1]));
            }
            bytes.push(parse_u8(&arguments[2], "colour index")?);
        }
        4 | 5 | 6 => {
            if arguments.len() != 3 {
                return Err(format!("{} requires one attribute object ID", arguments[1]));
            }
            push_u16(
                bytes,
                if arguments[2] == "null" {
                    u16::MAX
                } else {
                    parse_u16(&arguments[2], "attribute object ID")?
                },
            );
        }
        7 | 10 | 11 | 17 => {
            if arguments.len() != 4 {
                return Err(format!("{} requires WIDTH and HEIGHT", arguments[1]));
            }
            push_u16(bytes, parse_u16(&arguments[2], "width")?);
            push_u16(bytes, parse_u16(&arguments[3], "height")?);
        }
        8 | 9 | 14 => {
            if arguments.len() != 4 {
                return Err(format!("{} requires X and Y", arguments[1]));
            }
            push_i16(bytes, parse_i16(&arguments[2], "X coordinate")?);
            push_i16(bytes, parse_i16(&arguments[3], "Y coordinate")?);
        }
        12 => {
            if arguments.len() < 3 {
                return Err("draw-polygon requires POINT_COUNT followed by X Y pairs".into());
            }
            let count = parse_u8(&arguments[2], "polygon point count")?;
            if arguments.len() != 3 + usize::from(count) * 2 {
                return Err(format!(
                    "draw-polygon point count {count} requires {} coordinate arguments",
                    usize::from(count) * 2
                ));
            }
            bytes.push(count);
            for coordinate in &arguments[3..] {
                push_i16(bytes, parse_i16(coordinate, "polygon coordinate")?);
            }
        }
        13 => {
            if arguments.len() != 4 {
                return Err("draw-text requires transparent|opaque and TEXT".into());
            }
            bytes.push(match arguments[2].as_str() {
                "transparent" => 1,
                "opaque" => 0,
                value => {
                    return Err(format!(
                        "draw-text background must be transparent or opaque, got '{value}'"
                    ))
                }
            });
            let text = encode_latin1(&arguments[3])?;
            let length = u8::try_from(text.len())
                .map_err(|_| "draw-text text exceeds 255 bytes".to_owned())?;
            bytes.push(length);
            bytes.extend_from_slice(&text);
        }
        15 => {
            if arguments.len() != 3 {
                return Err("zoom-viewport requires a zoom value".into());
            }
            push_macro_float(bytes, &arguments[2], "zoom")?;
        }
        16 => {
            if arguments.len() != 5 {
                return Err("pan-and-zoom-viewport requires X, Y, and zoom".into());
            }
            push_i16(bytes, parse_i16(&arguments[2], "X coordinate")?);
            push_i16(bytes, parse_i16(&arguments[3], "Y coordinate")?);
            push_macro_float(bytes, &arguments[4], "zoom")?;
        }
        18 | 19 | 20 => {
            if arguments.len() != 3 {
                return Err(format!("{} requires one object ID", arguments[1]));
            }
            push_u16(bytes, parse_u16(&arguments[2], "object ID")?);
        }
        _ => unreachable!(),
    }
    pad_variable_command(bytes);
    debug_assert_eq!(name, "graphics-context");
    Ok(())
}

fn decode_graphics_context(
    instruction_name: &str,
    bytes: &[u8],
    offset: usize,
) -> Result<MacroInstruction, String> {
    let context = read_u16(&bytes[1..3]);
    let subcommand = bytes[3];
    let (_subcommand_name, arguments) = match subcommand {
        0 | 1 => {
            let command_name = if subcommand == 0 {
                "set-cursor"
            } else {
                "move-cursor"
            };
            let x = i16::from_le_bytes([bytes[4], bytes[5]]);
            let y = i16::from_le_bytes([bytes[6], bytes[7]]);
            (
                command_name,
                vec![
                    context.to_string(),
                    command_name.into(),
                    x.to_string(),
                    y.to_string(),
                ],
            )
        }
        2 | 3 => {
            let command_name = if subcommand == 2 {
                "set-foreground-colour"
            } else {
                "set-background-colour"
            };
            require_graphics_padding(bytes, 5, offset)?;
            (
                command_name,
                vec![
                    context.to_string(),
                    command_name.into(),
                    bytes[4].to_string(),
                ],
            )
        }
        4 | 5 | 6 => {
            let command_name = match subcommand {
                4 => "set-line-attributes",
                5 => "set-fill-attributes",
                _ => "set-font-attributes",
            };
            require_graphics_padding(bytes, 6, offset)?;
            let object = read_u16(&bytes[4..6]);
            (
                command_name,
                vec![
                    context.to_string(),
                    command_name.into(),
                    if object == u16::MAX {
                        "null".into()
                    } else {
                        object.to_string()
                    },
                ],
            )
        }
        7 | 10 | 11 | 17 => {
            let command_name = match subcommand {
                7 => "erase-rectangle",
                10 => "draw-rectangle",
                11 => "draw-closed-ellipse",
                _ => "change-viewport-size",
            };
            require_graphics_padding(bytes, 8, offset)?;
            (
                command_name,
                vec![
                    context.to_string(),
                    command_name.into(),
                    read_u16(&bytes[4..6]).to_string(),
                    read_u16(&bytes[6..8]).to_string(),
                ],
            )
        }
        8 | 9 | 14 => {
            let command_name = match subcommand {
                8 => "draw-point",
                9 => "draw-line",
                _ => "pan-viewport",
            };
            require_graphics_padding(bytes, 8, offset)?;
            (
                command_name,
                vec![
                    context.to_string(),
                    command_name.into(),
                    i16::from_le_bytes(bytes[4..6].try_into().unwrap()).to_string(),
                    i16::from_le_bytes(bytes[6..8].try_into().unwrap()).to_string(),
                ],
            )
        }
        12 => {
            let count = usize::from(bytes[4]);
            let mut values = vec![
                context.to_string(),
                "draw-polygon".into(),
                count.to_string(),
            ];
            for point in 0..count {
                let start = 5 + point * 4;
                values.push(
                    i16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()).to_string(),
                );
                values.push(
                    i16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap()).to_string(),
                );
            }
            let used = 5 + count * 4;
            require_graphics_padding(bytes, used, offset)?;
            ("draw-polygon", values)
        }
        13 => {
            let count = usize::from(bytes[5]);
            let text = bytes[6..6 + count]
                .iter()
                .copied()
                .map(char::from)
                .collect::<String>();
            let background = match bytes[4] {
                1 => "transparent",
                0 => "opaque",
                value => {
                    return Err(format!(
                        "Invalid Graphics Context Draw Text background {value} at offset {offset}"
                    ))
                }
            };
            require_graphics_padding(bytes, 6 + count, offset)?;
            (
                "draw-text",
                vec![
                    context.to_string(),
                    "draw-text".into(),
                    background.into(),
                    text,
                ],
            )
        }
        15 => {
            require_graphics_padding(bytes, 8, offset)?;
            (
                "zoom-viewport",
                vec![
                    context.to_string(),
                    "zoom-viewport".into(),
                    read_macro_float(&bytes[4..8])?,
                ],
            )
        }
        16 => {
            let x = i16::from_le_bytes(bytes[4..6].try_into().unwrap());
            let y = i16::from_le_bytes(bytes[6..8].try_into().unwrap());
            (
                "pan-and-zoom-viewport",
                vec![
                    context.to_string(),
                    "pan-and-zoom-viewport".into(),
                    x.to_string(),
                    y.to_string(),
                    read_macro_float(&bytes[8..12])?,
                ],
            )
        }
        18 | 19 | 20 => {
            let command_name = match subcommand {
                18 => "draw-vt-object",
                19 => "copy-canvas-to-picture-graphic",
                _ => "copy-viewport-to-picture-graphic",
            };
            require_graphics_padding(bytes, 6, offset)?;
            (
                command_name,
                vec![
                    context.to_string(),
                    command_name.into(),
                    read_u16(&bytes[4..6]).to_string(),
                ],
            )
        }
        _ => {
            return Err(format!(
                "Unsupported graphics-context sub-command {subcommand} at offset {offset}"
            ));
        }
    };
    Ok(MacroInstruction {
        name: instruction_name.to_owned(),
        arguments,
    })
}

fn require_graphics_padding(
    bytes: &[u8],
    used_length: usize,
    command_offset: usize,
) -> Result<(), String> {
    if let Some((relative, _)) = bytes[used_length..]
        .iter()
        .enumerate()
        .find(|(_, byte)| **byte != 0xFF)
    {
        return Err(format!(
            "Invalid graphics-context padding at offset {}: expected FF",
            command_offset + used_length + relative
        ));
    }
    Ok(())
}

fn parse_unsigned(value: &str, label: &str, max: u64) -> Result<u64, String> {
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse::<u64>()
    }
    .map_err(|_| format!("'{value}' is not a valid {label}"))?;
    if parsed > max {
        return Err(format!("{label} {parsed} exceeds {max}"));
    }
    Ok(parsed)
}

fn parse_u16(value: &str, label: &str) -> Result<u16, String> {
    Ok(parse_unsigned(value, label, u16::MAX as u64)? as u16)
}

fn parse_u8(value: &str, label: &str) -> Result<u8, String> {
    Ok(parse_unsigned(value, label, u8::MAX as u64)? as u8)
}

fn encode_child_location(value: &str, label: &str) -> Result<u8, String> {
    let value = value
        .parse::<i16>()
        .map_err(|_| format!("'{value}' is not a valid {label}"))?;
    if !(-127..=128).contains(&value) {
        return Err(format!("{label} must be between -127 and 128"));
    }
    Ok((value + 127) as u8)
}

fn parse_i16(value: &str, label: &str) -> Result<i16, String> {
    value
        .parse::<i16>()
        .map_err(|_| format!("'{value}' is not a signed 16-bit {label}"))
}

fn require_reserved_ff(
    bytes: &[u8],
    command_offset: usize,
    reserved_start: usize,
    name: &str,
) -> Result<(), String> {
    if let Some((relative, _)) = bytes[command_offset + reserved_start..command_offset + 8]
        .iter()
        .enumerate()
        .find(|(_, byte)| **byte != 0xFF)
    {
        return Err(format!(
            "Invalid {name} reserved byte at offset {}: expected FF",
            command_offset + reserved_start + relative
        ));
    }
    Ok(())
}

fn parse_u32(value: &str, label: &str) -> Result<u32, String> {
    Ok(parse_unsigned(value, label, u32::MAX as u64)? as u32)
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    parse_unsigned(value, label, u64::MAX)
}

fn push_macro_float(bytes: &mut Vec<u8>, value: &str, label: &str) -> Result<(), String> {
    let value = value
        .parse::<f32>()
        .map_err(|_| format!("'{value}' is not a valid {label} value"))?;
    if !value.is_finite() {
        return Err(format!("{label} value must be finite"));
    }
    bytes.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn read_macro_float(bytes: &[u8]) -> Result<String, String> {
    let value = f32::from_le_bytes(bytes.try_into().unwrap());
    if !value.is_finite() {
        return Err("Graphics Context zoom value is non-finite and has no readable form".into());
    }
    Ok(value.to_string())
}

fn encode_latin1(text: &str) -> Result<Vec<u8>, String> {
    text.chars()
        .map(|character| {
            u8::try_from(character as u32).map_err(|_| {
                format!(
                    "Change String Value character U+{:04X} is outside ISO-8859-1",
                    character as u32
                )
            })
        })
        .collect()
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i16(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn parse_nullable_u16(value: &str, label: &str) -> Result<u16, String> {
    if value == "null" {
        Ok(u16::MAX)
    } else {
        parse_u16(value, label)
    }
}

fn format_nullable_u16(value: u16) -> String {
    if value == u16::MAX {
        "null".into()
    } else {
        value.to_string()
    }
}

fn pad_variable_command(bytes: &mut Vec<u8>) {
    while bytes.len() < 8 {
        bytes.push(0xFF);
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_macro_commands, encode_macro_instruction};
    use ag_iso_stack::object_pool::vt_version::VtVersion;

    #[test]
    fn numeric_value_uses_readable_typed_arguments_and_round_trips() {
        let input = vec!["1234".into(), "305419896".into()];
        let bytes =
            encode_macro_instruction("change-numeric-value", &input, VtVersion::Version6).unwrap();
        assert_eq!(bytes, [0xA8, 0xD2, 0x04, 0xFF, 0x78, 0x56, 0x34, 0x12]);
        let decoded = decode_macro_commands(&bytes, VtVersion::Version6).unwrap();
        assert_eq!(decoded[0].name, "change-numeric-value");
        assert_eq!(decoded[0].arguments, input);
    }

    #[test]
    fn simple_state_instructions_use_named_values_and_canonical_padding() {
        for (instruction, state, expected_state) in [
            ("hide-show-object", "show", 1),
            ("enable-disable-object", "disable", 0),
        ] {
            let input = vec!["37".into(), state.into()];
            let bytes = encode_macro_instruction(instruction, &input, VtVersion::Version6).unwrap();
            assert_eq!(&bytes[1..3], &[37, 0]);
            assert_eq!(bytes[3], expected_state);
            assert_eq!(&bytes[4..], &[0xFF; 4]);
            assert_eq!(
                decode_macro_commands(&bytes, VtVersion::Version6).unwrap()[0].arguments,
                input
            );
        }
        let malformed = [0xA0, 37, 0, 1, 0, 0xFF, 0xFF, 0xFF];
        assert!(decode_macro_commands(&malformed, VtVersion::Version6)
            .unwrap_err()
            .contains("reserved byte"));
    }

    #[test]
    fn select_input_distinguishes_focus_from_data_input() {
        let input = vec!["71".into(), "set-focus".into()];
        let bytes =
            encode_macro_instruction("select-input-object", &input, VtVersion::Version2).unwrap();
        assert_eq!(bytes, [0xA2, 71, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(
            decode_macro_commands(&bytes, VtVersion::Version2).unwrap()[0].arguments,
            input
        );
        let activate = vec!["71".into(), "activate-for-input".into()];
        assert!(
            encode_macro_instruction("select-input-object", &activate, VtVersion::Version3)
                .unwrap_err()
                .contains("VT4")
        );
    }

    #[test]
    fn graphics_context_cursor_and_attributes_round_trip_as_named_values() {
        let cases: [(&str, Vec<String>, Vec<String>); 3] = [
            (
                "set-cursor",
                vec!["12".into(), "-17".into()],
                vec!["29".into(), "set-cursor".into(), "12".into(), "-17".into()],
            ),
            (
                "set-foreground-colour",
                vec!["3".into()],
                vec!["29".into(), "set-foreground-colour".into(), "3".into()],
            ),
            (
                "set-line-attributes",
                vec!["null".into()],
                vec!["29".into(), "set-line-attributes".into(), "null".into()],
            ),
        ];
        for (subcommand, values, expected) in cases {
            let mut input = vec!["29".into(), subcommand.into()];
            input.extend(values);
            let bytes =
                encode_macro_instruction("graphics-context", &input, VtVersion::Version4).unwrap();
            let decoded = decode_macro_commands(&bytes, VtVersion::Version4).unwrap();
            assert_eq!(decoded[0].name, "graphics-context");
            assert_eq!(decoded[0].arguments, expected);
        }
    }

    #[test]
    fn graphics_context_drawing_and_viewport_commands_round_trip() {
        let examples: &[&[&str]] = &[
            &["31", "erase-rectangle", "32", "24"],
            &["31", "draw-point", "-2", "12"],
            &["31", "draw-line", "4", "-7"],
            &["31", "draw-rectangle", "32", "24"],
            &["31", "draw-closed-ellipse", "32", "24"],
            &["31", "draw-polygon", "2", "-2", "12", "4", "-7"],
            &["31", "draw-text", "opaque", "a 'quote'\nline"],
            &["31", "pan-viewport", "-20", "100"],
            &["31", "zoom-viewport", "0.5"],
            &["31", "pan-and-zoom-viewport", "-20", "100", "1.25"],
            &["31", "change-viewport-size", "640", "480"],
            &["31", "draw-vt-object", "42"],
            &["31", "copy-canvas-to-picture-graphic", "42"],
            &["31", "copy-viewport-to-picture-graphic", "42"],
        ];
        for values in examples {
            let args = values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            let encoded =
                encode_macro_instruction("graphics-context", &args, VtVersion::Version6).unwrap();
            let decoded = decode_macro_commands(&encoded, VtVersion::Version6).unwrap();
            assert_eq!(decoded[0].arguments, args, "{args:?}");
            assert_eq!(
                encode_macro_instruction(
                    "graphics-context",
                    &decoded[0].arguments,
                    VtVersion::Version6
                )
                .unwrap(),
                encoded,
                "{args:?}"
            );
        }
    }

    #[test]
    fn graphics_context_bytes_match_annex_subcommand_fields() {
        let vectors: &[(&[&str], &[u8])] = &[
            (
                &["29", "set-cursor", "12", "-17"],
                &[0xB8, 29, 0, 0, 12, 0, 0xEF, 0xFF],
            ),
            (
                &["29", "move-cursor", "-2", "3"],
                &[0xB8, 29, 0, 1, 0xFE, 0xFF, 3, 0],
            ),
            (
                &["29", "set-foreground-colour", "4"],
                &[0xB8, 29, 0, 2, 4, 0xFF, 0xFF, 0xFF],
            ),
            (
                &["29", "set-background-colour", "5"],
                &[0xB8, 29, 0, 3, 5, 0xFF, 0xFF, 0xFF],
            ),
            (
                &["29", "set-line-attributes", "null"],
                &[0xB8, 29, 0, 4, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                &["29", "set-fill-attributes", "3"],
                &[0xB8, 29, 0, 5, 3, 0, 0xFF, 0xFF],
            ),
            (
                &["29", "set-font-attributes", "4"],
                &[0xB8, 29, 0, 6, 4, 0, 0xFF, 0xFF],
            ),
            (
                &["29", "erase-rectangle", "32", "24"],
                &[0xB8, 29, 0, 7, 32, 0, 24, 0],
            ),
            (
                &["29", "draw-point", "-2", "3"],
                &[0xB8, 29, 0, 8, 0xFE, 0xFF, 3, 0],
            ),
            (
                &["29", "draw-line", "4", "-7"],
                &[0xB8, 29, 0, 9, 4, 0, 0xF9, 0xFF],
            ),
            (
                &["29", "draw-rectangle", "32", "24"],
                &[0xB8, 29, 0, 10, 32, 0, 24, 0],
            ),
            (
                &["29", "draw-closed-ellipse", "32", "24"],
                &[0xB8, 29, 0, 11, 32, 0, 24, 0],
            ),
            (
                &["29", "draw-polygon", "1", "-2", "3"],
                &[0xB8, 29, 0, 12, 1, 0xFE, 0xFF, 3, 0],
            ),
            (
                &["29", "draw-text", "opaque", "A"],
                &[0xB8, 29, 0, 13, 0, 1, b'A', 0xFF],
            ),
            (
                &["29", "pan-viewport", "-20", "100"],
                &[0xB8, 29, 0, 14, 0xEC, 0xFF, 100, 0],
            ),
            (
                &["29", "zoom-viewport", "0.5"],
                &[0xB8, 29, 0, 15, 0, 0, 0, 0x3F],
            ),
            (
                &["29", "pan-and-zoom-viewport", "-20", "100", "0.5"],
                &[0xB8, 29, 0, 16, 0xEC, 0xFF, 100, 0, 0, 0, 0, 0x3F],
            ),
            (
                &["29", "change-viewport-size", "640", "480"],
                &[0xB8, 29, 0, 17, 0x80, 2, 0xE0, 1],
            ),
            (
                &["29", "draw-vt-object", "42"],
                &[0xB8, 29, 0, 18, 42, 0, 0xFF, 0xFF],
            ),
            (
                &["29", "copy-canvas-to-picture-graphic", "42"],
                &[0xB8, 29, 0, 19, 42, 0, 0xFF, 0xFF],
            ),
            (
                &["29", "copy-viewport-to-picture-graphic", "42"],
                &[0xB8, 29, 0, 20, 42, 0, 0xFF, 0xFF],
            ),
        ];
        for (values, expected) in vectors {
            let args = values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                encode_macro_instruction("graphics-context", &args, VtVersion::Version6).unwrap(),
                *expected,
                "{args:?}"
            );
        }
    }

    #[test]
    fn string_instructions_preserve_difficult_latin1_text() {
        let input = vec!["42".into(), "first line\nsecond 'quoted' line".into()];
        let bytes =
            encode_macro_instruction("change-string-value", &input, VtVersion::Version6).unwrap();
        let decoded = decode_macro_commands(&bytes, VtVersion::Version6).unwrap();
        assert_eq!(decoded[0].arguments, input);
        let high_byte =
            decode_macro_commands(&[0xB3, 1, 0, 1, 0, 0xFF, 0xFF, 0xFF], VtVersion::Version6)
                .unwrap();
        assert_eq!(high_byte[0].arguments[1], "ÿ");
    }

    #[test]
    fn wide_string_instruction_uses_bom_utf16_and_round_trips() {
        let args = vec!["42".into(), "A😃".into()];
        let bytes =
            encode_macro_instruction("change-string-value-wide", &args, VtVersion::Version4)
                .unwrap();
        assert_eq!(
            bytes,
            [0xB3, 42, 0, 8, 0, 0xFF, 0xFE, 0x41, 0, 0x3D, 0xD8, 0x03, 0xDE]
        );
        let decoded = decode_macro_commands(&bytes, VtVersion::Version4).unwrap();
        assert_eq!(decoded[0].name, "change-string-value-wide");
        assert_eq!(decoded[0].arguments, args);
        assert!(encode_macro_instruction(
            "change-string-value-wide",
            &["1".into(), "text".into()],
            VtVersion::Version3,
        )
        .unwrap_err()
        .contains("VT4"));
    }

    #[test]
    fn fixed_instruction_layouts_round_trip_with_typed_fields() {
        let examples: &[(&str, &[&str])] = &[
            ("select-active-working-set", &["578437695752307201"]),
            ("control-audio-signal", &["3", "440", "200", "300"]),
            ("set-audio-volume", &["75"]),
            ("change-child-location", &["1", "2", "-3", "4"]),
            ("change-size", &["1", "320", "240"]),
            ("change-background-colour", &["1", "12"]),
            ("change-end-point", &["1", "20", "30", "2"]),
            ("change-font-attributes", &["1", "2", "3", "4", "5"]),
            ("change-line-attributes", &["1", "2", "3", "4"]),
            ("change-fill-attributes", &["1", "2", "3", "null"]),
            ("change-active-mask", &["1", "null"]),
            ("change-soft-key-mask", &["data", "1", "2"]),
            ("change-attribute", &["1", "2", "305419896"]),
            ("change-priority", &["1", "2"]),
            ("change-list-item", &["1", "2", "null"]),
            ("change-child-position", &["1", "2", "-300", "400"]),
            ("change-object-label", &["1", "null", "2", "3"]),
            ("change-polygon-point", &["1", "2", "40000", "4"]),
            ("change-polygon-scale", &["1", "320", "240"]),
            ("select-colour-map-or-palette", &["null"]),
            ("execute-extended-macro", &["1"]),
            ("lock-unlock-mask", &["lock", "1", "500"]),
            ("execute-macro", &["1"]),
        ];
        for (name, values) in examples {
            let args = values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            let encoded = encode_macro_instruction(name, &args, VtVersion::Version6).unwrap();
            let decoded = decode_macro_commands(&encoded, VtVersion::Version6).unwrap();
            assert_eq!(decoded[0].name, *name, "{name}");
            assert_eq!(decoded[0].arguments, args, "{name}");
            assert_eq!(
                encode_macro_instruction(
                    &decoded[0].name,
                    &decoded[0].arguments,
                    VtVersion::Version6
                )
                .unwrap(),
                encoded,
                "{name}"
            );
        }
    }

    #[test]
    fn fixed_instruction_bytes_match_annex_field_layouts() {
        let vectors: &[(&str, &[&str], &[u8])] = &[
            (
                "select-active-working-set",
                &["578437695752307201"],
                &[0x90, 1, 2, 3, 4, 5, 6, 7, 8],
            ),
            (
                "hide-show-object",
                &["4660", "show"],
                &[0xA0, 0x34, 0x12, 1, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "enable-disable-object",
                &["1", "disable"],
                &[0xA1, 1, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "select-input-object",
                &["1", "set-focus"],
                &[0xA2, 1, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "control-audio-signal",
                &["3", "440", "200", "300"],
                &[0xA3, 3, 0xB8, 1, 0xC8, 0, 0x2C, 1],
            ),
            (
                "set-audio-volume",
                &["75"],
                &[0xA4, 75, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "change-child-location",
                &["1", "2", "0", "4"],
                &[0xA5, 1, 0, 2, 0, 0x7F, 0x83, 0xFF],
            ),
            (
                "change-size",
                &["1", "320", "240"],
                &[0xA6, 1, 0, 0x40, 1, 0xF0, 0, 0xFF],
            ),
            (
                "change-background-colour",
                &["1", "12"],
                &[0xA7, 1, 0, 12, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "change-numeric-value",
                &["1234", "305419896"],
                &[0xA8, 0xD2, 4, 0xFF, 0x78, 0x56, 0x34, 0x12],
            ),
            (
                "change-end-point",
                &["1", "20", "30", "2"],
                &[0xA9, 1, 0, 20, 0, 30, 0, 2],
            ),
            (
                "change-font-attributes",
                &["1", "2", "3", "4", "5"],
                &[0xAA, 1, 0, 2, 3, 4, 5, 0xFF],
            ),
            (
                "change-line-attributes",
                &["1", "2", "3", "4"],
                &[0xAB, 1, 0, 2, 3, 4, 0, 0xFF],
            ),
            (
                "change-fill-attributes",
                &["1", "2", "3", "null"],
                &[0xAC, 1, 0, 2, 3, 0xFF, 0xFF, 0xFF],
            ),
            (
                "change-active-mask",
                &["1", "null"],
                &[0xAD, 1, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "change-soft-key-mask",
                &["data", "1", "2"],
                &[0xAE, 1, 1, 0, 2, 0, 0xFF, 0xFF],
            ),
            (
                "change-attribute",
                &["1", "2", "305419896"],
                &[0xAF, 1, 0, 2, 0x78, 0x56, 0x34, 0x12],
            ),
            (
                "change-priority",
                &["1", "2"],
                &[0xB0, 1, 0, 2, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "change-list-item",
                &["1", "2", "null"],
                &[0xB1, 1, 0, 2, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "change-string-value",
                &["42", "A"],
                &[0xB3, 42, 0, 1, 0, b'A', 0xFF, 0xFF],
            ),
            (
                "change-child-position",
                &["1", "2", "-300", "400"],
                &[0xB4, 1, 0, 2, 0, 0xD4, 0xFE, 0x90, 1],
            ),
            (
                "change-object-label",
                &["1", "null", "2", "3"],
                &[0xB5, 1, 0, 0xFF, 0xFF, 2, 3, 0],
            ),
            (
                "change-polygon-point",
                &["1", "2", "40000", "4"],
                &[0xB6, 1, 0, 2, 0x40, 0x9C, 4, 0],
            ),
            (
                "change-polygon-scale",
                &["1", "320", "240"],
                &[0xB7, 1, 0, 0x40, 1, 0xF0, 0, 0xFF],
            ),
            (
                "select-colour-map-or-palette",
                &["null"],
                &[0xBA, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "execute-extended-macro",
                &["1"],
                &[0xBC, 1, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                "lock-unlock-mask",
                &["lock", "1", "500"],
                &[0xBD, 1, 1, 0, 0xF4, 1, 0xFF, 0xFF],
            ),
            (
                "execute-macro",
                &["1"],
                &[0xBE, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
        ];
        for (name, values, expected) in vectors {
            let args = values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                encode_macro_instruction(name, &args, VtVersion::Version6).unwrap(),
                *expected,
                "{name}"
            );
        }
    }

    #[test]
    fn reserved_padding_must_be_canonical() {
        let malformed = [0xA4, 75, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(decode_macro_commands(&malformed, VtVersion::Version6)
            .unwrap_err()
            .contains("reserved byte"));
    }

    #[test]
    fn change_polygon_point_rejects_signed_coordinate_values() {
        let error = encode_macro_instruction(
            "change-polygon-point",
            &["1".into(), "2".into(), "-1".into(), "4".into()],
            VtVersion::Version6,
        )
        .unwrap_err();
        assert!(error.contains("X position"));
    }

    #[test]
    fn rejects_unknown_instruction_and_wrong_target_version() {
        assert!(
            encode_macro_instruction("do-anything", &[], VtVersion::Version6)
                .unwrap_err()
                .contains("Unsupported")
        );
        assert!(encode_macro_instruction(
            "execute-macro",
            &vec!["0".into(); 7],
            VtVersion::Version2
        )
        .unwrap_err()
        .contains("VT4"));
    }
}
