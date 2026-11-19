//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen
//!
//! Carry egui's persisted memory across egui upgrades. eframe drops the whole memory
//! (collapsed sections, window positions, ...) when one nested struct gained a field.

/// The storage key eframe saves egui's memory under.
pub const STORAGE_KEY: &str = "egui";

/// Decode memory that eframe could not restore by filling in fields that newer egui
/// versions added, using their defaults. Returns `None` when `stored` already decodes
/// (eframe restored it) or cannot be repaired.
pub fn migrate(stored: &str) -> Option<egui::Memory> {
    if ron::from_str::<egui::Memory>(stored).is_ok() {
        return None;
    }
    let defaults = ron::to_string(&egui::Memory::default()).ok()?;
    let mut patched = stored.to_owned();
    for _ in 0..32 {
        let error = match ron::from_str::<egui::Memory>(&patched) {
            Ok(memory) => return Some(memory),
            Err(error) => error,
        };
        let ron::error::Error::MissingStructField { field, .. } = &error.code else {
            return None;
        };
        // eframe stores memory on one line; the error points at the struct's closing paren
        if error.span.start.line != 1 {
            return None;
        }
        let at = patched
            .char_indices()
            .nth(error.span.start.col.checked_sub(1)?)?
            .0;
        if !patched[at..].starts_with(')') {
            return None;
        }
        let value = field_value(&defaults, field)?;
        patched.insert_str(at, &format!(",{field}:{value}"));
    }
    None
}

/// The serialized value of the first `field` in `source`, up to its closing delimiter.
fn field_value<'a>(source: &'a str, field: &str) -> Option<&'a str> {
    let needle = format!("{field}:");
    let start = source
        .match_indices(&needle)
        .find(|(index, _)| {
            !source[..*index].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '"')
        })?
        .0
        + needle.len();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in source[start..].char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' | ',' if depth == 0 => return Some(&source[start..start + offset]),
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_memory_missing_a_field_added_by_newer_egui() {
        let mut memory = egui::Memory::default();
        memory.options.input_options.line_scroll_speed = 12.0;
        memory.options.input_options.surrender_focus_on = egui::SurrenderFocusOn::Never;
        let current = ron::to_string(&memory).unwrap();
        // Memory saved by egui 0.32, before `surrender_focus_on` existed
        let old = current.replace(",surrender_focus_on:Never", "");
        assert_ne!(old, current);

        let migrated = migrate(&old).expect("old memory migrates");
        let options = migrated.options.input_options;
        assert_eq!(options.line_scroll_speed, 12.0);
        assert_eq!(
            options.surrender_focus_on,
            egui::SurrenderFocusOn::default()
        );
    }

    #[test]
    fn leaves_memory_that_eframe_restored_alone() {
        let current = ron::to_string(&egui::Memory::default()).unwrap();
        assert!(migrate(&current).is_none());
    }

    #[test]
    fn gives_up_on_memory_it_cannot_repair() {
        assert!(migrate("not ron").is_none());
        assert!(migrate("(options:(theme_preference:NoSuchTheme))").is_none());
    }

    #[test]
    fn field_values_end_at_their_delimiter() {
        let source = r#"(a:(b:1,c:"x,)"),d:[1,2],e:Some(3))"#;
        assert_eq!(field_value(source, "a"), Some(r#"(b:1,c:"x,)")"#));
        assert_eq!(field_value(source, "c"), Some(r#""x,)""#));
        assert_eq!(field_value(source, "d"), Some("[1,2]"));
        assert_eq!(field_value(source, "e"), Some("Some(3)"));
        assert_eq!(field_value(source, "missing"), None);
    }
}
