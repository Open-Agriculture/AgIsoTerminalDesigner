//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use ag_iso_stack::object_pool::ObjectType;
use std::collections::{HashMap, HashSet};

/// ISOBUS maximum object count (16-bit IDs)
const MAX_OBJECTS: u32 = 65535;

/// Normalize an authoring name for case- and separator-insensitive matching.
/// Stored names remain untouched; this representation is only for comparisons.
pub(crate) fn normalized_name(name: &str) -> String {
    let mut normalized = String::new();
    let mut separator_pending = false;

    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_whitespace() || matches!(character, '-' | '_') {
            separator_pending = !normalized.is_empty();
        } else {
            if separator_pending {
                normalized.push('-');
                separator_pending = false;
            }
            normalized.push(character);
        }
    }

    normalized
}

pub(crate) fn names_equal(left: &str, right: &str) -> bool {
    normalized_name(left) == normalized_name(right)
}

/// Get a user-friendly name for an object type
pub fn get_object_type_name(object_type: ObjectType) -> &'static str {
    match object_type {
        ObjectType::WorkingSet => "Working Set",
        ObjectType::DataMask => "Data Mask",
        ObjectType::AlarmMask => "Alarm Mask",
        ObjectType::Container => "Container",
        ObjectType::SoftKeyMask => "Soft Key Mask",
        ObjectType::Key => "Key",
        ObjectType::Button => "Button",
        ObjectType::InputBoolean => "Input Boolean",
        ObjectType::InputString => "Input String",
        ObjectType::InputNumber => "Input Number",
        ObjectType::InputList => "Input List",
        ObjectType::OutputString => "Output String",
        ObjectType::OutputNumber => "Output Number",
        ObjectType::OutputList => "Output List",
        ObjectType::OutputLine => "Line",
        ObjectType::OutputRectangle => "Rectangle",
        ObjectType::OutputEllipse => "Ellipse",
        ObjectType::OutputPolygon => "Polygon",
        ObjectType::OutputMeter => "Meter",
        ObjectType::OutputLinearBarGraph => "Linear Bar",
        ObjectType::OutputArchedBarGraph => "Arched Bar",
        ObjectType::PictureGraphic => "Picture",
        ObjectType::NumberVariable => "Number Variable",
        ObjectType::StringVariable => "String Variable",
        ObjectType::FontAttributes => "Font Attributes",
        ObjectType::LineAttributes => "Line Attributes",
        ObjectType::FillAttributes => "Fill Attributes",
        ObjectType::InputAttributes => "Input Attributes",
        ObjectType::ObjectPointer => "Object Pointer",
        ObjectType::Macro => "Macro",
        ObjectType::AuxiliaryFunctionType1 => "Aux Function v1",
        ObjectType::AuxiliaryInputType1 => "Aux Input v1",
        ObjectType::AuxiliaryFunctionType2 => "Aux Function v2",
        ObjectType::AuxiliaryInputType2 => "Aux Input v2",
        ObjectType::AuxiliaryControlDesignatorType2 => "Aux Control v2",
        ObjectType::ColourMap => "Colour Map",
        ObjectType::GraphicsContext => "Graphics Context",
        ObjectType::ColourPalette => "Colour Palette",
        ObjectType::GraphicData => "Graphic Data",
        ObjectType::WorkingSetSpecialControls => "Special Controls",
        ObjectType::ScaledGraphic => "Scaled Graphic",
        ObjectType::WindowMask => "Window Mask",
        ObjectType::KeyGroup => "Key Group",
        ObjectType::ExtendedInputAttributes => "Extended Input Attributes",
        ObjectType::ObjectLabelReferenceList => "Label Reference List",
        ObjectType::ExternalObjectDefinition => "External Object Definition",
        ObjectType::ExternalReferenceName => "External Reference Name",
        ObjectType::ExternalObjectPointer => "External Object Pointer",
        ObjectType::Animation => "Animation",
    }
}

/// Generates a smart default name for an object based on its type and context
pub fn generate_smart_default_name(
    object_type: ObjectType,
    existing_names: &HashMap<String, ObjectType>,
) -> String {
    // Count existing objects of the same type
    let same_type_count = existing_names
        .values()
        .filter(|&&t| t == object_type)
        .count();

    generate_smart_name_from_count(object_type, existing_names, same_type_count)
}

pub(crate) fn generate_smart_name_from_count(
    object_type: ObjectType,
    existing_names: &HashMap<String, ObjectType>,
    same_type_count: usize,
) -> String {
    let normalized_names: HashSet<_> = existing_names
        .keys()
        .map(|name| normalized_name(name))
        .collect();
    generate_smart_name_from_count_with_normalized_names(
        object_type,
        same_type_count,
        &normalized_names,
    )
}

pub(crate) fn generate_smart_name_from_count_with_normalized_names(
    object_type: ObjectType,
    same_type_count: usize,
    existing_normalized_names: &HashSet<String>,
) -> String {
    // Generate numbered name
    let mut counter = same_type_count + 1;
    loop {
        let mut candidate = format!("{}", get_object_type_name(object_type));
        if counter > 1 {
            candidate.push_str(&format!(" {}", counter));
        }
        if !existing_normalized_names.contains(&normalized_name(&candidate)) {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{names_equal, normalized_name};
    use std::collections::HashSet;

    #[test]
    fn name_matching_folds_case_and_separators() {
        assert!(names_equal("Start Button", "START_button"));
        assert!(names_equal("  start---button  ", "start button"));
        assert_eq!(normalized_name("  Ångström__UNIT  "), "ångström-unit");
    }

    #[test]
    fn generated_names_avoid_normalized_collisions() {
        let existing = [normalized_name("BUTTON_2")]
            .into_iter()
            .collect::<HashSet<_>>();
        assert_eq!(
            super::generate_smart_name_from_count_with_normalized_names(
                ag_iso_stack::object_pool::ObjectType::Button,
                1,
                &existing,
            ),
            "Button 3"
        );
    }
}
