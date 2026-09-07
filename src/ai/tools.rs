use super::AiDesignState;
use crate::ai::snapshot::{
    active_mask, layout_instances, object_snapshot, selected_ids, DesignContextRequest,
    DesignSnapshot, ProjectSnapshot, SnapshotScope, MAX_SNAPSHOT_OBJECTS,
};
use ag_iso_stack::object_pool::ObjectId;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiDomainTool {
    GetDesignContext,
    GetObjects,
    GetObjectSchema,
    ProposeTransaction,
}
#[derive(Debug, Clone)]
pub enum AiToolError {
    ObjectNotFound(u16),
    RequestTooLarge,
    InvalidScope,
}
#[derive(Debug, Clone, Default)]
pub struct GetObjectsRequest {
    pub ids: Vec<u16>,
    pub include_properties: bool,
    pub include_references: bool,
    pub include_children: bool,
    pub include_parents: bool,
    pub include_macros: bool,
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
