//! Colour resolution and descriptor-backed style inspection.
use crate::{
    effective_colour as colours,
    object_properties::{
        get_object_property, get_property_descriptors, PropertyKey, PropertySemantic,
    },
};
use ag_iso_stack::object_pool::{object::Object, ObjectPool, ObjectType};
use serde_json::{json, Value};

pub fn enrich(pool: &ObjectPool, ty: ObjectType, name: PropertyKey, value: Value) -> Value {
    if get_property_descriptors(ty)
        .iter()
        .any(|d| d.name == name && matches!(d.semantic, PropertySemantic::Colour))
    {
        if let Some(i) = value.as_u64().and_then(|i| u8::try_from(i).ok()) {
            return json!(colours::resolve_index(pool, i));
        }
    }
    value
}

/// Read and enrich an object's descriptor-backed properties for style inspection.
pub(crate) fn property_style(
    pool: &ObjectPool,
    object: &Object,
    max_value_length: Option<usize>,
) -> Value {
    let mut properties = json!({});
    for descriptor in get_property_descriptors(object.object_type()) {
        let Ok(value) = get_object_property(object, descriptor.name) else {
            continue;
        };
        if max_value_length.is_some_and(|limit| value.to_string().len() > limit) {
            continue;
        }
        properties[descriptor.name.as_str()] =
            enrich(pool, object.object_type(), descriptor.name, value);
    }
    properties
}
