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
fn transaction_schema() -> Value { json!({"type":"object","properties":{"schema_version":{"type":"integer","const":2},"description":{"type":["string","null"]},"operations":{"type":"array","minItems":1,"items":{"type":"object","properties":{"type":{"type":"string","enum":["CreateObject","DeleteObject","SetProperty","SetObjectReference","ChangeObjectId","ReorderObjects","AddChild","RemoveChild","SetChildPosition","SetChildren","SetObjectList","SetMacroReferences","SetObjectLabels","RenameObject"]},"data":{"type":"object","description":"Payload for the selected public Operation variant. Object references use {id:number} or {name:string}."}},"required":["type","data"]}}},"required":["schema_version","operations"],"additionalProperties":false}) }
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
