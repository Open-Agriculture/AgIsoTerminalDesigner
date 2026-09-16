use super::AiDesignState;
use crate::ai::snapshot::{
    active_mask, layout_instances, object_snapshot, selected_ids, DesignContextRequest,
    DesignSnapshot, ProjectSnapshot, SnapshotScope, MAX_SNAPSHOT_OBJECTS,
};
use ag_iso_stack::object_pool::ObjectId;
use ag_iso_stack::object_pool::ObjectType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiDomainTool {
    GetDesignContext,
    GetObjects,
    GetObjectSchema,
    ProposeTransaction,
    AnalyzeDesign,
}
#[derive(Debug, Clone, Serialize)]
pub enum AiToolError {
    ObjectNotFound(u16),
    RequestTooLarge,
    InvalidScope,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GetObjectsRequest {
    pub ids: Vec<u16>,
    pub include_properties: bool,
    pub include_references: bool,
    pub include_children: bool,
    pub include_parents: bool,
    pub include_macros: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetObjectSchemaRequest { pub object_type: String }

/// Provider-facing definitions for precisely the four Phase 3 domain tools.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        function("get_design_context", "Returns a semantic overview of the current VT design, including active mask, selection, object relationships and layout. Use this first when inspecting or modifying a design.", json!({"type":"object","properties":{"scope":{"type":"string","enum":["ActiveMask","Selection","EntirePool"]},"include_selection":{"type":"boolean"},"depth":{"type":"integer","minimum":0,"maximum":16},"max_objects":{"type":"integer","minimum":1,"maximum":200}},"additionalProperties":false})),
        function("get_objects", "Returns requested objects by ObjectId with their semantic properties and relationships.", json!({"type":"object","properties":{"ids":{"type":"array","items":{"type":"integer","minimum":1}},"include_properties":{"type":"boolean"},"include_references":{"type":"boolean"},"include_children":{"type":"boolean"},"include_parents":{"type":"boolean"},"include_macros":{"type":"boolean"}},"required":["ids"],"additionalProperties":false})),
        function("get_object_schema", "Returns writable properties, reference types, child relationships and supported events for one ISOBUS VT object type. Use before guessing valid fields.", json!({"type":"object","properties":{"object_type":{"type":"string","enum":ObjectType::values().into_iter().map(|v| format!("{v:?}")).collect::<Vec<_>>() }},"required":["object_type"],"additionalProperties":false})),
        function("propose_transaction", "Validates and stages one atomic design change without modifying the user's project. All edits must be submitted through this tool.", transaction_schema()),
        function("analyze_design", "Returns deterministic visual design-quality diagnostics. It is read-only and does not replace domain validation.", json!({"type":"object","properties":{"scope":{"type":"string","enum":["active_mask","proposal"]}},"additionalProperties":false})),
    ]
}
fn function(name: &str, description: &str, parameters: Value) -> Value { json!({"type":"function","name":name,"description":description,"parameters":parameters,"strict":false}) }
/// Mirrors the public serde transaction envelope. Operation payloads remain the source of truth
/// and are deserialized by `OperationTransaction`; skipped restoration variants are excluded.
fn transaction_schema() -> Value {
    let id = json!({"type":"integer","minimum":0,"maximum":65534});
    let target = json!({"anyOf":[object_schema(json!({"id":id}), &["id"]), object_schema(json!({"name":{"type":"string"}}), &["name"])]});
    let nullable_target = json!({"anyOf":[target,{"type":"null"}]});
    let coordinate = json!({"type":"integer","minimum":-32768,"maximum":32767});
    let string = json!({"type":"string"});
    let variants = vec![
        operation_schema("CreateObject", json!({"object_id":{"anyOf":[id,{"type":"null"}]},"object_type":{"type":"string","enum":ObjectType::values().into_iter().map(|v|format!("{v:?}")).collect::<Vec<_>>()},"name":string}), &["object_type","name"]),
        operation_schema("DeleteObject", json!({"object_id":id}), &["object_id"]),
        operation_schema("SetProperty", json!({"object":target,"property":string,"value":{"description":"Use the property's type and valid range from get_object_schema."}}), &["object","property","value"]),
        operation_schema("SetObjectReference", json!({"object":target,"property":string,"reference":nullable_target}), &["object","property","reference"]),
        operation_schema("ChangeObjectId", json!({"old_id":id,"new_id":id}), &["old_id","new_id"]),
        operation_schema("ReorderObjects", json!({"object_ids":{"type":"array","items":id}}), &["object_ids"]),
        operation_schema("AddChild", json!({"parent":target,"child":target,"x":coordinate,"y":coordinate}), &["parent","child","x","y"]),
        operation_schema("RemoveChild", json!({"parent_id":id,"child_id":id}), &["parent_id","child_id"]),
        operation_schema("SetChildPosition", json!({"parent_id":id,"child_id":id,"x":coordinate,"y":coordinate}), &["parent_id","child_id","x","y"]),
        operation_schema("SetChildren", json!({"parent":target,"children":{"type":"array","items":object_schema(json!({"child":target,"x":coordinate,"y":coordinate}), &["child","x","y"])}}), &["parent","children"]),
        operation_schema("SetObjectList", json!({"object":target,"objects":{"anyOf":[
            object_schema(json!({"kind":{"const":"Required","type":"string"},"objects":{"type":"array","items":target}}), &["kind","objects"]),
            object_schema(json!({"kind":{"const":"Nullable","type":"string"},"objects":{"type":"array","items":nullable_target}}), &["kind","objects"])
        ]}}), &["object","objects"]),
        operation_schema("SetMacroReferences", json!({"object":target,"macro_refs":{"type":"array","items":object_schema(json!({"event_id":{"type":"string","description":"Event name from get_object_schema possible_events."},"macro_object":target}), &["event_id","macro_object"])}}), &["object","macro_refs"]),
        operation_schema("SetObjectLabels", json!({"object_id":id,"labels":{"type":"array","items":object_schema(json!({"id":id,"font_type":{"type":"integer","minimum":0,"maximum":255},"string_variable_reference":{"anyOf":[id,{"type":"null"}]},"graphic_representation":{"anyOf":[id,{"type":"null"}]}}), &["id","font_type","string_variable_reference","graphic_representation"])}}), &["object_id","labels"]),
        operation_schema("RenameObject", json!({"object":target,"name":string}), &["object","name"]),
    ];
    object_schema(json!({"schema_version":{"type":"integer","const":2},"description":{"type":["string","null"]},"operations":{"type":"array","minItems":1,"items":{"anyOf":variants}}}), &["schema_version","operations"])
}
fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn operation_schema(name: &str, properties: Value, required: &[&str]) -> Value {
    object_schema(json!({"type":{"type":"string","const":name},"data":object_schema(properties, required)}), &["type","data"])
}
pub fn get_design_context(
    state: &AiDesignState,
    request: DesignContextRequest,
) -> Result<DesignSnapshot, AiToolError> {
    let root = match request.scope {
        SnapshotScope::ActiveMask => active_mask(&state.pool),
        SnapshotScope::Selection => state.selected,
        SnapshotScope::EntirePool => None,
    };
    let (mut ids, mut truncated) = selected_ids(
        &state.pool,
        root,
        request.scope,
        request.depth.min(crate::ai::snapshot::MAX_SNAPSHOT_DEPTH),
    );
    if request.include_selection {
        if let Some(selected) = state.selected {
            if !ids.contains(&selected) {
                ids.push(selected);
            }
        }
    }
    let limit = request.max_objects.min(MAX_SNAPSHOT_OBJECTS);
    if ids.len() > limit {
        ids.truncate(limit);
        truncated = true;
    }
    let active_id = active_mask(&state.pool);
    let active =
        active_id.and_then(|id| crate::ai::snapshot::summary(&state.pool, &state.object_info, id));
    let selection = state
        .selected
        .and_then(|id| crate::ai::snapshot::summary(&state.pool, &state.object_info, id));
    let objects = ids
        .into_iter()
        .filter_map(|id| object_snapshot(&state.pool, &state.object_info, id))
        .collect();
    let layout = layout_instances(
        &state.pool,
        &state.object_info,
        active_id,
        request.depth.min(crate::ai::snapshot::MAX_SNAPSHOT_DEPTH),
    );
    Ok(DesignSnapshot {
        schema_version: 3,
        project: ProjectSnapshot {
            revision: state.revision,
            vt_version: format!("{:?}", state.vt_version),
            mask_width: state.mask_size,
            mask_height: state.mask_size,
            object_count: state.pool.objects().len(),
            active_mask: active,
        },
        selection,
        objects,
        layout,
        truncated,
    })
}
pub fn get_objects(
    state: &AiDesignState,
    request: GetObjectsRequest,
) -> Result<Vec<crate::ai::snapshot::ObjectSnapshot>, AiToolError> {
    if request.ids.len() > MAX_SNAPSHOT_OBJECTS {
        return Err(AiToolError::RequestTooLarge);
    }
    let mut ids = request.ids;
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let id = ObjectId::new(id).map_err(|_| AiToolError::ObjectNotFound(id))?;
            object_snapshot(&state.pool, &state.object_info, id)
                .ok_or(AiToolError::ObjectNotFound(id.value()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(schema: &Value) -> Value {
        if let Some(value) = schema.get("const") { return value.clone(); }
        if let Some(values) = schema.get("enum") { return values[0].clone(); }
        if let Some(variants) = schema.get("anyOf") { return example(&variants[0]); }
        match schema["type"].as_str() {
            Some("object") => {
                let mut result = json!({});
                for field in schema["required"].as_array().unwrap() {
                    let field = field.as_str().unwrap();
                    result[field] = example(&schema["properties"][field]);
                }
                result
            }
            Some("array") => json!([example(&schema["items"])]),
            Some("integer") => json!(schema["minimum"].as_i64().unwrap_or(0)),
            Some("string") => json!("OnActivate"),
            _ => Value::Null,
        }
    }

    #[test]
    fn every_advertised_operation_payload_deserializes() {
        let schema = transaction_schema();
        let variants = schema["properties"]["operations"]["items"]["anyOf"].as_array().unwrap();
        assert_eq!(variants.len(), 14);
        for variant in variants {
            let payload = example(variant);
            let result = serde_json::from_value::<crate::operations::Operation>(payload.clone());
            assert!(result.is_ok(), "{payload}: {result:?}");
        }
    }
}
