use super::AiDesignState;
use crate::cli::snapshot::{
    active_mask, object_snapshot_sections, selected_ids, DesignContextRequest, DesignSnapshot,
    SnapshotScope, MAX_SNAPSHOT_OBJECTS,
};
use ag_iso_stack::object_pool::ObjectId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    pub object: crate::cli::snapshot::ObjectSnapshot,
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
    let depth = request.depth.min(crate::cli::snapshot::MAX_CONTEXT_DEPTH);
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
                crate::cli::snapshot::context_layout(&state.pool, &state.object_info, root, depth)
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
            crate::cli::snapshot::context_layout(&state.pool, &state.object_info, root, depth)
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
                let summary = crate::cli::snapshot::summary(&state.pool, &state.object_info, *id)?;
                crate::cli::snapshot::ObjectSnapshot {
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
    let summary = crate::cli::snapshot::project_summary(state);
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
                    crate::cli::snapshot::ObjectListSnapshot::Required(v) => {
                        page(v, request.detail_offset, limit, &mut more)
                    }
                    crate::cli::snapshot::ObjectListSnapshot::Nullable(v) => {
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
        crate::cli::snapshot::context_layout(&state.pool, &state.object_info, Some(root), 2)
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
        for property in [
            crate::object_properties::PropertyKey::Width,
            crate::object_properties::PropertyKey::Height,
            crate::object_properties::PropertyKey::BackgroundColour,
            crate::object_properties::PropertyKey::BorderColour,
            crate::object_properties::PropertyKey::Justification,
        ] {
            let name = property.as_str();
            let Ok(value) = get_object_property(object, property) else {
                continue;
            };
            example[name] =
                super::visual_intent::enrich(&state.pool, object.object_type(), property, value);
        }
        for property in [
            crate::object_properties::PropertyKey::FontAttributes,
            crate::object_properties::PropertyKey::LineAttributes,
            crate::object_properties::PropertyKey::FillAttributes,
        ] {
            let name = property.as_str();
            if let Ok(value) = get_object_property(object, property) {
                if let Ok(id) =
                    serde_json::from_value::<ag_iso_stack::object_pool::NullableObjectId>(value)
                {
                    if let Some(style) = id.0.and_then(|id| state.pool.object_by_id(id)) {
                        let mut resolved =
                            super::visual_intent::property_style(&state.pool, style, None);
                        resolved["id"] = json!(style.id().value());
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
fn page_properties<K: Ord>(
    properties: &mut std::collections::BTreeMap<K, Value>,
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

pub fn component_catalog(state: &AiDesignState) -> Value {
    json!(state.components.definitions.values().map(|d| json!({"id":d.id,"name":d.name,"description":d.description,"category":d.category_path,"tags":d.tags,"revision":d.revision,"bounds":crate::components::component_bounds(d),"parameters":d.parameters})).collect::<Vec<_>>())
}

/// Semantic instance records omit compiled implementation details from normal context.
pub fn component_instances(state: &AiDesignState) -> Value {
    json!(state.components.instances.iter().map(|i| {
        let d = &state.components.definitions[&i.component_id];
        json!({"instance_id":i.id,"component_id":d.id,"component":d.name,"parent":i.parent,"x":i.x,"y":i.y,"parameters":d.parameters.iter().map(|p| json!({"id":p.id,"name":p.name,"value":i.overrides.get(&p.id).unwrap_or(&p.default_value),"overridden":i.overrides.contains_key(&p.id)})).collect::<Vec<_>>()})
    }).collect::<Vec<_>>())
}

pub fn source_context(source: &crate::components::ComponentSource, id: u16) -> Option<Value> {
    // Reservations are authoritative source metadata, never generated names.
    let map = crate::components::source_map(source);
    let crate::components::MaterializedSource::ComponentNode {
        instance_id,
        node_id,
    } = map.by_object_id.get(&id)?
    else {
        return None;
    };
    let instance = source.instances.iter().find(|i| i.id == *instance_id)?;
    let definition = source.definitions.get(&instance.component_id)?;
    let node = definition.nodes.iter().find(|n| n.id == *node_id)?;
    Some(
        json!({"component_id":definition.id,"component":definition.name,"instance_id":instance_id,"node_id":node_id,"node":node.name,"materialized_object_id":id}),
    )
}

pub fn component_design_context(
    state: &AiDesignState,
    snapshot: crate::cli::snapshot::DesignSnapshot,
) -> Value {
    let mut context = serde_json::to_value(snapshot).expect("snapshot serializes");
    let map = crate::components::source_map(&state.components);
    if let Some(objects) = context["objects"].as_array_mut() {
        objects.retain(|object| {
            !object["id"]
                .as_u64()
                .is_some_and(|id| map.by_object_id.contains_key(&(id as u16)))
        });
    }
    if let Some(layout) = context["layout"].as_array_mut() {
        layout.retain(|item| {
            !item["object"]["id"]
                .as_u64()
                .is_some_and(|id| map.by_object_id.contains_key(&(id as u16)))
        });
    }
    if let Some(id) = state.selected {
        if let Some(source) = source_context(&state.components, id.value()) {
            context["selection"] = source;
        }
    }
    context["components"] = component_catalog(state);
    context["component_instances"] = component_instances(state);
    context
}
