//! Compact automation responses. Rich domain DTOs and validation stay unchanged.
use serde_json::{json, Value};

fn summary(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        if let Some(ty) = map.remove("object_type") {
            map.insert("type".into(), ty);
        }
        map.retain(|key, v| key == "id" || !v.is_null());
    }
    value
}
pub fn object(value: Value) -> Value {
    let mut value = summary(value);
    let map = value.as_object_mut().unwrap();
    if let Some(Value::Array(children)) = map.remove("positioned_children") {
        map.insert(
            "children".into(),
            Value::Array(
                children
                    .into_iter()
                    .map(|c| {
                        let mut child = summary(c["child"].clone());
                        child["x"] = c["x"].clone();
                        child["y"] = c["y"].clone();
                        child
                    })
                    .collect(),
            ),
        );
    }
    if let Some(macros) = map.remove("macro_references") {
        map.insert("macros".into(), macros);
    }
    if let Some(Value::Object(refs)) = map.get_mut("references") {
        for reference in refs.values_mut() {
            *reference = summary(reference.take());
        }
    }
    if let Some(Value::Array(parents)) = map.get_mut("parents") {
        for parent in parents {
            *parent = summary(parent.take());
        }
    }
    if let Some(Value::Array(objects)) = map
        .get_mut("object_list")
        .and_then(|v| v.get_mut("objects"))
    {
        for object in objects {
            *object = summary(object.take());
        }
    }
    map.retain(|k, v| {
        !(v.is_null()
            || v.as_array().is_some_and(Vec::is_empty)
            || (k == "truncated" && v == false))
    });
    value
}
pub fn objects(values: Vec<super::tools::ObjectDetails>) -> Value {
    Value::Array(
        values
            .into_iter()
            .map(|v| object(serde_json::to_value(v).unwrap()))
            .collect(),
    )
}
pub fn context(value: super::snapshot::DesignSnapshot) -> Value {
    let mut value = serde_json::to_value(value).unwrap();
    value["selection"] = summary(value["selection"].take());
    value["project"]["active_mask"] = summary(value["project"]["active_mask"].take());
    for obj in value["objects"].as_array_mut().unwrap() {
        *obj = object(obj.take());
        // An overview needs live links. Explicit nulls remain available in get_objects.
        if let Some(refs) = obj.get_mut("references").and_then(Value::as_object_mut) {
            refs.retain(|_, reference| !reference["id"].is_null());
            if refs.is_empty() {
                obj.as_object_mut().unwrap().remove("references");
            }
        }
    }
    // Identity/type/name are already present in objects; placements need only IDs.
    for placement in value["layout"].as_array_mut().unwrap() {
        placement["id"] = placement["object"]["id"].clone();
        placement["parent"] = placement["parent"]["id"].clone();
        let fields = placement.as_object_mut().unwrap();
        fields.remove("object");
        // Keep the actual root path: shared objects can have multiple placements.
        placement
            .as_object_mut()
            .unwrap()
            .retain(|_, v| !v.is_null());
    }
    value.as_object_mut().unwrap().retain(|k, v| {
        !(v.is_null()
            || v.as_array().is_some_and(Vec::is_empty)
            || (matches!(
                k.as_str(),
                "truncated" | "depth_limited" | "properties_truncated"
            ) && v == false))
    });
    value
}
pub fn schema(schema: super::schema::ObjectSchemaSnapshot) -> Value {
    use super::schema::PropertySemanticSnapshot;
    let properties: Vec<_> = schema
        .properties
        .into_iter()
        .map(|p| {
            let mut v = json!({"name":p.name});
            if !p.writable {
                v["writable"] = json!(false);
            }
            match p.semantic {
                PropertySemanticSnapshot::ObjectReference {
                    allowed_object_types,
                } => {
                    v["semantic"] = json!("object_reference");
                    v["allowed_types"] = json!(allowed_object_types);
                    // Metadata does not distinguish required from nullable references.
                    // Do not invent nullability; transaction validation is authoritative.
                }
                semantic => {
                    v["type"] = json!(match &p.default_value {
                        Value::Bool(_) => "boolean",
                        Value::String(_) => "string",
                        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
                        Value::Number(_) => "number",
                        Value::Array(_) => "array",
                        Value::Object(_) => "object",
                        Value::Null => "null",
                    });
                    if !matches!(semantic, PropertySemanticSnapshot::Normal) {
                        v["semantic"] = json!(semantic);
                        if matches!(semantic, PropertySemanticSnapshot::Colour) {
                            v["value_forms"] = json!(["index", "name", "rgb"]);
                            v["examples"] = json!([{"name":"dark_blue"},{"rgb":"#06162E"},6]);
                        }
                    }
                    // Structured/enum defaults convey the accepted representation; retain them.
                    if p.default_value.is_object()
                        || p.default_value.is_array()
                        || matches!(
                            semantic,
                            PropertySemanticSnapshot::Enum | PropertySemanticSnapshot::Bitflags
                        )
                    {
                        v["example"] = p.default_value;
                    }
                    if let Some(range) = p.valid_range {
                        v["range"] = json!(range);
                    }
                }
            }
            v
        })
        .collect();
    let mut v = json!({"type":schema.object_type,"supported_for_target":schema.supported_for_target,"properties":properties});
    if !schema.allowed_positioned_children.is_empty() {
        v["allowed_positioned_children"] = json!(schema.allowed_positioned_children);
    }
    if let Some(list) = schema.object_list {
        v["object_list"] = json!(list);
    }
    if !schema.possible_events.is_empty() {
        v["possible_events"] = json!(schema.possible_events);
    }
    if schema.supports_object_labels {
        v["supports_object_labels"] = json!(true);
    }
    v
}
