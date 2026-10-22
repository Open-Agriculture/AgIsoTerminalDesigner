//! Human readable labels for numeric object-property enums.

use crate::object_properties::PropertyKey;
use ag_iso_stack::object_pool::ObjectType;
use serde_json::Value;

/// Return the ISO 11783-6 label for an enum value while preserving its stored value.
pub(crate) fn enum_choice_label(
    object_type: ObjectType,
    property: PropertyKey,
    value: &Value,
) -> String {
    if let Some(number) = value.as_u64() {
        let label = match (object_type, property, number) {
            (ObjectType::OutputEllipse, PropertyKey::EllipseType, 0) => "Closed Ellipse",
            (ObjectType::OutputEllipse, PropertyKey::EllipseType, 1) => "Open Ellipse",
            (ObjectType::OutputEllipse, PropertyKey::EllipseType, 2) => "Closed Ellipse Segment",
            (ObjectType::OutputEllipse, PropertyKey::EllipseType, 3) => "Closed Ellipse Section",
            (ObjectType::OutputPolygon, PropertyKey::PolygonType, 0) => "Convex",
            (ObjectType::OutputPolygon, PropertyKey::PolygonType, 1) => "Non-convex",
            (ObjectType::OutputPolygon, PropertyKey::PolygonType, 2) => "Complex",
            (ObjectType::OutputPolygon, PropertyKey::PolygonType, 3) => "Open",
            (ObjectType::FillAttributes, PropertyKey::FillType, 0) => "No fill",
            (ObjectType::FillAttributes, PropertyKey::FillType, 1) => "Fill with line colour",
            (ObjectType::FillAttributes, PropertyKey::FillType, 2) => "Fill with specified colour",
            (ObjectType::FillAttributes, PropertyKey::FillType, 3) => "Fill with pattern",
            (ObjectType::AuxiliaryFunctionType1, PropertyKey::FunctionType, 0)
            | (ObjectType::AuxiliaryInputType1, PropertyKey::FunctionType, 0) => "Latching Boolean",
            (ObjectType::AuxiliaryFunctionType1, PropertyKey::FunctionType, 1)
            | (ObjectType::AuxiliaryInputType1, PropertyKey::FunctionType, 1) => "Analogue",
            (ObjectType::AuxiliaryFunctionType1, PropertyKey::FunctionType, 2)
            | (ObjectType::AuxiliaryInputType1, PropertyKey::FunctionType, 2) => {
                "Non-latching Boolean"
            }
            (ObjectType::AuxiliaryControlDesignatorType2, PropertyKey::PointerType, 0) => {
                "Auxiliary object"
            }
            (ObjectType::AuxiliaryControlDesignatorType2, PropertyKey::PointerType, 1) => {
                "Assigned auxiliary function or input"
            }
            (ObjectType::AuxiliaryControlDesignatorType2, PropertyKey::PointerType, 2) => {
                "Owner Working Set"
            }
            (ObjectType::AuxiliaryControlDesignatorType2, PropertyKey::PointerType, 3) => {
                "Working Set owning the assignment"
            }
            (ObjectType::GraphicData, PropertyKey::Format, 0) => "PNG",
            (ObjectType::ScaledGraphic, PropertyKey::ScaleType, packed) => {
                return scaled_graphic_label(packed);
            }
            _ => return format!("Unknown ({number})"),
        };
        return label.to_owned();
    }
    if let Some(number) = value.as_i64() {
        return format!("Unknown ({number})");
    }

    if let Some(name) = value.as_str() {
        return human_readable_variant_name(name);
    }
    if let Some(fields) = value.as_object() {
        if let Some(size) = fields.get("NonProportional").and_then(Value::as_str) {
            return format!("{} px", size.trim_start_matches("Px").replace('x', " × "));
        }
        if let Some(height) = fields.get("Proportional") {
            return format!("Proportional: {height} px");
        }
        if let Some((variant, payload)) = fields.iter().next() {
            return format!("{}: {payload}", human_readable_variant_name(variant));
        }
    }
    value.to_string()
}

fn scaled_graphic_label(packed: u64) -> String {
    if packed & 0x80 != 0 {
        return format!("Unknown ({packed})");
    }
    let scale = packed & 0x07;
    let horizontal = (packed >> 3) & 0x03;
    let vertical = (packed >> 5) & 0x03;
    let Some(scale) = [
        "Not scaled",
        "Scale to width, maintaining aspect ratio",
        "Scale to height, maintaining aspect ratio",
        "Scale to width and height",
        "Scale to fit, maintaining aspect ratio",
    ]
    .get(scale as usize) else {
        return format!("Unknown ({packed})");
    };
    let Some(horizontal) = ["left", "middle", "right"].get(horizontal as usize) else {
        return format!("Unknown ({packed})");
    };
    let Some(vertical) = ["top", "middle", "bottom"].get(vertical as usize) else {
        return format!("Unknown ({packed})");
    };
    format!("{scale}; position {horizontal}, {vertical}")
}

fn human_readable_variant_name(name: &str) -> String {
    let mut snake_case = String::new();
    let mut previous_lowercase = false;
    for character in name.chars() {
        if character == '_' || character == '-' {
            snake_case.push('_');
            previous_lowercase = false;
        } else {
            if character.is_uppercase() && previous_lowercase {
                snake_case.push('_');
            }
            snake_case.push(character);
            previous_lowercase = character.is_lowercase();
        }
    }
    crate::object_configuring::human_readable_property_name(&snake_case)
}

#[cfg(test)]
mod tests {
    use super::enum_choice_label;
    use crate::object_properties::PropertyKey;
    use ag_iso_stack::object_pool::ObjectType;
    use serde_json::json;

    #[test]
    fn labels_numeric_enums_and_unknown_values() {
        assert_eq!(
            enum_choice_label(
                ObjectType::OutputPolygon,
                PropertyKey::PolygonType,
                &json!(0)
            ),
            "Convex"
        );
        assert_eq!(
            enum_choice_label(
                ObjectType::OutputPolygon,
                PropertyKey::PolygonType,
                &json!(3)
            ),
            "Open"
        );
        assert_eq!(
            enum_choice_label(
                ObjectType::OutputEllipse,
                PropertyKey::EllipseType,
                &json!(2)
            ),
            "Closed Ellipse Segment"
        );
        assert_eq!(
            enum_choice_label(ObjectType::FillAttributes, PropertyKey::FillType, &json!(3)),
            "Fill with pattern"
        );
        assert_eq!(
            enum_choice_label(
                ObjectType::AuxiliaryInputType1,
                PropertyKey::FunctionType,
                &json!(2)
            ),
            "Non-latching Boolean"
        );
        assert_eq!(
            enum_choice_label(ObjectType::GraphicData, PropertyKey::Format, &json!(0)),
            "PNG"
        );
        assert_eq!(
            enum_choice_label(
                ObjectType::OutputPolygon,
                PropertyKey::PolygonType,
                &json!(9)
            ),
            "Unknown (9)"
        );
        assert_eq!(
            enum_choice_label(
                ObjectType::OutputPolygon,
                PropertyKey::PolygonType,
                &json!("NonProportional")
            ),
            "Non Proportional"
        );
    }

    #[test]
    fn labels_packed_scaled_graphic_fields() {
        assert_eq!(
            enum_choice_label(
                ObjectType::ScaledGraphic,
                PropertyKey::ScaleType,
                &json!(1 | (2 << 3) | (2 << 5))
            ),
            "Scale to width, maintaining aspect ratio; position right, bottom"
        );
        assert_eq!(
            enum_choice_label(ObjectType::ScaledGraphic, PropertyKey::ScaleType, &json!(7)),
            "Unknown (7)"
        );
    }
}
