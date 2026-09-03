//! Single mutation operation on the object pool
//! Each operation is reversible: apply() returns its inverse

use crate::object_info::ObjectInfo;
use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::{ObjectId, ObjectPool, ObjectRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// A single mutation operation on the object pool
/// Each operation can generate its inverse during application
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Operation {
    /// Create a new object with default properties
    /// If object_id is None, executor allocates one
    /// If handle is Some, executor maps handle → allocated_id for same-transaction reference
    /// If captured_object is Some, deserialize and restore (from DeleteObject inverse)
    CreateObject {
        handle: Option<String>,
        object_id: Option<u16>,
        object_type: String,
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        captured_object: Option<serde_json::Value>,
        #[serde(skip)]
        captured_info: Option<ObjectInfo>,
    },

    /// Delete an object (must not be referenced by any other object)
    /// Inverse recreates with all metadata via captured_object
    DeleteObject {
        object_id: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        captured_object: Option<serde_json::Value>,
    },

    /// Set an object property (position excluded - handled via placement)
    /// Returns old value as inverse via JSON
    SetProperty {
        object_id: u16,
        property: String,
        value: Value,
    },

    /// Replace one complete object. This is primarily the adapter used by the
    /// existing egui property editors: widgets edit a draft object and the
    /// draft is committed as one reversible operation at the end of the frame.
    ReplaceObject { object_id: u16, object: Object },

    /// Change serialization/display order without copying the object pool.
    ReorderObjects { object_ids: Vec<u16> },

    /// Add a parent → child reference with placement at (x, y)
    /// Inverse is RemoveChild
    AddChild {
        parent_id: u16,
        child_id: u16,
        x: i16,
        y: i16,
    },

    /// Remove a parent → child reference
    /// Inverse is AddChild with remembered position
    RemoveChild { parent_id: u16, child_id: u16 },

    /// Update position of a child within its parent
    /// Returns old position as inverse
    SetChildPosition {
        parent_id: u16,
        child_id: u16,
        x: i16,
        y: i16,
    },

    /// Rename an object (modifies editor metadata, not ObjectPool)
    /// Returns old name as inverse
    RenameObject { object_id: u16, name: String },
}

/// Context passed to each operation during execution
/// Manages handle resolution and inverse collection
#[derive(Debug, Clone)]
pub struct OperationContext {
    /// Maps temporary handles (e.g., "rate_value") to allocated ObjectIds
    pub handle_map: HashMap<String, ObjectId>,
    /// Collects inverse operations as we apply forward operations
    /// Stored in execution order; reversed when creating undo transaction
    pub inverse_operations: Vec<Operation>,
    /// Track affected object IDs for diagnostics
    pub affected_objects: HashSet<ObjectId>,
}

impl Default for OperationContext {
    fn default() -> Self {
        Self {
            handle_map: HashMap::new(),
            inverse_operations: Vec::new(),
            affected_objects: HashSet::new(),
        }
    }
}

/// Errors that can occur during operation execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationError {
    ObjectNotFound(u16),
    CircularReference {
        parent: u16,
        child: u16,
    },
    InvalidReference {
        parent_type: String,
        child_type: String,
    },
    PropertyError {
        object_id: u16,
        property: String,
        reason: String,
    },
    DeleteReferencedObject(u16),
    ResolveHandleFailed {
        handle: String,
    },
    ObjectAlreadyExists(u16),
    InvalidObjectOrder,
}

impl Operation {
    /// Apply this operation to working pool and metadata
    /// Returns the inverse operation
    /// Caller (OperationExecutor) maintains OperationContext to map handles
    pub fn apply(
        &self,
        pool: &mut ObjectPool,
        object_info: &mut HashMap<ObjectId, ObjectInfo>,
        context: &mut OperationContext,
    ) -> Result<Operation, OperationError> {
        use crate::pool_validation::{validate_object_exists, would_create_circular_reference};

        match self {
            Operation::CreateObject {
                handle,
                object_id,
                object_type,
                name,
                captured_object,
                captured_info,
            } => {
                let target_id = if let Some(id) = object_id {
                    ObjectId::new(*id).map_err(|_| OperationError::ObjectNotFound(*id))?
                } else if let Some(id) = handle
                    .as_ref()
                    .and_then(|handle| context.handle_map.get(handle))
                {
                    *id
                } else {
                    (1..u16::MAX)
                        .filter_map(|value| ObjectId::new(value).ok())
                        .find(|id| pool.object_by_id(*id).is_none())
                        .ok_or(OperationError::PropertyError {
                            object_id: 0,
                            property: "CreateObject".to_string(),
                            reason: "No object IDs are available".to_string(),
                        })?
                };

                if pool.object_by_id(target_id).is_some() {
                    return Err(OperationError::ObjectAlreadyExists(target_id.value()));
                }

                // If we have a captured object (from DeleteObject inverse), deserialize and restore
                let mut obj = if let Some(captured_json) = captured_object {
                    serde_json::from_value::<Object>(captured_json.clone()).map_err(|_| {
                        OperationError::PropertyError {
                            object_id: object_id.unwrap_or(0),
                            property: "CreateObject".to_string(),
                            reason: "Failed to deserialize captured object".to_string(),
                        }
                    })?
                } else {
                    let object_type = ag_iso_stack::object_pool::ObjectType::values()
                        .into_iter()
                        .find(|candidate| format!("{:?}", candidate) == *object_type)
                        .ok_or_else(|| OperationError::PropertyError {
                            object_id: target_id.value(),
                            property: "CreateObject".to_string(),
                            reason: format!("Unknown object type {object_type}"),
                        })?;
                    let mut object = crate::default_object(object_type);
                    object.mut_id().set_value(target_id.value()).map_err(|_| {
                        OperationError::PropertyError {
                            object_id: target_id.value(),
                            property: "CreateObject".to_string(),
                            reason: "Invalid allocated object ID".to_string(),
                        }
                    })?;
                    object
                };
                obj.mut_id().set_value(target_id.value()).map_err(|_| {
                    OperationError::PropertyError {
                        object_id: target_id.value(),
                        property: "CreateObject".to_string(),
                        reason: "Invalid object ID".to_string(),
                    }
                })?;

                // Add to pool
                pool.add(obj.clone());

                if let Some(handle) = handle {
                    context.handle_map.insert(handle.clone(), target_id);
                }
                context.affected_objects.insert(target_id);

                // Add to object_info if name provided
                if let Some(info) = captured_info {
                    object_info.insert(target_id, info.clone());
                } else if let Some(n) = name {
                    let mut info = ObjectInfo::new(&obj);
                    info.name = Some(n.clone());
                    object_info.insert(target_id, info);
                }

                // Inverse is DeleteObject without captured state (will be captured on re-deletion)
                Ok(Operation::DeleteObject {
                    object_id: target_id.value(),
                    captured_object: None,
                })
            }

            Operation::DeleteObject {
                object_id,
                captured_object: _,
            } => {
                let id = ObjectId::new(*object_id)
                    .map_err(|_| OperationError::ObjectNotFound(*object_id))?;

                // Capture object if not already captured
                let obj = pool
                    .object_by_id(id)
                    .ok_or(OperationError::ObjectNotFound(*object_id))?;

                let object_type_str = format!("{:?}", obj.object_type());
                let old_name = object_info.get(&id).and_then(|info| info.name.clone());

                let captured_json =
                    Some(
                        serde_json::to_value(obj).map_err(|_| OperationError::PropertyError {
                            object_id: *object_id,
                            property: "DeleteObject".to_string(),
                            reason: "Failed to serialize object for capture".to_string(),
                        })?,
                    );
                let captured_info = object_info.get(&id).cloned();

                // RejectIfReferenced policy: prevent deletion if other objects reference this
                let referencing = crate::pool_validation::get_referencing_objects(pool, id);
                if !referencing.is_empty() {
                    return Err(OperationError::DeleteReferencedObject(*object_id));
                }

                // Delete from pool and metadata
                pool.remove(id);
                object_info.remove(&id);
                context.affected_objects.insert(id);

                // Inverse: CreateObject with captured state for full restoration
                Ok(Operation::CreateObject {
                    handle: None,
                    object_id: Some(*object_id),
                    object_type: object_type_str,
                    name: old_name,
                    captured_object: captured_json,
                    captured_info,
                })
            }

            Operation::SetProperty {
                object_id,
                property,
                value,
            } => {
                let id = ObjectId::new(*object_id)
                    .map_err(|_| OperationError::ObjectNotFound(*object_id))?;

                // Validate object exists
                validate_object_exists(pool, id).map_err(|diag| OperationError::PropertyError {
                    object_id: *object_id,
                    property: property.clone(),
                    reason: diag.message,
                })?;

                crate::object_properties::validate_property(pool, id, property, value).map_err(
                    |e| OperationError::PropertyError {
                        object_id: *object_id,
                        property: property.clone(),
                        reason: format!("{:?}", e),
                    },
                )?;

                // Get old value for inverse, then set new value.
                let old_value = crate::object_properties::get_property(pool, id, property)
                    .map_err(|e| OperationError::PropertyError {
                        object_id: *object_id,
                        property: property.clone(),
                        reason: format!("{:?}", e),
                    })?;

                crate::object_properties::set_property(pool, id, property, value.clone()).map_err(
                    |e| OperationError::PropertyError {
                        object_id: *object_id,
                        property: property.clone(),
                        reason: format!("{:?}", e),
                    },
                )?;

                // Inverse: SetProperty with old value
                Ok(Operation::SetProperty {
                    object_id: *object_id,
                    property: property.clone(),
                    value: old_value,
                })
            }

            Operation::ReplaceObject { object_id, object } => {
                let old_id = ObjectId::new(*object_id)
                    .map_err(|_| OperationError::ObjectNotFound(*object_id))?;
                let old_object = pool
                    .object_by_id(old_id)
                    .cloned()
                    .ok_or(OperationError::ObjectNotFound(*object_id))?;
                let new_id = object.id();

                if new_id != old_id && pool.object_by_id(new_id).is_some() {
                    return Err(OperationError::ObjectAlreadyExists(new_id.value()));
                }

                if new_id == old_id {
                    let old_references = old_object.referenced_objects();
                    for child_id in object.referenced_objects() {
                        if !old_references.contains(&child_id) {
                            would_create_circular_reference(pool, old_id, child_id).map_err(
                                |_| OperationError::CircularReference {
                                    parent: old_id.value(),
                                    child: child_id.value(),
                                },
                            )?;
                        }
                    }
                }

                let slot = pool
                    .objects_mut()
                    .iter_mut()
                    .find(|candidate| candidate.id() == old_id)
                    .ok_or(OperationError::ObjectNotFound(*object_id))?;
                *slot = object.clone();

                if new_id != old_id {
                    if let Some(info) = object_info.remove(&old_id) {
                        object_info.insert(new_id, info);
                    }
                }
                context.affected_objects.insert(old_id);
                context.affected_objects.insert(new_id);

                Ok(Operation::ReplaceObject {
                    object_id: new_id.value(),
                    object: old_object,
                })
            }

            Operation::ReorderObjects { object_ids } => {
                if object_ids.len() != pool.objects().len() {
                    return Err(OperationError::InvalidObjectOrder);
                }
                let old_order = pool
                    .objects()
                    .iter()
                    .map(|object| object.id().value())
                    .collect();
                let mut reordered = Vec::with_capacity(object_ids.len());
                for value in object_ids {
                    let id =
                        ObjectId::new(*value).map_err(|_| OperationError::InvalidObjectOrder)?;
                    let object = pool
                        .object_by_id(id)
                        .cloned()
                        .ok_or(OperationError::InvalidObjectOrder)?;
                    if reordered
                        .iter()
                        .any(|existing: &Object| existing.id() == id)
                    {
                        return Err(OperationError::InvalidObjectOrder);
                    }
                    reordered.push(object);
                }
                pool.objects_mut().clone_from_slice(&reordered);
                Ok(Operation::ReorderObjects {
                    object_ids: old_order,
                })
            }

            Operation::AddChild {
                parent_id,
                child_id,
                x,
                y,
            } => {
                let parent_obj_id = ObjectId::new(*parent_id)
                    .map_err(|_| OperationError::ObjectNotFound(*parent_id))?;
                let child_obj_id = ObjectId::new(*child_id)
                    .map_err(|_| OperationError::ObjectNotFound(*child_id))?;

                // Validate both objects exist
                validate_object_exists(pool, parent_obj_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *parent_id,
                        property: "parent".to_string(),
                        reason: diag.message,
                    }
                })?;

                validate_object_exists(pool, child_obj_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *child_id,
                        property: "child".to_string(),
                        reason: diag.message,
                    }
                })?;

                // Check for circular references
                would_create_circular_reference(pool, parent_obj_id, child_obj_id).map_err(
                    |_| OperationError::CircularReference {
                        parent: *parent_id,
                        child: *child_id,
                    },
                )?;
                crate::pool_validation::validate_parent_child_relationship(
                    pool,
                    parent_obj_id,
                    child_obj_id,
                )
                .map_err(|diag| OperationError::InvalidReference {
                    parent_type: format!("{}: {}", parent_id, diag.message),
                    child_type: child_id.to_string(),
                })?;

                let refs = object_refs_mut(pool, parent_obj_id, "AddChild")?;
                refs.push(ObjectRef {
                    id: child_obj_id,
                    offset: Point { x: *x, y: *y },
                });
                context.affected_objects.insert(parent_obj_id);
                context.affected_objects.insert(child_obj_id);
                Ok(Operation::RemoveChild {
                    parent_id: *parent_id,
                    child_id: *child_id,
                })
            }

            Operation::RemoveChild {
                parent_id,
                child_id,
            } => {
                let parent_obj_id = ObjectId::new(*parent_id)
                    .map_err(|_| OperationError::ObjectNotFound(*parent_id))?;
                let child_obj_id = ObjectId::new(*child_id)
                    .map_err(|_| OperationError::ObjectNotFound(*child_id))?;

                // Verify both objects exist
                validate_object_exists(pool, parent_obj_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *parent_id,
                        property: "parent".to_string(),
                        reason: diag.message,
                    }
                })?;

                validate_object_exists(pool, child_obj_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *child_id,
                        property: "child".to_string(),
                        reason: diag.message,
                    }
                })?;

                let refs = object_refs_mut(pool, parent_obj_id, "RemoveChild")?;
                let index = refs
                    .iter()
                    .position(|reference| reference.id == child_obj_id)
                    .ok_or_else(|| OperationError::PropertyError {
                        object_id: *parent_id,
                        property: "RemoveChild".to_string(),
                        reason: format!("Object {} is not a child", child_id),
                    })?;
                let removed = refs.remove(index);
                context.affected_objects.insert(parent_obj_id);
                context.affected_objects.insert(child_obj_id);
                Ok(Operation::AddChild {
                    parent_id: *parent_id,
                    child_id: *child_id,
                    x: removed.offset.x,
                    y: removed.offset.y,
                })
            }

            Operation::SetChildPosition {
                parent_id,
                child_id,
                x,
                y,
            } => {
                let parent_obj_id = ObjectId::new(*parent_id)
                    .map_err(|_| OperationError::ObjectNotFound(*parent_id))?;
                let child_obj_id = ObjectId::new(*child_id)
                    .map_err(|_| OperationError::ObjectNotFound(*child_id))?;

                // Verify both objects exist
                validate_object_exists(pool, parent_obj_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *parent_id,
                        property: "parent".to_string(),
                        reason: diag.message,
                    }
                })?;

                validate_object_exists(pool, child_obj_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *child_id,
                        property: "child".to_string(),
                        reason: diag.message,
                    }
                })?;

                let refs = object_refs_mut(pool, parent_obj_id, "SetChildPosition")?;
                let reference = refs
                    .iter_mut()
                    .find(|reference| reference.id == child_obj_id)
                    .ok_or_else(|| OperationError::PropertyError {
                        object_id: *parent_id,
                        property: "SetChildPosition".to_string(),
                        reason: format!("Object {} is not a child", child_id),
                    })?;
                let old = reference.offset;
                reference.offset = Point { x: *x, y: *y };
                context.affected_objects.insert(parent_obj_id);
                context.affected_objects.insert(child_obj_id);
                Ok(Operation::SetChildPosition {
                    parent_id: *parent_id,
                    child_id: *child_id,
                    x: old.x,
                    y: old.y,
                })
            }

            Operation::RenameObject { object_id, name } => {
                let id = ObjectId::new(*object_id)
                    .map_err(|_| OperationError::ObjectNotFound(*object_id))?;

                // Get old name for inverse operation
                let old_name = if let Some(info) = object_info.get(&id) {
                    info.name.clone().unwrap_or_default()
                } else {
                    String::new()
                };

                // Update or create object info with new name
                if let Some(info) = object_info.get_mut(&id) {
                    info.name = Some(name.clone());
                } else {
                    let object = pool
                        .object_by_id(id)
                        .ok_or(OperationError::ObjectNotFound(*object_id))?;
                    let mut info = ObjectInfo::new(object);
                    info.name = Some(name.clone());
                    object_info.insert(id, info);
                }
                context.affected_objects.insert(id);

                // Return inverse operation (rename back to old name)
                Ok(Operation::RenameObject {
                    object_id: *object_id,
                    name: old_name,
                })
            }
        }
    }
}

fn object_refs_mut<'a>(
    pool: &'a mut ObjectPool,
    parent_id: ObjectId,
    operation: &str,
) -> Result<&'a mut Vec<ObjectRef>, OperationError> {
    let object = pool
        .object_mut_by_id(parent_id)
        .ok_or(OperationError::ObjectNotFound(parent_id.value()))?;
    match object {
        Object::WorkingSet(object) => Ok(&mut object.object_refs),
        Object::DataMask(object) => Ok(&mut object.object_refs),
        Object::AlarmMask(object) => Ok(&mut object.object_refs),
        Object::Container(object) => Ok(&mut object.object_refs),
        Object::Key(object) => Ok(&mut object.object_refs),
        Object::Button(object) => Ok(&mut object.object_refs),
        Object::AuxiliaryFunctionType1(object) => Ok(&mut object.object_refs),
        Object::AuxiliaryInputType1(object) => Ok(&mut object.object_refs),
        Object::AuxiliaryFunctionType2(object) => Ok(&mut object.object_refs),
        Object::AuxiliaryInputType2(object) => Ok(&mut object.object_refs),
        Object::WindowMask(object) => Ok(&mut object.object_refs),
        Object::Animation(object) => Ok(&mut object.object_refs),
        _ => Err(OperationError::PropertyError {
            object_id: parent_id.value(),
            property: operation.to_owned(),
            reason: "This object type does not support positioned children".to_owned(),
        }),
    }
}
