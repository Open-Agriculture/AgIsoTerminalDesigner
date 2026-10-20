//! Discoverable command metadata derived from shared property and relationship rules.
use super::AiDesignState;
use crate::{
    default_object,
    object_properties::{
        get_object_property, get_property_descriptors, PropertyKey, PropertySemantic,
    },
};
use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectType};
use serde_json::{json, Value};

/// Inspect the target-version schema without maintaining parallel metadata types.
pub fn get_object_schema(state: &AiDesignState, object_type: ObjectType) -> Value {
    let object = default_object(object_type);
    let properties: Vec<_> = get_property_descriptors(object_type)
        .into_iter()
        .map(|mut descriptor| {
            descriptor.writable &= crate::vt_compatibility::supported_property(
                object_type,
                descriptor.name.as_str(),
                state.vt_version,
            );
            if let PropertySemantic::ObjectReference { allowed_types } = &mut descriptor.semantic {
                allowed_types.retain(|ty| {
                    crate::vt_compatibility::supported_object_type(*ty, state.vt_version)
                });
            }
            if object_type == ObjectType::InputBoolean && descriptor.name == PropertyKey::Value {
                descriptor.valid_range = Some((
                    json!(0),
                    json!(if state.vt_version < VtVersion::Version4 {
                        1
                    } else {
                        255
                    }),
                ));
            } else if matches!(descriptor.semantic, PropertySemantic::Colour) {
                descriptor.valid_range = Some((json!(0), json!(255)));
            }
            descriptor.enum_choices.retain(|choice| {
                crate::vt_compatibility::supported_enum_value(
                    object_type,
                    descriptor.name,
                    choice,
                    state.vt_version,
                )
            });
            let default = get_object_property(&object, descriptor.name).unwrap_or(Value::Null);
            let mut array_schemas =
                crate::object_properties::property_array_element_schemas(descriptor.name);
            let element = array_schemas.remove("");
            let mut property =
                serde_json::to_value(descriptor).expect("property metadata serializes");
            property["default_value"] = default;
            if let Some(element) = element {
                property["array_element"] = element;
            }
            if !array_schemas.is_empty() {
                property["nested_array_elements"] = json!(array_schemas);
            }
            property
        })
        .collect();
    let children =
        crate::allowed_object_relationships::get_allowed_child_refs(object_type, state.vt_version);
    let events = crate::possible_events::get_possible_events_for_version(&object, state.vt_version);
    let list=crate::operations::object_list(&object).map(|list|json!({"nullable":matches!(list,crate::operations::ObjectReferenceList::Nullable(_))}));
    json!({"object_type":object_type,"supported_for_target":crate::vt_compatibility::supported_object_type(object_type,state.vt_version),"properties":properties,"allowed_positioned_children":children,"object_list":list,"possible_events":events,"supports_object_labels":object_type==ObjectType::ObjectLabelReferenceList})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_schema_tracks_project_version_boundaries() {
        let mut state = crate::EditorProject::default().ai_design_state();
        for number in 2..=6 {
            state.vt_version = VtVersion::try_from(number).unwrap();
            let choices = |object_type, property: &str| {
                get_object_schema(&state, object_type)["properties"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["name"] == property)
                    .unwrap()["enum_choices"]
                    .clone()
            };
            assert_eq!(
                choices(ObjectType::OutputPolygon, "polygon_type"),
                json!([0, 1, 2, 3])
            );
            assert_eq!(
                choices(ObjectType::AuxiliaryFunctionType1, "function_type"),
                if number == 2 {
                    json!([0, 1, 2])
                } else {
                    json!([])
                }
            );
            assert_eq!(
                choices(ObjectType::AuxiliaryInputType1, "function_type"),
                if number == 2 {
                    json!([0, 1, 2])
                } else {
                    json!([])
                }
            );
            assert_eq!(
                choices(ObjectType::AuxiliaryControlDesignatorType2, "pointer_type"),
                if number >= 3 {
                    json!([0, 1, 2, 3])
                } else {
                    json!([])
                }
            );
            assert_eq!(
                choices(ObjectType::GraphicData, "format"),
                if number == 6 { json!([0]) } else { json!([]) }
            );
            assert_eq!(
                choices(ObjectType::ScaledGraphic, "scale_type")
                    .as_array()
                    .unwrap()
                    .len(),
                if number == 6 { 45 } else { 0 }
            );
            assert_eq!(
                choices(ObjectType::FontAttributes, "font_type")
                    .as_array()
                    .unwrap()
                    .contains(&json!("Greek")),
                number >= 4
            );
            assert_eq!(
                choices(ObjectType::FontAttributes, "font_size")
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|choice| choice.get("Proportional").is_some()),
                number >= 4
            );
        }
    }
}
