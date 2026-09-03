//! Single mutation operation on the object pool
//! Each operation is reversible: apply() returns its inverse

use crate::object_info::ObjectInfo;
use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::object_attributes::{MacroRef, ObjectLabel, Point};
use ag_iso_stack::object_pool::{NullableObjectId, ObjectId, ObjectPool, ObjectRef};
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

    /// Change an object's identity and rewrite every inbound reference.
    ChangeObjectId { old_id: u16, new_id: u16 },

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

    /// Replace the complete ordered positioned-child list.
    /// This covers reorder and child replacement edits that cannot be expressed
    /// by AddChild/RemoveChild/SetChildPosition alone.
    SetChildren {
        parent_id: u16,
        children: Vec<ObjectRef>,
    },

    /// Replace an object's ordered, unpositioned child-reference list.
    SetObjectList {
        object_id: u16,
        objects: ObjectReferenceList,
    },

    /// Replace the complete ordered macro-reference list.
    SetMacroReferences {
        object_id: u16,
        macro_refs: Vec<MacroRef>,
    },

    /// Replace the labels owned by an ObjectLabelReferenceList.
    SetObjectLabels {
        object_id: u16,
        labels: Vec<ObjectLabel>,
    },

    /// Rename an object (modifies editor metadata, not ObjectPool)
    /// Returns old name as inverse
    RenameObject { object_id: u16, name: String },
}

/// The two list representations used for unpositioned child references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "objects")]
pub enum ObjectReferenceList {
    Required(Vec<ObjectId>),
    Nullable(Vec<NullableObjectId>),
}

impl ObjectReferenceList {
    fn ids(&self) -> impl Iterator<Item = ObjectId> + '_ {
        match self {
            Self::Required(objects) => {
                Box::new(objects.iter().copied()) as Box<dyn Iterator<Item = ObjectId>>
            }
            Self::Nullable(objects) => Box::new(objects.iter().filter_map(|object| object.0)),
        }
    }
}

fn object_label_references(label: &ObjectLabel) -> impl Iterator<Item = ObjectId> + '_ {
    [
        Some(label.id),
        label.string_variable_reference.0,
        label.graphic_representation.0,
    ]
    .into_iter()
    .flatten()
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

            Operation::ChangeObjectId { old_id, new_id } => {
                let old_id =
                    ObjectId::new(*old_id).map_err(|_| OperationError::ObjectNotFound(*old_id))?;
                let new_id =
                    ObjectId::new(*new_id).map_err(|_| OperationError::ObjectNotFound(*new_id))?;
                validate_object_exists(pool, old_id)
                    .map_err(|_| OperationError::ObjectNotFound(old_id.value()))?;
                if pool.object_by_id(new_id).is_some() {
                    return Err(OperationError::ObjectAlreadyExists(new_id.value()));
                }

                for object in pool.objects_mut() {
                    let references_changed = replace_references(object, old_id, new_id)?;
                    if references_changed {
                        context.affected_objects.insert(object.id());
                    }
                    if object.id() == old_id {
                        object.mut_id().set_value(new_id.value()).map_err(|error| {
                            OperationError::PropertyError {
                                object_id: old_id.value(),
                                property: "id".to_owned(),
                                reason: format!("{error:?}"),
                            }
                        })?;
                    }
                }

                if let Some(info) = object_info.remove(&old_id) {
                    object_info.insert(new_id, info);
                }
                context.affected_objects.insert(old_id);
                context.affected_objects.insert(new_id);

                Ok(Operation::ChangeObjectId {
                    old_id: new_id.value(),
                    new_id: old_id.value(),
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

            Operation::SetChildren {
                parent_id,
                children,
            } => {
                let parent_obj_id = ObjectId::new(*parent_id)
                    .map_err(|_| OperationError::ObjectNotFound(*parent_id))?;
                validate_object_exists(pool, parent_obj_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *parent_id,
                        property: "children".to_owned(),
                        reason: diag.message,
                    }
                })?;

                for child in children {
                    validate_object_exists(pool, child.id).map_err(|diag| {
                        OperationError::PropertyError {
                            object_id: *parent_id,
                            property: "children".to_owned(),
                            reason: diag.message,
                        }
                    })?;
                    would_create_circular_reference(pool, parent_obj_id, child.id).map_err(
                        |_| OperationError::CircularReference {
                            parent: *parent_id,
                            child: child.id.value(),
                        },
                    )?;
                    crate::pool_validation::validate_parent_child_relationship(
                        pool,
                        parent_obj_id,
                        child.id,
                    )
                    .map_err(|diag| OperationError::InvalidReference {
                        parent_type: format!("{}: {}", parent_id, diag.message),
                        child_type: child.id.value().to_string(),
                    })?;
                }

                let refs = object_refs_mut(pool, parent_obj_id, "SetChildren")?;
                let old_children = std::mem::replace(refs, children.clone());
                context.affected_objects.insert(parent_obj_id);
                context
                    .affected_objects
                    .extend(old_children.iter().chain(children).map(|child| child.id));

                Ok(Operation::SetChildren {
                    parent_id: *parent_id,
                    children: old_children,
                })
            }

            Operation::SetObjectList { object_id, objects } => {
                let parent_id = ObjectId::new(*object_id)
                    .map_err(|_| OperationError::ObjectNotFound(*object_id))?;
                validate_object_exists(pool, parent_id).map_err(|diag| {
                    OperationError::PropertyError {
                        object_id: *object_id,
                        property: "objects".to_owned(),
                        reason: diag.message,
                    }
                })?;

                for child_id in objects.ids() {
                    validate_object_exists(pool, child_id).map_err(|diag| {
                        OperationError::PropertyError {
                            object_id: *object_id,
                            property: "objects".to_owned(),
                            reason: diag.message,
                        }
                    })?;
                    would_create_circular_reference(pool, parent_id, child_id).map_err(|_| {
                        OperationError::CircularReference {
                            parent: *object_id,
                            child: child_id.value(),
                        }
                    })?;
                    crate::pool_validation::validate_parent_child_relationship(
                        pool, parent_id, child_id,
                    )
                    .map_err(|diag| OperationError::InvalidReference {
                        parent_type: format!("{}: {}", object_id, diag.message),
                        child_type: child_id.value().to_string(),
                    })?;
                }

                let parent = pool
                    .object_mut_by_id(parent_id)
                    .ok_or(OperationError::ObjectNotFound(*object_id))?;
                let old_objects =
                    object_list(parent).ok_or_else(|| OperationError::PropertyError {
                        object_id: *object_id,
                        property: "objects".to_owned(),
                        reason: "This object type does not support an object list".to_owned(),
                    })?;
                if !set_object_list(parent, objects.clone()) {
                    return Err(OperationError::PropertyError {
                        object_id: *object_id,
                        property: "objects".to_owned(),
                        reason: "Object-list representation does not match object type".to_owned(),
                    });
                }

                context.affected_objects.insert(parent_id);
                context
                    .affected_objects
                    .extend(old_objects.ids().chain(objects.ids()));
                Ok(Operation::SetObjectList {
                    object_id: *object_id,
                    objects: old_objects,
                })
            }

            Operation::SetMacroReferences {
                object_id,
                macro_refs,
            } => {
                let id = ObjectId::new(*object_id)
                    .map_err(|_| OperationError::ObjectNotFound(*object_id))?;

                for macro_ref in macro_refs {
                    let macro_id = ObjectId::new(u16::from(macro_ref.macro_id)).map_err(|_| {
                        OperationError::PropertyError {
                            object_id: *object_id,
                            property: "macro_refs".to_owned(),
                            reason: format!("Invalid macro object ID {}", macro_ref.macro_id),
                        }
                    })?;
                    let macro_object = pool.object_by_id(macro_id).ok_or_else(|| {
                        OperationError::PropertyError {
                            object_id: *object_id,
                            property: "macro_refs".to_owned(),
                            reason: format!("Macro object {} does not exist", macro_ref.macro_id),
                        }
                    })?;
                    if macro_object.object_type() != ag_iso_stack::object_pool::ObjectType::Macro {
                        return Err(OperationError::InvalidReference {
                            parent_type: format!("Object {} macro reference", object_id),
                            child_type: format!("{:?}", macro_object.object_type()),
                        });
                    }
                }

                let refs = macro_refs_mut(pool, id, "SetMacroReferences")?;
                let old_macro_refs = std::mem::replace(refs, macro_refs.clone());
                context.affected_objects.insert(id);
                for macro_ref in old_macro_refs.iter().chain(macro_refs) {
                    if let Ok(macro_id) = ObjectId::new(u16::from(macro_ref.macro_id)) {
                        context.affected_objects.insert(macro_id);
                    }
                }

                Ok(Operation::SetMacroReferences {
                    object_id: *object_id,
                    macro_refs: old_macro_refs,
                })
            }

            Operation::SetObjectLabels { object_id, labels } => {
                let id = ObjectId::new(*object_id)
                    .map_err(|_| OperationError::ObjectNotFound(*object_id))?;
                for reference_id in labels.iter().flat_map(object_label_references) {
                    validate_object_exists(pool, reference_id).map_err(|diag| {
                        OperationError::PropertyError {
                            object_id: *object_id,
                            property: "object_labels".to_owned(),
                            reason: diag.message,
                        }
                    })?;
                    would_create_circular_reference(pool, id, reference_id).map_err(|_| {
                        OperationError::CircularReference {
                            parent: *object_id,
                            child: reference_id.value(),
                        }
                    })?;
                    crate::pool_validation::validate_parent_child_relationship(
                        pool,
                        id,
                        reference_id,
                    )
                    .map_err(|diag| OperationError::InvalidReference {
                        parent_type: format!("{}: {}", object_id, diag.message),
                        child_type: reference_id.value().to_string(),
                    })?;
                }

                let object = pool
                    .object_mut_by_id(id)
                    .ok_or(OperationError::ObjectNotFound(*object_id))?;
                let Object::ObjectLabelReferenceList(object) = object else {
                    return Err(OperationError::PropertyError {
                        object_id: *object_id,
                        property: "object_labels".to_owned(),
                        reason: "This object type does not support object labels".to_owned(),
                    });
                };
                let old_labels = std::mem::replace(&mut object.object_labels, labels.clone());
                context.affected_objects.insert(id);
                context.affected_objects.extend(
                    old_labels
                        .iter()
                        .chain(labels)
                        .flat_map(object_label_references),
                );

                Ok(Operation::SetObjectLabels {
                    object_id: *object_id,
                    labels: old_labels,
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

macro_rules! with_positioned_children {
    ($object:expr, $binding:ident => $body:expr, $fallback:expr) => {
        match $object {
            Object::WorkingSet($binding) => $body,
            Object::DataMask($binding) => $body,
            Object::AlarmMask($binding) => $body,
            Object::Container($binding) => $body,
            Object::Key($binding) => $body,
            Object::Button($binding) => $body,
            Object::AuxiliaryFunctionType1($binding) => $body,
            Object::AuxiliaryInputType1($binding) => $body,
            Object::AuxiliaryFunctionType2($binding) => $body,
            Object::AuxiliaryInputType2($binding) => $body,
            Object::WindowMask($binding) => $body,
            Object::Animation($binding) => $body,
            _ => $fallback,
        }
    };
}

macro_rules! with_macro_references {
    ($object:expr, $binding:ident => $body:expr, $fallback:expr) => {
        match $object {
            Object::WorkingSet($binding) => $body,
            Object::DataMask($binding) => $body,
            Object::AlarmMask($binding) => $body,
            Object::Container($binding) => $body,
            Object::SoftKeyMask($binding) => $body,
            Object::Key($binding) => $body,
            Object::Button($binding) => $body,
            Object::InputBoolean($binding) => $body,
            Object::InputString($binding) => $body,
            Object::InputNumber($binding) => $body,
            Object::InputList($binding) => $body,
            Object::OutputString($binding) => $body,
            Object::OutputNumber($binding) => $body,
            Object::OutputList($binding) => $body,
            Object::OutputLine($binding) => $body,
            Object::OutputRectangle($binding) => $body,
            Object::OutputEllipse($binding) => $body,
            Object::OutputPolygon($binding) => $body,
            Object::OutputMeter($binding) => $body,
            Object::OutputLinearBarGraph($binding) => $body,
            Object::OutputArchedBarGraph($binding) => $body,
            Object::PictureGraphic($binding) => $body,
            Object::FontAttributes($binding) => $body,
            Object::LineAttributes($binding) => $body,
            Object::FillAttributes($binding) => $body,
            Object::InputAttributes($binding) => $body,
            Object::WindowMask($binding) => $body,
            Object::KeyGroup($binding) => $body,
            Object::Animation($binding) => $body,
            Object::ScaledGraphic($binding) => $body,
            _ => $fallback,
        }
    };
}

fn object_refs_mut<'a>(
    pool: &'a mut ObjectPool,
    parent_id: ObjectId,
    operation: &str,
) -> Result<&'a mut Vec<ObjectRef>, OperationError> {
    let object = pool
        .object_mut_by_id(parent_id)
        .ok_or(OperationError::ObjectNotFound(parent_id.value()))?;
    with_positioned_children!(
        object,
        object => Ok(&mut object.object_refs),
        Err(OperationError::PropertyError {
            object_id: parent_id.value(),
            property: operation.to_owned(),
            reason: "This object type does not support positioned children".to_owned(),
        })
    )
}

/// Read-only positioned children for descriptor-driven structural editors.
pub(crate) fn object_refs(object: &Object) -> Option<&[ObjectRef]> {
    with_positioned_children!(object, object => Some(object.object_refs.as_slice()), None)
}

pub(super) fn set_object_refs(object: &mut Object, refs: Vec<ObjectRef>) -> bool {
    with_positioned_children!(
        object,
        object => {
            object.object_refs = refs;
            true
        },
        false
    )
}

/// Clone an object's ordered unpositioned reference list for structural edits.
pub(crate) fn object_list(object: &Object) -> Option<ObjectReferenceList> {
    match object {
        Object::SoftKeyMask(object) => Some(ObjectReferenceList::Required(object.objects.clone())),
        Object::KeyGroup(object) => Some(ObjectReferenceList::Required(object.objects.clone())),
        Object::InputList(object) => Some(ObjectReferenceList::Nullable(object.list_items.clone())),
        Object::OutputList(object) => {
            Some(ObjectReferenceList::Nullable(object.list_items.clone()))
        }
        Object::WindowMask(object) => Some(ObjectReferenceList::Nullable(object.objects.clone())),
        Object::ExternalObjectDefinition(object) => {
            Some(ObjectReferenceList::Nullable(object.objects.clone()))
        }
        _ => None,
    }
}

pub(super) fn set_object_list(object: &mut Object, objects: ObjectReferenceList) -> bool {
    match (object, objects) {
        (Object::SoftKeyMask(object), ObjectReferenceList::Required(objects)) => {
            object.objects = objects
        }
        (Object::KeyGroup(object), ObjectReferenceList::Required(objects)) => {
            object.objects = objects
        }
        (Object::InputList(object), ObjectReferenceList::Nullable(objects)) => {
            object.list_items = objects
        }
        (Object::OutputList(object), ObjectReferenceList::Nullable(objects)) => {
            object.list_items = objects
        }
        (Object::WindowMask(object), ObjectReferenceList::Nullable(objects)) => {
            object.objects = objects
        }
        (Object::ExternalObjectDefinition(object), ObjectReferenceList::Nullable(objects)) => {
            object.objects = objects
        }
        _ => return false,
    }
    true
}

pub(super) fn object_labels(object: &Object) -> Option<&[ObjectLabel]> {
    match object {
        Object::ObjectLabelReferenceList(object) => Some(&object.object_labels),
        _ => None,
    }
}

pub(super) fn set_object_labels(object: &mut Object, labels: Vec<ObjectLabel>) -> bool {
    match object {
        Object::ObjectLabelReferenceList(object) => object.object_labels = labels,
        _ => return false,
    }
    true
}

fn replace_references(
    object: &mut Object,
    old_id: ObjectId,
    new_id: ObjectId,
) -> Result<bool, OperationError> {
    let mut changed = crate::object_properties::replace_object_reference(object, old_id, new_id)
        .map_err(|error| OperationError::PropertyError {
            object_id: object.id().value(),
            property: "object reference".to_owned(),
            reason: format!("{error:?}"),
        })?;

    with_positioned_children!(
        object,
        value => {
            for child in &mut value.object_refs {
                if child.id == old_id {
                    child.id = new_id;
                    changed = true;
                }
            }
        },
        ()
    );

    match object {
        Object::SoftKeyMask(value) => {
            for child in &mut value.objects {
                if *child == old_id {
                    *child = new_id;
                    changed = true;
                }
            }
        }
        Object::KeyGroup(value) => {
            for child in &mut value.objects {
                if *child == old_id {
                    *child = new_id;
                    changed = true;
                }
            }
        }
        Object::InputList(value) => {
            replace_nullable_references(&mut value.list_items, old_id, new_id, &mut changed)
        }
        Object::OutputList(value) => {
            replace_nullable_references(&mut value.list_items, old_id, new_id, &mut changed)
        }
        Object::WindowMask(value) => {
            replace_nullable_references(&mut value.objects, old_id, new_id, &mut changed)
        }
        Object::ExternalObjectDefinition(value) => {
            replace_nullable_references(&mut value.objects, old_id, new_id, &mut changed)
        }
        Object::ObjectLabelReferenceList(value) => {
            for label in &mut value.object_labels {
                if label.id == old_id {
                    label.id = new_id;
                    changed = true;
                }
                for reference in [
                    &mut label.string_variable_reference,
                    &mut label.graphic_representation,
                ] {
                    if reference.0 == Some(old_id) {
                        reference.0 = Some(new_id);
                        changed = true;
                    }
                }
            }
        }
        _ => {}
    }

    if old_id.value() <= u8::MAX.into() {
        let owner_id = object.id().value();
        with_macro_references!(
            object,
            value => {
                for macro_ref in &mut value.macro_refs {
                    if u16::from(macro_ref.macro_id) == old_id.value() {
                        macro_ref.macro_id = u8::try_from(new_id.value()).map_err(|_| {
                            OperationError::PropertyError {
                                object_id: owner_id,
                                property: "macro_refs".to_owned(),
                                reason: format!(
                                    "Macro object ID {} cannot be represented in a macro reference",
                                    new_id.value()
                                ),
                            }
                        })?;
                        changed = true;
                    }
                }
            },
            ()
        );
    }

    Ok(changed)
}

fn replace_nullable_references(
    references: &mut [NullableObjectId],
    old_id: ObjectId,
    new_id: ObjectId,
    changed: &mut bool,
) {
    for reference in references {
        if reference.0 == Some(old_id) {
            reference.0 = Some(new_id);
            *changed = true;
        }
    }
}

pub(crate) fn macro_refs(object: &Object) -> Option<&[MacroRef]> {
    with_macro_references!(
        object,
        object => Some(object.macro_refs.as_slice()),
        None
    )
}

pub(super) fn set_macro_refs(object: &mut Object, refs: Vec<MacroRef>) -> bool {
    with_macro_references!(
        object,
        object => {
            object.macro_refs = refs;
            true
        },
        false
    )
}

fn macro_refs_mut<'a>(
    pool: &'a mut ObjectPool,
    object_id: ObjectId,
    operation: &str,
) -> Result<&'a mut Vec<MacroRef>, OperationError> {
    let object = pool
        .object_mut_by_id(object_id)
        .ok_or(OperationError::ObjectNotFound(object_id.value()))?;
    with_macro_references!(
        object,
        object => Ok(&mut object.macro_refs),
        Err(OperationError::PropertyError {
            object_id: object_id.value(),
            property: operation.to_owned(),
            reason: "This object type does not support macro references".to_owned(),
        })
    )
}
