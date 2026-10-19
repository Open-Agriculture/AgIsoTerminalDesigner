//! Readable authoring of VT strings while retaining their wire encoding.

use ag_iso_stack::object_pool::{object::VtString, object_attributes::FontType};
use serde_json::Value;
use std::collections::HashMap;

/// Encode readable text into an explicitly encoded VT string property value.
/// `utf16le` includes the required BOM; other encodings use standard `FontType`
/// names. Unsupported character sets and unrepresentable characters fail.
pub fn encode(text: &str, encoding: &str) -> Result<Value, String> {
    let bytes = if encoding == "utf16le" {
        VtString::Text(text.to_owned()).wire_bytes()
    } else {
        let font: FontType = serde_json::from_value(Value::String(encoding.to_owned()))
            .map_err(|_| format!("Unknown string encoding '{encoding}'; choose utf16le, Latin1, Latin9, Latin2, Latin4, Cyrillic, or Greek"))?;
        let mut characters = HashMap::new();
        for byte in 0..=u8::MAX {
            let decoded = VtString::Encoded {
                encoded_bytes: vec![byte],
            }
            .decode(font)
            .map_err(|message| format!("{encoding}: {message}"))?;
            let mut chars = decoded.chars();
            if let (Some(character), None) = (chars.next(), chars.next()) {
                // Undefined table entries decode to U+FFFD. That replacement
                // character is not a representable character in these sets.
                if character != char::REPLACEMENT_CHARACTER {
                    characters.entry(character).or_insert(byte);
                }
            }
        }
        text.chars()
            .map(|character| {
                characters.get(&character).copied().ok_or_else(|| {
                    format!(
                        "Character U+{:04X} is not representable in {encoding}",
                        character as u32
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    serde_json::to_value(VtString::Encoded {
        encoded_bytes: bytes,
    })
    .map_err(|error| error.to_string())
}

/// Return an exact readable `(encoding, text)` for an encoded VT string value,
/// or `None` for ordinary property values. Invalid UTF-16 is rejected rather
/// than normalized. Legacy byte strings use the bijective Latin1 mapping.
pub fn export_value(value: &Value) -> Result<Option<(String, String)>, String> {
    if value.get("encoded_bytes").is_none() {
        return Ok(None);
    }
    let VtString::Encoded {
        encoded_bytes: bytes,
    } = serde_json::from_value::<VtString>(value.clone())
        .map_err(|error| format!("Invalid encoded VT string: {error}"))?
    else {
        return Ok(None);
    };
    let (encoding, text) = if bytes.starts_with(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return Err("Encoded UTF-16LE string has an odd byte count".into());
        }
        let words = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
        let text = char::decode_utf16(words)
            .collect::<Result<String, _>>()
            .map_err(|_| "Encoded UTF-16LE string contains an unpaired surrogate".to_owned())?;
        ("utf16le".to_owned(), text)
    } else {
        (
            "Latin1".to_owned(),
            bytes.iter().map(|byte| char::from(*byte)).collect(),
        )
    };
    if encode(&text, &encoding)? != *value {
        return Err("Encoded VT string cannot be reproduced exactly as readable text".into());
    }
    Ok(Some((encoding, text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn readable_strings_round_trip_their_exact_encoding() {
        for (text, encoding) in [
            ("ÿ\0\n'\\", "Latin1"),
            ("€", "Latin9"),
            ("Ω", "Greek"),
            ("Привет", "Cyrillic"),
            ("Hello 🌾\n", "utf16le"),
        ] {
            let value = encode(text, encoding).unwrap();
            let (exported_encoding, exported_text) = export_value(&value).unwrap().unwrap();
            assert_eq!(encode(&exported_text, &exported_encoding).unwrap(), value);
        }
        let bytes: Vec<u8> = (0..=255).collect();
        let value = json!({"encoded_bytes": bytes});
        let (encoding, text) = export_value(&value).unwrap().unwrap();
        assert_eq!(encode(&text, &encoding).unwrap(), value);
        assert!(encode("🌾", "Latin1").is_err());
        assert!(encode("�", "Greek").is_err());
        assert_eq!(export_value(&json!("ordinary")).unwrap(), None);
    }
    #[test]
    fn malformed_utf16_is_rejected() {
        assert!(export_value(&json!({"encoded_bytes": [255, 254, 1]})).is_err());
        assert!(export_value(&json!({"encoded_bytes": [255, 254, 0, 216]})).is_err());
    }
}
