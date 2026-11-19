//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::ObjectId;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    /// A unique identifier for each object.
    /// Note that we can't use the object_id itself, as that can be changed and therefore is not unique for the object.
    unique_id: Uuid,

    /// Optional name for the object.
    /// This is used to give the object a name throughout the editor that is more human-readable
    pub name: Option<String>,

    /// Optional semantic description used by graphics discovery.
    pub description: Option<String>,

    /// Optional authoring notes that are persisted with the project.
    pub notes: Option<String>,

    /// Searchable semantic tags used by graphics discovery.
    pub tags: Vec<String>,

    /// Ordered editor-only targets for an ObjectPointer. Empty means unrestricted.
    pub allowed_references: Vec<ObjectId>,

    /// True when the user explicitly assigned this object's current ID.
    /// This is authoring metadata and is not serialized into the object pool.
    pub id_overridden: bool,
}

impl ObjectInfo {
    pub fn new(_object: &Object) -> Self {
        ObjectInfo {
            unique_id: Uuid::new_v4(),
            name: None,
            description: None,
            notes: None,
            tags: vec![],
            allowed_references: vec![],
            id_overridden: false,
        }
    }

    /// Return the authoring name, falling back to the object type for metadata
    /// that has not yet received a generated name. Names never depend on IDs.
    pub fn get_name(&self, object: &Object) -> String {
        if let Some(ref n) = self.name {
            n.clone()
        } else {
            crate::smart_naming::get_object_type_name(object.object_type()).to_owned()
        }
    }

    /// Set the name of the object.
    pub fn set_name(&mut self, name: String) {
        if !name.is_empty() {
            self.name = Some(name);
        }
    }

    pub fn get_unique_id(&self) -> Uuid {
        self.unique_id
    }

    pub(crate) fn content_eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.description == other.description
            && self.notes == other.notes
            && self.tags == other.tags
            && self.allowed_references == other.allowed_references
            && self.id_overridden == other.id_overridden
    }
}

impl PartialEq for ObjectInfo {
    fn eq(&self, other: &Self) -> bool {
        self.get_unique_id() == other.get_unique_id()
    }
}
