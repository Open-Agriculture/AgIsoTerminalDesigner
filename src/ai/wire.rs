//! Compact provider-facing projections. Rich domain DTOs and validation stay unchanged.
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
        // Context placements are direct parent-relative edges, not traversal paths.
        fields.remove("path");
        fields.remove("depth");
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
    let mut v = json!({"type":schema.object_type,"properties":properties});
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::*;
    #[test]
    fn compact_context_retains_structure_dimensions_and_references() {
        let state = crate::ai::efficiency_tests::pool(387, 2000);
        let rich = get_design_context(&state, DesignContextRequest::default()).unwrap();
        let before = serde_json::to_string(&rich).unwrap().len();
        let compact = context(rich);
        assert!(compact.to_string().len() < before);
        assert_eq!(compact["objects"][0]["type"], "DataMask");
        assert_eq!(
            compact["objects"][1]["references"]["font_attributes"]["id"],
            0
        );
        assert!(compact["objects"][1].get("properties").is_none());
        assert_eq!(compact["layout"][0]["parent"], 2);
        assert!(compact["layout"][0]["width"].is_number());
        assert_eq!(compact["next_offset"], 50);
        println!(
            "Context rich {before} -> compact {} bytes",
            compact.to_string().len()
        );
    }
    #[test]
    fn optional_object_sections_are_absent_and_property_names_unchanged() {
        let state = crate::ai::efficiency_tests::pool(2, 2);
        let compact = objects(
            get_objects(
                &state,
                GetObjectsRequest {
                    ids: vec![10],
                    include_properties: false,
                    include_references: false,
                    include_parents: false,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        for key in [
            "properties",
            "references",
            "children",
            "parents",
            "macros",
            "object_list",
            "next_detail_offset",
            "next_property_offset",
            "truncated",
        ] {
            assert!(compact[0].get(key).is_none(), "{key}");
        }
        let reference = object(
            json!({"id":10,"object_type":"OutputNumber","references":{"variable_reference":{"id":null,"name":null,"object_type":null}}}),
        );
        assert_eq!(
            reference["references"]["variable_reference"],
            json!({"id":null})
        );
        let custom = object(
            json!({"object_type":"Button","properties":{"object_type":"keep", "macro_references":[]}}),
        );
        assert_eq!(custom["properties"]["object_type"], "keep");
    }
    #[test]
    fn compact_schemas_preserve_all_property_rules() {
        let state = crate::ai::efficiency_tests::pool(0, 0);
        for ty in ag_iso_stack::object_pool::ObjectType::values() {
            let rich = get_object_schema(&state, ty).unwrap();
            let compact = schema(rich.clone());
            assert_eq!(
                compact["properties"].as_array().unwrap().len(),
                rich.properties.len()
            );
            for (p, c) in rich
                .properties
                .iter()
                .zip(compact["properties"].as_array().unwrap())
            {
                assert_eq!(c["name"], p.name);
                if let Some(range) = &p.valid_range {
                    assert_eq!(c["range"], json!(range));
                }
                if !p.writable {
                    assert_eq!(c["writable"], false);
                }
            }
        }
    }
}
