//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use ag_iso_stack::object_pool::{object::Object, ObjectPool, ObjectType};
use std::collections::HashMap;

/// ISOBUS maximum object count (16-bit IDs)
const MAX_OBJECTS: u32 = 65535;

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

    // Generate numbered name
    let mut counter = same_type_count + 1;
    loop {
        let mut candidate = format!("{}", get_object_type_name(object_type));
        if counter > 1 {
            candidate.push_str(&format!(" {}", counter));
        }
        if !existing_names.contains_key(&candidate) {
            return candidate;
        }
        counter += 1;
    }
}
