//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use ag_iso_stack::object_pool::object::Object;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    /// A unique identifier for each object.
    /// Note that we can't use the object_id itself, as that can be changed and therefore is not unique for the object.
    unique_id: Uuid,

    /// Optional name for the object.
    /// This is used to give the object a name throughout the editor that is more human-readable
    name: Option<String>,
}

impl ObjectInfo {
    pub fn new(object: &Object) -> Self {
        ObjectInfo {
            unique_id: Uuid::new_v4(),
            name: None,
        }
    }

    /// Get the name of the object.
    /// If the object has no name, a default name is generated.
    /// Default Format: "{object_id}: {object_type}"
    pub fn get_name(&self, object: &Object) -> String {
        if let Some(ref n) = self.name {
            n.clone()
        } else {
            format!("{:?}: {:?}", u16::from(object.id()), object.object_type())
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
}

impl PartialEq for ObjectInfo {
    fn eq(&self, other: &Self) -> bool {
        self.get_unique_id() == other.get_unique_id()
    }
}
