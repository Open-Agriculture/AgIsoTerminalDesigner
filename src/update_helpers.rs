//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

//! Structured update helpers that provide descriptive updates with parameter information

use crate::EditorProject;
use ag_iso_stack::object_pool::{object::Object, ObjectId};

/// Describes a specific parameter update with metadata for logging/debugging
#[derive(Debug, Clone)]
pub struct ParameterUpdate {
    /// The object being updated
    pub object_id: ObjectId,
    /// Human-readable field name
    pub field_name: String,
    /// Description of the old value
    pub old_value: String,
    /// Description of the new value
    pub new_value: String,
}

impl ParameterUpdate {
    /// Generate a human-readable description of this update
    pub fn description(&self) -> String {
        format!(
            "Object {}: {} changed from {} to {}",
            self.object_id.value(),
            self.field_name,
            self.old_value,
            self.new_value
        )
    }
}

/// Helper trait to provide convenient update queueing methods with descriptions
/// This is the required format for all object updates
pub trait UpdateHelpers {
    /// Queue an update for a u8 field (like background_colour)
    fn queue_u8_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: u8,
        new_value: u8,
        setter: impl FnOnce(&mut Object, u8) + Send + 'static,
    ) -> ParameterUpdate;

    /// Queue an update for a u16 field (like width, height)
    fn queue_u16_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: u16,
        new_value: u16,
        setter: impl FnOnce(&mut Object, u16) + Send + 'static,
    ) -> ParameterUpdate;

    /// Queue an update for a bool field (like selectable, enabled)
    fn queue_bool_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: bool,
        new_value: bool,
        setter: impl FnOnce(&mut Object, bool) + Send + 'static,
    ) -> ParameterUpdate;

    /// Queue an update for a String field
    fn queue_string_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: String,
        new_value: String,
        setter: impl FnOnce(&mut Object, String) + Send + 'static,
    ) -> ParameterUpdate;

    /// Queue an update for a u32 field (like value in NumberVariable)
    fn queue_u32_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: u32,
        new_value: u32,
        setter: impl FnOnce(&mut Object, u32) + Send + 'static,
    ) -> ParameterUpdate;
}

impl UpdateHelpers for EditorProject {
    fn queue_u8_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: u8,
        new_value: u8,
        setter: impl FnOnce(&mut Object, u8) + Send + 'static,
    ) -> ParameterUpdate {
        let update_desc = ParameterUpdate {
            object_id,
            field_name: field_name.to_string(),
            old_value: old_value.to_string(),
            new_value: new_value.to_string(),
        };

        log::debug!("{}", update_desc.description());

        self.queue_update(object_id, move |obj| {
            setter(obj, new_value);
        });

        update_desc
    }

    fn queue_u16_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: u16,
        new_value: u16,
        setter: impl FnOnce(&mut Object, u16) + Send + 'static,
    ) -> ParameterUpdate {
        let update_desc = ParameterUpdate {
            object_id,
            field_name: field_name.to_string(),
            old_value: old_value.to_string(),
            new_value: new_value.to_string(),
        };

        log::debug!("{}", update_desc.description());

        self.queue_update(object_id, move |obj| {
            setter(obj, new_value);
        });

        update_desc
    }

    fn queue_bool_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: bool,
        new_value: bool,
        setter: impl FnOnce(&mut Object, bool) + Send + 'static,
    ) -> ParameterUpdate {
        let update_desc = ParameterUpdate {
            object_id,
            field_name: field_name.to_string(),
            old_value: old_value.to_string(),
            new_value: new_value.to_string(),
        };

        log::debug!("{}", update_desc.description());

        self.queue_update(object_id, move |obj| {
            setter(obj, new_value);
        });

        update_desc
    }

    fn queue_string_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: String,
        new_value: String,
        setter: impl FnOnce(&mut Object, String) + Send + 'static,
    ) -> ParameterUpdate {
        let update_desc = ParameterUpdate {
            object_id,
            field_name: field_name.to_string(),
            old_value: format!("\"{}\"", old_value),
            new_value: format!("\"{}\"", new_value.clone()),
        };

        log::debug!("{}", update_desc.description());

        self.queue_update(object_id, move |obj| {
            setter(obj, new_value);
        });

        update_desc
    }

    fn queue_u32_update(
        &self,
        object_id: ObjectId,
        field_name: &str,
        old_value: u32,
        new_value: u32,
        setter: impl FnOnce(&mut Object, u32) + Send + 'static,
    ) -> ParameterUpdate {
        let update_desc = ParameterUpdate {
            object_id,
            field_name: field_name.to_string(),
            old_value: old_value.to_string(),
            new_value: new_value.to_string(),
        };

        log::debug!("{}", update_desc.description());

        self.queue_update(object_id, move |obj| {
            setter(obj, new_value);
        });

        update_desc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{object::NumberVariable, ObjectPool};

    #[test]
    fn test_parameter_update_description() {
        let update = ParameterUpdate {
            object_id: ObjectId::new(1).unwrap(),
            field_name: "value".to_string(),
            old_value: "42".to_string(),
            new_value: "100".to_string(),
        };

        assert_eq!(
            update.description(),
            "Object 1: value changed from 42 to 100"
        );
    }

    #[test]
    fn test_typed_update_helpers() {
        let pool = ObjectPool::new();
        let project = EditorProject::from(pool);

        let obj_id = project.allocate_object_id();

        // Test u8 update
        let desc = project.queue_u8_update(
            obj_id,
            "background_colour",
            0,
            255,
            |obj, val| {
                if let Object::WorkingSet(ws) = obj {
                    ws.background_colour = val;
                }
            },
        );

        assert_eq!(desc.field_name, "background_colour");
        assert_eq!(desc.old_value, "0");
        assert_eq!(desc.new_value, "255");

        // Test bool update
        let desc2 = project.queue_bool_update(
            obj_id,
            "selectable",
            false,
            true,
            |obj, val| {
                if let Object::WorkingSet(ws) = obj {
                    ws.selectable = val;
                }
            },
        );

        assert_eq!(desc2.field_name, "selectable");
        assert_eq!(desc2.old_value, "false");
        assert_eq!(desc2.new_value, "true");

        // Verify updates were queued
        assert!(project.has_pending_updates());
    }
}
