//! Version-specific preparation of edited strings for object-pool export.

use std::collections::{HashMap, HashSet};

use ag_iso_stack::object_pool::{
    object::{Object, VtString},
    object_attributes::FontType,
    vt_version::VtVersion,
    NullableObjectId, ObjectId, ObjectPool,
};

/// Return a pool whose edited strings use the target VT's wire encoding.
///
/// Imported `Encoded` strings retain their bytes. For VT2 and VT3, edited
/// text is encoded using the Font Attributes object referenced by each
/// consumer. An unreferenced string variable uses ISO-8859-1. VT4 and later
/// use the stack's existing UTF-16LE handling and are returned unchanged.
///
/// This does not mutate `pool`. It rejects unrepresentable text, conflicting
/// encodings for a shared string variable, and active WideStrings on VT2/3.
pub fn prepare_pool_for_export(
    pool: &ObjectPool,
    version: VtVersion,
) -> Result<ObjectPool, String> {
    let (prepared, diagnostics) = prepare_pool(pool, version);
    if diagnostics.is_empty() {
        Ok(prepared)
    } else {
        Err(diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// Return all version-specific string export diagnostics without stopping at
/// the first invalid value. Each diagnostic identifies the object that owns
/// the invalid string or font reference.
pub(crate) fn string_export_diagnostics(
    pool: &ObjectPool,
    version: VtVersion,
) -> Vec<crate::pool_validation::ValidationDiagnostic> {
    prepare_pool(pool, version).1
}

fn prepare_pool(
    pool: &ObjectPool,
    version: VtVersion,
) -> (
    ObjectPool,
    Vec<crate::pool_validation::ValidationDiagnostic>,
) {
    let legacy = version < VtVersion::Version4;
    let mut diagnostics = Vec::new();
    let mut variable_fonts: HashMap<ObjectId, Vec<FontType>> = HashMap::new();
    let mut input_variables = HashSet::new();
    let mut variables_with_font_errors = HashSet::new();
    let mut encoders = HashMap::<u8, HashMap<char, u8>>::new();

    if legacy {
        for object in pool.objects() {
            let (variable, font_id) = match object {
                Object::InputString(value) => {
                    (value.variable_reference.0, Some(value.font_attributes))
                }
                Object::OutputString(value) => {
                    (value.variable_reference.0, Some(value.font_attributes))
                }
                _ => (None, None),
            };
            let (Some(variable), Some(font_id)) = (variable, font_id) else {
                continue;
            };
            if matches!(object, Object::InputString(_)) {
                input_variables.insert(variable);
            }
            if !variable_needs_encoding(pool, variable) {
                continue;
            }
            match font_type_for(pool, font_id) {
                Ok(font_type) => variable_fonts.entry(variable).or_default().push(font_type),
                Err(message) => {
                    variables_with_font_errors.insert(variable);
                    diagnostics.push(diagnostic(object.id(), "VT_STRING_FONT_REFERENCE", message));
                }
            }
        }
    } else {
        for object in pool.objects() {
            if let Object::InputString(value) = object {
                if let Some(variable) = value.variable_reference.0 {
                    input_variables.insert(variable);
                }
            }
        }
    }

    let mut result = pool.clone();
    for object in result.objects_mut() {
        match object {
            Object::InputString(value) => {
                if legacy && value.variable_reference.0.is_some() {
                    // The legacy wire record omits this fallback value.
                    continue;
                }
                if legacy {
                    prepare_inline_string(
                        &mut value.value,
                        value.font_attributes,
                        value.id,
                        pool,
                        &mut encoders,
                        &mut diagnostics,
                    );
                }
                if !legacy || !matches!(value.value, VtString::Text(_)) {
                    check_length_diagnostic(
                        &value.value,
                        value.id,
                        "Input String",
                        255,
                        &mut diagnostics,
                    );
                }
            }
            Object::OutputString(value) => {
                if legacy && value.variable_reference.0.is_some() {
                    continue;
                }
                if legacy {
                    prepare_inline_string(
                        &mut value.value,
                        value.font_attributes,
                        value.id,
                        pool,
                        &mut encoders,
                        &mut diagnostics,
                    );
                }
                if !legacy || !matches!(value.value, VtString::Text(_)) {
                    check_length_diagnostic(
                        &value.value,
                        value.id,
                        "Output String",
                        u16::MAX as usize,
                        &mut diagnostics,
                    );
                }
            }
            Object::StringVariable(value) => {
                let original_text = match &value.value {
                    VtString::Text(text) => Some(text.clone()),
                    VtString::Encoded { .. } => None,
                };
                let mut encodable = true;
                if legacy {
                    if let Some(text) = &original_text {
                        if text.is_empty() {
                            value.value = VtString::Encoded {
                                encoded_bytes: Vec::new(),
                            };
                        } else if variables_with_font_errors.contains(&value.id) {
                            encodable = false;
                        } else {
                            let fonts = variable_fonts
                                .get(&value.id)
                                .map(Vec::as_slice)
                                .unwrap_or(&[FontType::Latin1]);
                            match encode_for_consumers(text, fonts, &mut encoders) {
                                Ok(encoded_bytes) => {
                                    value.value = VtString::Encoded { encoded_bytes };
                                }
                                Err(message) => {
                                    encodable = false;
                                    diagnostics.push(diagnostic(
                                        value.id,
                                        if message.contains("conflicting") {
                                            "VT_STRING_SHARED_ENCODING"
                                        } else {
                                            "VT_STRING_ENCODING"
                                        },
                                        message,
                                    ));
                                }
                            }
                        }
                    } else if is_wide(&value.value) {
                        encodable = false;
                        diagnostics.push(diagnostic(
                            value.id,
                            "VT_STRING_WIDE_UNSUPPORTED",
                            format!(
                                "String Variable contains a WideString, which VT{} cannot export",
                                u8::from(version)
                            ),
                        ));
                    }
                }
                if encodable {
                    let max_len = if input_variables.contains(&value.id) {
                        255
                    } else {
                        u16::MAX as usize
                    };
                    check_length_diagnostic(
                        &value.value,
                        value.id,
                        "String Variable",
                        max_len,
                        &mut diagnostics,
                    );
                }
            }
            _ => {}
        }
    }
    (result, diagnostics)
}

fn variable_needs_encoding(pool: &ObjectPool, variable_id: ObjectId) -> bool {
    matches!(
        pool.object_by_id(variable_id),
        Some(Object::StringVariable(value))
            if matches!(&value.value, VtString::Text(text) if !text.is_empty())
    )
}

fn font_type_for(pool: &ObjectPool, font_id: NullableObjectId) -> Result<FontType, String> {
    match pool.object_by_nullable_id(font_id) {
        None if font_id.0.is_none() => Err("has a null Font Attributes reference".to_owned()),
        None => Err("references a missing Font Attributes object".to_owned()),
        Some(Object::FontAttributes(font)) => Ok(font.font_type),
        Some(_) => Err("references an object that is not Font Attributes".to_owned()),
    }
}

fn prepare_inline_string(
    value: &mut VtString,
    font_id: NullableObjectId,
    object_id: ObjectId,
    pool: &ObjectPool,
    encoders: &mut HashMap<u8, HashMap<char, u8>>,
    diagnostics: &mut Vec<crate::pool_validation::ValidationDiagnostic>,
) {
    if matches!(value, VtString::Text(text) if text.is_empty()) {
        *value = VtString::Encoded {
            encoded_bytes: Vec::new(),
        };
        return;
    }

    if let VtString::Text(text) = value {
        let font_type = match font_type_for(pool, font_id) {
            Ok(font_type) => font_type,
            Err(message) => {
                diagnostics.push(diagnostic(
                    object_id,
                    "VT_STRING_FONT_REFERENCE",
                    format!("string has an invalid Font Attributes reference: {message}"),
                ));
                return;
            }
        };
        match encode_for_consumers(text, &[font_type], encoders) {
            Ok(encoded_bytes) => *value = VtString::Encoded { encoded_bytes },
            Err(message) => {
                diagnostics.push(diagnostic(object_id, "VT_STRING_ENCODING", message));
                return;
            }
        }
    } else if is_wide(value) {
        diagnostics.push(diagnostic(
            object_id,
            "VT_STRING_WIDE_UNSUPPORTED",
            "string contains a WideString, which VT2/3 cannot export".to_owned(),
        ));
        return;
    }
}

fn check_length_diagnostic(
    value: &VtString,
    object_id: ObjectId,
    kind: &str,
    max_len: usize,
    diagnostics: &mut Vec<crate::pool_validation::ValidationDiagnostic>,
) {
    let len = value.wire_bytes().len();
    if len > max_len {
        diagnostics.push(diagnostic(
            object_id,
            "VT_STRING_LENGTH",
            format!("{kind} string is {len} encoded bytes; maximum is {max_len}"),
        ));
    }
}

fn diagnostic(
    object_id: ObjectId,
    code: &str,
    message: String,
) -> crate::pool_validation::ValidationDiagnostic {
    crate::pool_validation::ValidationDiagnostic {
        severity: crate::pool_validation::ValidationSeverity::Error,
        message,
        affected_objects: Some(vec![object_id.value()]),
        code: code.to_owned(),
    }
}

fn is_wide(value: &VtString) -> bool {
    matches!(value, VtString::Encoded { encoded_bytes } if encoded_bytes.starts_with(&[0xFF, 0xFE]))
}

fn encode_for_consumers(
    text: &str,
    fonts: &[FontType],
    encoders: &mut HashMap<u8, HashMap<char, u8>>,
) -> Result<Vec<u8>, String> {
    let fonts = if fonts.is_empty() {
        &[FontType::Latin1][..]
    } else {
        fonts
    };

    let mut encoded: Option<Vec<u8>> = None;
    for &font in fonts {
        let font_id = u8::from(font);
        let mapping = encoders
            .entry(font_id)
            .or_insert_with(|| build_encoder(font));
        let mut bytes = Vec::with_capacity(text.chars().count());
        for character in text.chars() {
            let Some(byte) = mapping.get(&character) else {
                return Err(format!(
                    "string contains {:?}, which is not representable in VT font type {}",
                    character, font_id
                ));
            };
            bytes.push(*byte);
        }

        if let Some(previous) = &encoded {
            if previous != &bytes {
                return Err(
                    "String Variable is shared by consumers that require conflicting 8-bit encodings"
                        .to_owned(),
                );
            }
        } else {
            encoded = Some(bytes);
        }
    }

    Ok(encoded.unwrap_or_default())
}

/// Invert the stack's own decoder so encoding and presentation use the same
/// ISO table, including the VT's treatment of undefined code points.
fn build_encoder(font: FontType) -> HashMap<char, u8> {
    let mut mapping = HashMap::with_capacity(256);
    for byte in 0..=u8::MAX {
        let decoded = VtString::Encoded {
            encoded_bytes: vec![byte],
        }
        .decode(font);
        if let Ok(character) = decoded {
            let mut chars = character.chars();
            if let (Some(character), None) = (chars.next(), chars.next()) {
                if character != char::REPLACEMENT_CHARACTER {
                    mapping.entry(character).or_insert(byte);
                }
            }
        }
    }
    mapping
}
