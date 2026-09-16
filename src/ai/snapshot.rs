//! Deterministic, semantic snapshots used by provider-neutral AI tools.
use crate::object_properties::{get_object_property, get_property_descriptors, PropertySemantic};
use crate::operations::{macro_refs, object_list, object_refs, ObjectReferenceList};
use crate::{ObjectInfo, ProjectRevision};
use ag_iso_stack::object_pool::{object::Object, NullableObjectId, ObjectId, ObjectPool};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_SNAPSHOT_OBJECTS: usize = 50;
pub const MAX_CONTEXT_DEPTH: usize = 8;
pub const MAX_SNAPSHOT_DEPTH: usize = 16;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignSnapshot {
    pub schema_version: u32,
    pub project: ProjectSnapshot,
    pub selection: Option<ObjectSummary>,
    pub objects: Vec<ObjectSnapshot>,
    pub layout: Vec<LayoutInstance>,
    pub truncated: bool,
    pub returned_objects: usize,
    pub total_matching_objects: usize,
    pub next_offset: Option<usize>,
    pub depth_limited: bool,
    pub next_layout_offset: Option<usize>,
    pub layout_coordinates: String,
    pub properties_truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub revision: ProjectRevision,
    pub vt_version: String,
    pub mask_width: u16,
    pub mask_height: u16,
    pub object_count: usize,
    pub active_mask: Option<ObjectSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectSummary {
    pub id: u16,
    pub name: String,
    pub object_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub id: u16,
    pub name: String,
    pub object_type: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub references: BTreeMap<String, ObjectReferenceSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positioned_children: Vec<PositionedChildSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_list: Option<ObjectListSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macro_references: Vec<MacroReferenceSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<ObjectSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectReferenceSnapshot {
    pub id: Option<u16>,
    pub name: Option<String>,
    pub object_type: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionedChildSnapshot {
    pub child: ObjectSummary,
    pub x: i16,
    pub y: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "objects")]
pub enum ObjectListSnapshot {
    Required(Vec<ObjectSummary>),
    Nullable(Vec<Option<ObjectSummary>>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroReferenceSnapshot {
    pub event: String,
    pub macro_id: u16,
    pub macro_name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutInstance {
    pub object: ObjectSummary,
    pub parent: ObjectSummary,
    pub x: i32,
    pub y: i32,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub depth: usize,
    pub path: Vec<u16>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SnapshotScope {
    ActiveMask,
    Selection,
    EntirePool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesignContextRequest {
    pub scope: SnapshotScope,
    pub include_selection: bool,
    pub depth: usize,
    pub max_objects: usize,
    pub offset: usize,
    pub layout_offset: usize,
    pub root_id: Option<u16>,
    pub include_properties: bool,
    pub include_references: bool,
}
impl Default for DesignContextRequest {
    fn default() -> Self {
        Self {
            scope: SnapshotScope::ActiveMask,
            include_selection: true,
            depth: MAX_CONTEXT_DEPTH,
            offset: 0,
            layout_offset: 0,
            root_id: None,
            include_properties: false,
            include_references: false,
            max_objects: MAX_SNAPSHOT_OBJECTS,
        }
    }
}
#[derive(Debug, Clone)]
pub enum SnapshotError {
    SerializationFailed(String),
}

pub(crate) fn summary(
    pool: &ObjectPool,
    info: &HashMap<ObjectId, ObjectInfo>,
    id: ObjectId,
) -> Option<ObjectSummary> {
    let object = pool.object_by_id(id)?;
    Some(ObjectSummary {
        id: id.value(),
        name: info
            .get(&id)
            .and_then(|v| v.name.clone())
            .unwrap_or_else(|| format!("Object_{}", id.value())),
        object_type: format!("{:?}", object.object_type()),
    })
}
fn reference(
    pool: &ObjectPool,
    info: &HashMap<ObjectId, ObjectInfo>,
    value: Value,
) -> ObjectReferenceSnapshot {
    let id = serde_json::from_value::<NullableObjectId>(value)
        .ok()
        .and_then(|id| id.0)
        .map(|id| id.value());
    let found = id
        .and_then(|id| ObjectId::new(id).ok())
        .and_then(|id| summary(pool, info, id));
    ObjectReferenceSnapshot {
        id,
        name: found.as_ref().map(|v| v.name.clone()),
        object_type: found.map(|v| v.object_type),
    }
}
pub(crate) fn object_snapshot(
    pool: &ObjectPool,
    info: &HashMap<ObjectId, ObjectInfo>,
    id: ObjectId,
) -> Option<ObjectSnapshot> {
    let object = pool.object_by_id(id)?;
    let s = summary(pool, info, id)?;
    let mut properties = BTreeMap::new();
    let mut references = BTreeMap::new();
    for descriptor in get_property_descriptors(object.object_type()) {
        let Ok(value) = get_object_property(object, descriptor.name) else {
            continue;
        };
        if matches!(
            descriptor.semantic,
            PropertySemantic::ObjectReference { .. }
        ) {
            references.insert(descriptor.name.to_owned(), reference(pool, info, value));
        } else {
            properties.insert(
                descriptor.name.to_owned(),
                sanitize_binary(descriptor.name, value),
            );
        }
    }
    let positioned_children = object_refs(object)
        .unwrap_or_default()
        .iter()
        .filter_map(|placement| {
            summary(pool, info, placement.id).map(|child| PositionedChildSnapshot {
                child,
                x: placement.offset.x,
                y: placement.offset.y,
            })
        })
        .collect();
    let object_list = object_list(object).map(|list| match list {
        ObjectReferenceList::Required(ids) => ObjectListSnapshot::Required(
            ids.into_iter()
                .filter_map(|id| summary(pool, info, id))
                .collect(),
        ),
        ObjectReferenceList::Nullable(ids) => ObjectListSnapshot::Nullable(
            ids.into_iter()
                .map(|id| id.0.and_then(|id| summary(pool, info, id)))
                .collect(),
        ),
    });
    let macro_references = macro_refs(object)
        .unwrap_or_default()
        .iter()
        .map(|r| {
            let macro_id = u16::from(r.macro_id);
            let macro_name = ObjectId::new(macro_id)
                .ok()
                .and_then(|id| summary(pool, info, id))
                .map(|s| s.name);
            MacroReferenceSnapshot {
                event: format!("{:?}", r.event_id),
                macro_id,
                macro_name,
            }
        })
        .collect();
    let mut parents: Vec<_> = pool
        .parent_objects(id)
        .into_iter()
        .filter_map(|p| summary(pool, info, p.id()))
        .collect();
    parents.sort_by_key(|p| p.id);
    parents.dedup_by_key(|p| p.id);
    Some(ObjectSnapshot {
        id: s.id,
        name: s.name,
        object_type: s.object_type,
        properties,
        references,
        positioned_children,
        object_list,
        macro_references,
        parents,
    })
}
fn sanitize_binary(name: &str, value: Value) -> Value {
    if name == "data" {
        if let Value::Array(bytes) = &value {
            return serde_json::json!({ "byte_length": bytes.len() });
        }
    }
    value
}
pub(crate) fn active_mask(pool: &ObjectPool) -> Option<ObjectId> {
    pool.objects()
        .iter()
        .find(|o| matches!(o, Object::WorkingSet(_)))
        .and_then(|o| get_object_property(o, "active_mask").ok())
        .and_then(|v| serde_json::from_value::<NullableObjectId>(v).ok())
        .and_then(|id| id.0)
}
pub(crate) fn layout_instances(
    pool: &ObjectPool,
    info: &HashMap<ObjectId, ObjectInfo>,
    root: Option<ObjectId>,
    max_depth: usize,
) -> Vec<LayoutInstance> {
    let Some(root) = root else { return Vec::new() };
    let Some(root_summary) = summary(pool, info, root) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut stack = vec![(
        root,
        0i32,
        0i32,
        0usize,
        vec![root.value()],
        HashSet::from([root]),
    )];
    while let Some((parent_id, px, py, depth, path, ancestors)) = stack.pop() {
        if depth >= max_depth {
            continue;
        }
        let Some(parent) = pool.object_by_id(parent_id) else {
            continue;
        };
        for placement in object_refs(parent).unwrap_or_default().iter().rev() {
            if ancestors.contains(&placement.id) {
                continue;
            }
            let Some(child) = summary(pool, info, placement.id) else {
                continue;
            };
            let x = px + i32::from(placement.offset.x);
            let y = py + i32::from(placement.offset.y);
            let Some(child_object) = pool.object_by_id(placement.id) else {
                continue;
            };
            let width = get_object_property(child_object, "width")
                .ok()
                .and_then(|v| v.as_u64())
                .and_then(|v| u16::try_from(v).ok());
            let height = get_object_property(child_object, "height")
                .ok()
                .and_then(|v| v.as_u64())
                .and_then(|v| u16::try_from(v).ok());
            let mut child_path = path.clone();
            child_path.push(placement.id.value());
            result.push(LayoutInstance {
                object: child.clone(),
                parent: summary(pool, info, parent_id).unwrap_or_else(|| root_summary.clone()),
                x,
                y,
                width,
                height,
                depth: depth + 1,
                path: child_path.clone(),
            });
            let mut child_ancestors = ancestors.clone();
            child_ancestors.insert(placement.id);
            stack.push((placement.id, x, y, depth + 1, child_path, child_ancestors));
        }
    }
    result
}
pub(crate) fn selected_ids(
    pool: &ObjectPool,
    root: Option<ObjectId>,
    scope: SnapshotScope,
    depth: usize,
) -> (Vec<ObjectId>, bool) {
    if scope == SnapshotScope::EntirePool {
        let mut ids: Vec<_> = pool.objects().iter().map(Object::id).collect();
        ids.sort_by_key(|id| id.value());
        return (ids, false);
    }
    let Some(root) = root else {
        return (Vec::new(), false);
    };
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![(root, 0usize)];
    let mut truncated = false;
    while let Some((id, level)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        ids.push(id);
        if level >= depth {
            truncated |= pool
                .object_by_id(id)
                .and_then(object_refs)
                .is_some_and(|children| !children.is_empty());
            continue;
        }
        if let Some(object) = pool.object_by_id(id) {
            if let Some(children) = object_refs(object) {
                for child in children.iter().rev() {
                    stack.push((child.id, level + 1));
                }
            }
        }
    }
    (ids, truncated)
}
pub type PoolSnapshot = DesignSnapshot;

/// Constant-size context: no traversal or property serialization.
#[derive(Debug, Clone, Serialize)]
pub struct AiProjectSummary {
    #[serde(flatten)]
    pub project: ProjectSnapshot,
    pub selected_object: Option<ObjectSummary>,
}
pub fn project_summary(state: &super::AiDesignState) -> AiProjectSummary {
    AiProjectSummary {
        project: ProjectSnapshot {
            revision: state.revision,
            vt_version: format!("{:?}", state.vt_version),
            mask_width: state.mask_size,
            mask_height: state.mask_size,
            object_count: state.pool.objects().len(),
            active_mask: active_mask(&state.pool)
                .and_then(|id| summary(&state.pool, &state.object_info, id)),
        },
        selected_object: state
            .selected
            .and_then(|id| summary(&state.pool, &state.object_info, id)),
    }
}
