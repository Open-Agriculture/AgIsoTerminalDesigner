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
