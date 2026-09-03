//! AI-friendly pool snapshots for LLM context
//! Distinguishes properties, references, and placements

use ag_iso_stack::object_pool::{
    object_attributes::ObjectRef, vt_version::VtVersion, ObjectId, ObjectPool,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Snapshot of pool state optimized for LLM consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolSnapshot {
    pub vt_version: String,
    pub mask_size: (u16, u16),
    pub selected_object: Option<u16>,
    pub objects: Vec<ObjectSnapshot>,
}

/// Snapshot of a single object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub id: u16,
    pub object_type: String,
    pub name: Option<String>,

    /// Object properties (excluding position, excluding references)
    pub properties: HashMap<String, serde_json::Value>,

    /// Child placements (position is part of relationship)
    pub children: Vec<ChildPlacement>,

    /// Objects that reference this one
    pub parents: Vec<u16>,
}

/// A child placement (relationship with position)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildPlacement {
    pub child_id: u16,
    pub x: i16,
    pub y: i16,
}

/// Snapshot error type
#[derive(Debug, Clone)]
pub enum SnapshotError {
    InvalidVtVersion,
    SerializationFailed(String),
}

impl PoolSnapshot {
    /// Create a snapshot from pool and metadata
    /// Takes explicit parameters instead of depending on EditorProject
    pub fn new(
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, crate::ObjectInfo>,
        selected_object: Option<ObjectId>,
        mask_size: (u16, u16),
        vt_version: VtVersion,
    ) -> Result<PoolSnapshot, SnapshotError> {
        let mut objects = Vec::new();

        for obj in pool.objects() {
            let properties = crate::object_properties::get_properties(pool, obj.id())
                .map_err(|error| SnapshotError::SerializationFailed(format!("{:?}", error)))?;
            let serialized = serde_json::to_value(obj)
                .map_err(|error| SnapshotError::SerializationFailed(error.to_string()))?;
            let children = serialized
                .get("properties")
                .and_then(|properties| properties.get("object_refs"))
                .cloned()
                .map(serde_json::from_value::<Vec<ObjectRef>>)
                .transpose()
                .map_err(|error| SnapshotError::SerializationFailed(error.to_string()))?
                .unwrap_or_default()
                .into_iter()
                .map(|reference| ChildPlacement {
                    child_id: reference.id.value(),
                    x: reference.offset.x,
                    y: reference.offset.y,
                })
                .collect();
            let object_snapshot = ObjectSnapshot {
                id: obj.id().value(),
                object_type: format!("{:?}", obj.object_type()),
                name: object_info
                    .get(&obj.id())
                    .and_then(|info| info.name.clone()),
                properties,
                children,
                parents: pool
                    .parent_objects(obj.id())
                    .into_iter()
                    .map(|parent_obj| parent_obj.id().value())
                    .collect(),
            };

            objects.push(object_snapshot);
        }

        Ok(PoolSnapshot {
            vt_version: format!("{:?}", vt_version),
            mask_size,
            selected_object: selected_object.map(|id| id.value()),
            objects,
        })
    }

    /// Serialize to compact JSON suitable for LLM context
    pub fn to_json(&self) -> Result<String, SnapshotError> {
        serde_json::to_string(self).map_err(|e| SnapshotError::SerializationFailed(e.to_string()))
    }

    /// Serialize to pretty-printed JSON
    pub fn to_json_pretty(&self) -> Result<String, SnapshotError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| SnapshotError::SerializationFailed(e.to_string()))
    }
}
