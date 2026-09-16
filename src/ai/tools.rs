use super::AiDesignState;
use crate::ai::snapshot::{
    active_mask, object_snapshot_sections, selected_ids, DesignContextRequest, DesignSnapshot,
    SnapshotScope, MAX_SNAPSHOT_OBJECTS,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GetObjectsRequest {
    pub ids: Vec<u16>,
    pub include_properties: bool,
    pub include_references: bool,
    pub include_children: bool,
    pub include_parents: bool,
    pub include_macros: bool,
    pub parent_offset: usize,
    pub parent_limit: usize,
    pub include_nearby_style: bool,
    pub detail_offset: usize,
    pub detail_limit: usize,
    pub property_offset: usize,
    pub property_limit: usize,
}

pub const DEFAULT_PARENT_LIMIT: usize = 8;
pub const MAX_PARENT_LIMIT: usize = 20;
pub const MAX_NEARBY_STYLE: usize = 8;
pub const MAX_GET_OBJECTS: usize = 20;
pub const MAX_DETAIL_ITEMS: usize = 100;
pub const MAX_PROPERTY_ITEMS: usize = 4096;
impl Default for GetObjectsRequest {
    fn default() -> Self {
        Self {
            ids: Vec::new(),
            include_properties: true,
            include_references: true,
            include_children: false,
            include_parents: false,
            parent_offset: 0,
            parent_limit: DEFAULT_PARENT_LIMIT,
            include_nearby_style: false,
            include_macros: false,
            detail_offset: 0,
            detail_limit: MAX_DETAIL_ITEMS,
            property_offset: 0,
            property_limit: MAX_PROPERTY_ITEMS,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ObjectDetails {
    #[serde(flatten)]
    pub object: crate::ai::snapshot::ObjectSnapshot,
    pub truncated: bool,
    pub next_detail_offset: Option<usize>,
    pub next_property_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parents_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_parent_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearby_style: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectSchemaRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object_types: Vec<String>,
}
pub const MAX_SCHEMA_TYPES_PER_CALL: usize = 10;
impl GetObjectSchemaRequest {
    pub fn types(self) -> Result<Vec<String>, String> {
        let mut types = self.object_types;
        if let Some(ty) = self.object_type {
            types.push(ty);
        }
        if types.is_empty() || types.len() > MAX_SCHEMA_TYPES_PER_CALL {
            return Err(format!(
                "Request 1–{MAX_SCHEMA_TYPES_PER_CALL} object types"
            ));
        }
        types.sort();
        types.dedup();
        for name in &types {
            if !ObjectType::values()
                .iter()
                .any(|v| format!("{v:?}") == *name)
            {
                return Err(format!("unknown object type: {name}"));
            }
        }
        Ok(types)
    }
}

/// Provider-facing definitions for precisely the four Phase 3 domain tools.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        function("get_design_context", "Forward layout overview, depth 1 by default: direct children with geometry first. Drill into root_id for relevant subtrees; page with layout_offset only when needed. Objects contain only summaries first needed on that layout page. EntirePool pages objects with offset. Coordinates are parent-relative; path identifies occurrences. Properties opt-in.", json!({"type":"object","properties":{"scope":{"type":"string","enum":["ActiveMask","Selection","EntirePool"]},"include_selection":{"type":"boolean"},"offset":{"type":"integer","minimum":0},"layout_offset":{"type":"integer","minimum":0},"root_id":{"type":"integer","minimum":0,"maximum":65534},"include_properties":{"type":"boolean"},"include_references":{"type":"boolean"},"depth":{"type":"integer","minimum":0,"maximum":8},"max_objects":{"type":"integer","minimum":1,"maximum":50}},"additionalProperties":false})),
        function("get_objects", "Batch relevant IDs (up to 20), requesting only required sections. Parents are reverse references and may be large: leave include_parents false unless reverse usage is needed; shared variables/styles may have many parents. Parents page independently with parent_offset/parent_limit (default 8, max 20). include_nearby_style returns up to 8 descendant style examples with resolved styles; page with detail_offset. Children/macros page with detail_offset/detail_limit. Long properties page with property_offset/property_limit.", json!({"type":"object","properties":{"ids":{"type":"array","maxItems":20,"items":{"type":"integer","minimum":0,"maximum":65534}},"include_properties":{"type":"boolean"},"include_references":{"type":"boolean"},"include_children":{"type":"boolean"},"include_parents":{"type":"boolean","default":false},"parent_offset":{"type":"integer","minimum":0},"parent_limit":{"type":"integer","minimum":1,"maximum":20},"include_nearby_style":{"type":"boolean"},"include_macros":{"type":"boolean"},"detail_offset":{"type":"integer","minimum":0},"detail_limit":{"type":"integer","minimum":1,"maximum":100},"property_offset":{"type":"integer","minimum":0},"property_limit":{"type":"integer","minimum":1,"maximum":4096}},"required":["ids"],"additionalProperties":false})),
        function("get_object_schema", "Writable properties and relationship rules. Batch all needed types (up to 10) in object_types; object_type accepts a single type.", json!({"type":"object","properties":{"object_type":{"type":"string"},"object_types":{"type":"array","minItems":1,"maxItems":MAX_SCHEMA_TYPES_PER_CALL,"items":{"type":"string"}}},"anyOf":[{"required":["object_type"]},{"required":["object_types"]}],"additionalProperties":false})),
        function("propose_transaction", "Validates and stages one atomic design change without modifying the user's project. All edits must be submitted through this tool.", compact_transaction_schema()),
        function("analyze_design", "Returns deterministic visual design-quality diagnostics. It is read-only and does not replace domain validation.", json!({"type":"object","properties":{"scope":{"type":"string","enum":["active_mask","proposal"]}},"additionalProperties":false})),
    ]
}
pub fn phase_tool_definitions(phase: super::task_state::AiTaskPhase) -> Vec<Value> {
    use super::task_state::AiTaskPhase;
    let mut tools: Vec<_> = tool_definitions()
        .into_iter()
        .filter(|t| match t["name"].as_str().unwrap_or("") {
            "propose_transaction" => phase != AiTaskPhase::Inspect,
            "get_design_context" => phase == AiTaskPhase::Inspect,
            "analyze_design" => phase == AiTaskPhase::Review,
            _ => true,
        })
        .collect();
    if matches!(phase, AiTaskPhase::Inspect | AiTaskPhase::Repair) {
        tools.push(function("inspect_design_region", "Focused nearby style examples, path, local geometry and absolute geometry relative to root. Forward display edges only; existing style references are resolved. Ranked by distance to region center then path.", json!({"type":"object","properties":{"root_id":{"type":"integer","minimum":0,"maximum":65534},"region":{"type":"string","enum":["top","bottom","left","right","center","top_left","top_right","bottom_left","bottom_right","all"]},"types":{"type":"array","items":{"type":"string"}},"purpose":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":30}},"required":["root_id"],"additionalProperties":false})));
    }
    if phase == AiTaskPhase::Inspect {
        tools.push(function("finish_inspection", "Local facts are sufficient; expose transaction proposal tools next round. No mutation.", json!({"type":"object","properties":{"summary":{"type":"string","maxLength":512}},"additionalProperties":false})));
    }
    tools.push(function("request_stronger_model", "Request stronger reasoning for a concrete blocker. The harness chooses the model; no mutation.", json!({"type":"object","properties":{"reason":{"type":"string","minLength":1,"maxLength":512}},"required":["reason"],"additionalProperties":false})));
    tools
}
fn function(name: &str, description: &str, parameters: Value) -> Value {
    json!({"type":"function","name":name,"description":description,"parameters":parameters,"strict":false})
}
/// Mirrors the public serde transaction envelope. Operation payloads remain the source of truth
/// and are deserialized by `OperationTransaction`; skipped restoration variants are excluded.
fn transaction_schema() -> Value {
    let id = json!({"type":"integer","minimum":0,"maximum":65534});
    let target = json!({"anyOf":[object_schema(json!({"id":id}), &["id"]), object_schema(json!({"name":{"type":"string"}}), &["name"])]});
    let nullable_target = json!({"anyOf":[target,{"type":"null"}]});
    let coordinate = json!({"type":"integer","minimum":-32768,"maximum":32767});
    let string = json!({"type":"string"});
    let variants = vec![
        operation_schema(
            "CreateObject",
            json!({"object_id":{"anyOf":[id,{"type":"null"}]},"object_type":{"type":"string","enum":ObjectType::values().into_iter().map(|v|format!("{v:?}")).collect::<Vec<_>>()},"name":string}),
            &["object_type", "name"],
        ),
        operation_schema("DeleteObject", json!({"object_id":id}), &["object_id"]),
        operation_schema(
            "SetProperty",
            json!({"object":target,"property":string,"value":{"description":"Use the property's type and valid range from get_object_schema."}}),
            &["object", "property", "value"],
        ),
        operation_schema(
            "SetObjectReference",
            json!({"object":target,"property":string,"reference":nullable_target}),
            &["object", "property", "reference"],
        ),
        operation_schema(
            "ChangeObjectId",
            json!({"old_id":id,"new_id":id}),
            &["old_id", "new_id"],
        ),
        operation_schema(
            "ReorderObjects",
            json!({"object_ids":{"type":"array","items":id}}),
            &["object_ids"],
        ),
        operation_schema(
            "AddChild",
            json!({"parent":target,"child":target,"x":coordinate,"y":coordinate}),
            &["parent", "child", "x", "y"],
        ),
        operation_schema(
            "RemoveChild",
            json!({"parent_id":id,"child_id":id}),
            &["parent_id", "child_id"],
        ),
        operation_schema(
            "SetChildPosition",
            json!({"parent_id":id,"child_id":id,"x":coordinate,"y":coordinate}),
            &["parent_id", "child_id", "x", "y"],
        ),
        operation_schema(
            "SetChildren",
            json!({"parent":target,"children":{"type":"array","items":object_schema(json!({"child":target,"x":coordinate,"y":coordinate}), &["child","x","y"])}}),
            &["parent", "children"],
        ),
        operation_schema(
            "SetObjectList",
            json!({"object":target,"objects":{"anyOf":[
                object_schema(json!({"kind":{"const":"Required","type":"string"},"objects":{"type":"array","items":target}}), &["kind","objects"]),
                object_schema(json!({"kind":{"const":"Nullable","type":"string"},"objects":{"type":"array","items":nullable_target}}), &["kind","objects"])
            ]}}),
            &["object", "objects"],
        ),
        operation_schema(
            "SetMacroReferences",
            json!({"object":target,"macro_refs":{"type":"array","items":object_schema(json!({"event_id":{"type":"string","description":"Event name from get_object_schema possible_events."},"macro_object":target}), &["event_id","macro_object"])}}),
            &["object", "macro_refs"],
        ),
        operation_schema(
            "SetObjectLabels",
            json!({"object_id":id,"labels":{"type":"array","items":object_schema(json!({"id":id,"font_type":{"type":"integer","minimum":0,"maximum":255},"string_variable_reference":{"anyOf":[id,{"type":"null"}]},"graphic_representation":{"anyOf":[id,{"type":"null"}]}}), &["id","font_type","string_variable_reference","graphic_representation"])}}),
            &["object_id", "labels"],
        ),
        operation_schema(
            "RenameObject",
            json!({"object":target,"name":string}),
            &["object", "name"],
        ),
    ];
    object_schema(
        json!({"schema_version":{"type":"integer","const":2},"description":{"type":["string","null"]},"operations":{"type":"array","minItems":1,"items":{"anyOf":variants}}}),
        &["schema_version", "operations"],
    )
}
/// Factor the repeated public ObjectTarget schema without changing its constraints.
fn compact_transaction_schema() -> Value {
    let mut schema = transaction_schema();
    let target = schema["properties"]["operations"]["items"]["anyOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["properties"]["type"]["const"] == "SetProperty")
        .unwrap()["properties"]["data"]["properties"]["object"]
        .clone();
    fn replace(value: &mut Value, target: &Value) {
        if value == target {
            *value = json!({"$ref":"#/$defs/ObjectTarget"});
            return;
        }
        match value {
            Value::Object(map) => {
                for v in map.values_mut() {
                    replace(v, target);
                }
            }
            Value::Array(array) => {
                for v in array {
                    replace(v, target);
                }
            }
            _ => {}
        }
    }
    replace(&mut schema, &target);
    schema["$defs"] = json!({"ObjectTarget":target});
    schema
}
fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn operation_schema(name: &str, properties: Value, required: &[&str]) -> Value {
    object_schema(
        json!({"type":{"type":"string","const":name},"data":object_schema(properties, required)}),
        &["type", "data"],
    )
}
pub fn get_design_context(
    state: &AiDesignState,
    request: DesignContextRequest,
) -> Result<DesignSnapshot, AiToolError> {
    let root = if let Some(id) = request.root_id {
        let object_id = ObjectId::new(id).map_err(|_| AiToolError::ObjectNotFound(id))?;
        if state.pool.object_by_id(object_id).is_none() {
            return Err(AiToolError::ObjectNotFound(id));
        }
        Some(object_id)
    } else {
        match request.scope {
            SnapshotScope::ActiveMask => active_mask(&state.pool),
            SnapshotScope::Selection => state.selected,
            SnapshotScope::EntirePool => None,
        }
    };
    let scope = if request.root_id.is_some() {
        SnapshotScope::Selection
    } else {
        request.scope
    };
    let depth = request.depth.min(crate::ai::snapshot::MAX_CONTEXT_DEPTH);
    let (all_ids, depth_limited) = selected_ids(&state.pool, root, scope, depth);
    let total_matching_objects = all_ids.len();
    let limit = request.max_objects.clamp(1, MAX_SNAPSHOT_OBJECTS);
    let mut layout = Vec::new();
    let mut earlier = std::collections::HashSet::new();
    let mut ids = Vec::new();
    let mut layout_more = false;
    // EntirePool retains explicit object pagination. Scoped reads page placements,
    // with a dictionary containing only identities first needed on this page.
    let object_paging = scope == SnapshotScope::EntirePool || request.offset > 0;
    let next_offset;
    if object_paging {
        ids.extend(all_ids.iter().skip(request.offset).take(limit).copied());
        next_offset = (request.offset.saturating_add(ids.len()) < total_matching_objects)
            .then_some(request.offset.saturating_add(ids.len()));
        if scope != SnapshotScope::EntirePool {
            layout =
                crate::ai::snapshot::context_layout(&state.pool, &state.object_info, root, depth)
                    .filter(|p| ids.iter().any(|id| id.value() == p.object.id))
                    .skip(request.layout_offset)
                    .take(limit + 1)
                    .collect();
            layout_more = layout.len() > limit;
            layout.truncate(limit);
        }
    } else {
        next_offset = None;
        if let Some(root) = root {
            earlier.insert(root.value());
            if request.layout_offset == 0 {
                ids.push(root);
            }
        }
        for (index, placement) in
            crate::ai::snapshot::context_layout(&state.pool, &state.object_info, root, depth)
                .enumerate()
        {
            if index < request.layout_offset {
                earlier.insert(placement.object.id);
                continue;
            }
            if layout.len() == limit {
                layout_more = true;
                break;
            }
            if earlier.insert(placement.object.id) {
                ids.push(ObjectId::new(placement.object.id).unwrap());
            }
            layout.push(placement);
        }
    }
    let mut properties_truncated = false;
    let objects: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            let mut object = if request.include_properties || request.include_references {
                object_snapshot_sections(&state.pool, &state.object_info, *id, false)?
            } else {
                let summary = crate::ai::snapshot::summary(&state.pool, &state.object_info, *id)?;
                crate::ai::snapshot::ObjectSnapshot {
                    id: summary.id,
                    name: summary.name,
                    object_type: summary.object_type,
                    properties: Default::default(),
                    references: Default::default(),
                    positioned_children: Vec::new(),
                    object_list: None,
                    macro_references: Vec::new(),
                    parents: Vec::new(),
                }
            };
            if !request.include_properties {
                object.properties.clear();
            } else {
                properties_truncated |=
                    page_properties(&mut object.properties, 0, MAX_PROPERTY_ITEMS);
            }
            if !request.include_references {
                object.references.clear();
            }
            // Relationships and full child lists are available through paginated get_objects.
            object.positioned_children.clear();
            object.object_list = None;
            object.parents.clear();
            object.macro_references.clear();
            Some(object)
        })
        .collect();
    let summary = crate::ai::snapshot::project_summary(state);
    Ok(DesignSnapshot {
        schema_version: 5,
        project: summary.project,
        selection: request
            .include_selection
            .then_some(summary.selected_object)
            .flatten(),
        returned_objects: objects.len(),
        total_matching_objects,
        next_offset,
        depth_limited,
        truncated: depth_limited || next_offset.is_some() || layout_more || properties_truncated,
        next_layout_offset: layout_more
            .then_some(request.layout_offset.saturating_add(layout.len())),
        layout_coordinates: "parent_relative".into(),
        properties_truncated,
        objects_truncated: next_offset.is_some(),
        layout_truncated: layout_more,
        objects,
        layout,
    })
}
pub fn get_objects(
    state: &AiDesignState,
    request: GetObjectsRequest,
) -> Result<Vec<ObjectDetails>, AiToolError> {
    if request.ids.len() > MAX_GET_OBJECTS {
        return Err(AiToolError::RequestTooLarge);
    }
    let mut ids = request.ids.clone();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let id = ObjectId::new(id).map_err(|_| AiToolError::ObjectNotFound(id))?;
            let mut object = object_snapshot_sections(
                &state.pool,
                &state.object_info,
                id,
                request.include_parents,
            )
            .ok_or(AiToolError::ObjectNotFound(id.value()))?;
            if !request.include_properties {
                object.properties.clear();
            }
            if !request.include_references {
                object.references.clear();
            }
            if !request.include_children {
                object.positioned_children.clear();
                object.object_list = None;
            }
            if !request.include_parents {
                object.parents.clear();
            }
            if !request.include_macros {
                object.macro_references.clear();
            }
            let property_limit = request.property_limit.clamp(1, MAX_PROPERTY_ITEMS);
            let property_more = page_properties(
                &mut object.properties,
                request.property_offset,
                property_limit,
            );
            let limit = request.detail_limit.clamp(1, MAX_DETAIL_ITEMS);
            let mut more = false;
            fn page<T>(v: &mut Vec<T>, offset: usize, limit: usize, more: &mut bool) {
                *more |= v.len() > offset.saturating_add(limit);
                *v = v.drain(..).skip(offset).take(limit).collect();
            }
            page(
                &mut object.positioned_children,
                request.detail_offset,
                limit,
                &mut more,
            );
            let parent_count = request.include_parents.then_some(object.parents.len());
            let parent_limit = request.parent_limit.clamp(1, MAX_PARENT_LIMIT);
            let mut parents_more = false;
            page(
                &mut object.parents,
                request.parent_offset,
                parent_limit,
                &mut parents_more,
            );
            page(
                &mut object.macro_references,
                request.detail_offset,
                limit,
                &mut more,
            );
            if let Some(list) = &mut object.object_list {
                match list {
                    crate::ai::snapshot::ObjectListSnapshot::Required(v) => {
                        page(v, request.detail_offset, limit, &mut more)
                    }
                    crate::ai::snapshot::ObjectListSnapshot::Nullable(v) => {
                        page(v, request.detail_offset, limit, &mut more)
                    }
                }
            }
            Ok(ObjectDetails {
                parent_count,
                parents_truncated: request.include_parents.then_some(parents_more),
                next_parent_offset: parents_more
                    .then_some(request.parent_offset.saturating_add(parent_limit)),
                nearby_style: request
                    .include_nearby_style
                    .then(|| nearby_style(state, id, request.detail_offset)),
                object,
                truncated: parents_more
                    || more
                    || request.detail_offset > 0
                    || property_more
                    || request.property_offset > 0,
                next_property_offset: property_more
                    .then_some(request.property_offset.saturating_add(property_limit)),
                next_detail_offset: more.then_some(request.detail_offset.saturating_add(limit)),
            })
        })
        .collect()
}

/// Inspect a container's display descendants, never its reverse users. This is
/// intentionally a bounded sample; detail_offset pages further examples.
fn nearby_style(state: &AiDesignState, root: ObjectId, offset: usize) -> Value {
    use crate::object_properties::get_object_property;
    let mut examples = Vec::new();
    let mut more = false;
    for placement in
        crate::ai::snapshot::context_layout(&state.pool, &state.object_info, Some(root), 2)
            .skip(offset)
            .take(MAX_NEARBY_STYLE + 1)
    {
        if examples.len() == MAX_NEARBY_STYLE {
            more = true;
            break;
        }
        let object = state
            .pool
            .object_by_id(ObjectId::new(placement.object.id).unwrap())
            .unwrap();
        let mut example = json!({"id":placement.object.id,"type":placement.object.object_type,
            "x":placement.x,"y":placement.y,"path":placement.path});
        for name in ["width", "height", "background_colour", "justification"] {
            if let Ok(value) = get_object_property(object, name) {
                example[name] = value;
            }
        }
        for name in ["font_attributes", "line_attributes", "fill_attributes"] {
            if let Ok(value) = get_object_property(object, name) {
                if let Ok(id) =
                    serde_json::from_value::<ag_iso_stack::object_pool::NullableObjectId>(value)
                {
                    if let Some(style) = id.0.and_then(|id| state.pool.object_by_id(id)) {
                        let mut resolved = json!({"id":style.id().value()});
                        for descriptor in
                            crate::object_properties::get_property_descriptors(style.object_type())
                        {
                            if let Ok(v) = get_object_property(style, descriptor.name) {
                                resolved[descriptor.name] = v;
                            }
                        }
                        example[name] = resolved;
                    }
                }
            }
        }
        examples.push(example);
    }
    json!({"examples":examples,"truncated":more,"next_offset":more.then_some(offset.saturating_add(MAX_NEARBY_STYLE))})
}

/// Long scalar strings and arrays remain accessible without unbounded tool payloads.
fn page_properties(
    properties: &mut std::collections::BTreeMap<String, Value>,
    offset: usize,
    limit: usize,
) -> bool {
    let mut more = false;
    for value in properties.values_mut() {
        match value {
            Value::String(text) => {
                more |= text.chars().count() > offset.saturating_add(limit);
                *text = text.chars().skip(offset).take(limit).collect();
            }
            Value::Array(items) => {
                more |= items.len() > offset.saturating_add(limit);
                *items = items.drain(..).skip(offset).take(limit).collect();
            }
            _ => {}
        }
    }
    more
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(schema: &Value) -> Value {
        if let Some(value) = schema.get("const") {
            return value.clone();
        }
        if let Some(values) = schema.get("enum") {
            return values[0].clone();
        }
        if let Some(variants) = schema.get("anyOf") {
            return example(&variants[0]);
        }
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
        let variants = schema["properties"]["operations"]["items"]["anyOf"]
            .as_array()
            .unwrap();
        assert_eq!(variants.len(), 14);
        for variant in variants {
            let payload = example(variant);
            let result = serde_json::from_value::<crate::operations::Operation>(payload.clone());
            assert!(result.is_ok(), "{payload}: {result:?}");
        }
    }
}

#[cfg(test)]
mod phase52_schema_tests {
    use super::*;
    #[test]
    fn factored_public_operation_schema_is_identical_when_expanded() {
        let original = transaction_schema();
        let mut compact = compact_transaction_schema();
        assert!(compact.to_string().len() < original.to_string().len());
        let target = compact["$defs"]["ObjectTarget"].clone();
        compact.as_object_mut().unwrap().remove("$defs");
        fn expand(value: &mut Value, target: &Value) {
            if value
                .get("$ref")
                .is_some_and(|v| v == "#/$defs/ObjectTarget")
            {
                *value = target.clone();
                return;
            }
            match value {
                Value::Object(map) => {
                    for v in map.values_mut() {
                        expand(v, target);
                    }
                }
                Value::Array(array) => {
                    for v in array {
                        expand(v, target);
                    }
                }
                _ => {}
            }
        }
        expand(&mut compact, &target);
        assert_eq!(compact, original);
    }
}

#[cfg(test)]
pub(crate) fn phase51_tool_definitions() -> Vec<Value> {
    let mut tools = tool_definitions();
    tools[2] = function("get_object_schema", "Returns writable properties, reference types, child relationships and supported events for one ISOBUS VT object type. Use before guessing valid fields.", json!({"type":"object","properties":{"object_type":{"type":"string","enum":ObjectType::values().into_iter().map(|v| format!("{v:?}")).collect::<Vec<_>>() }},"required":["object_type"],"additionalProperties":false}));
    tools[3]["parameters"] = transaction_schema();
    tools
}
