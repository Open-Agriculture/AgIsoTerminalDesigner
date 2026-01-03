//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

//! Helper functions and macros for queueing object updates using the new UpdateQueue system

use crate::EditorProject;
use ag_iso_stack::object_pool::{object::Object, ObjectId};

/// Helper trait to provide convenient update queueing methods for common parameter types
/// This extends EditorProject with type-safe update methods
pub trait UpdateHelpers {
    /// Queue an update for a u8 field (like background_colour)
    fn queue_u8_update<F>(&self, object_id: ObjectId, field_name: &'static str, getter: F, new_value: u8)
    where
        F: Fn(&Object) -> Option<u8> + Send + 'static;

    /// Queue an update for a u16 field (like width, height)
    fn queue_u16_update<F>(&self, object_id: ObjectId, field_name: &'static str, getter: F, new_value: u16)
    where
        F: Fn(&Object) -> Option<u16> + Send + 'static;

    /// Queue an update for a bool field (like selectable, enabled)
    fn queue_bool_update<F>(&self, object_id: ObjectId, field_name: &'static str, getter: F, new_value: bool)
    where
        F: Fn(&Object) -> Option<bool> + Send + 'static;
}

impl UpdateHelpers for EditorProject {
    fn queue_u8_update<F>(&self, object_id: ObjectId, _field_name: &'static str, _getter: F, new_value: u8)
    where
        F: Fn(&Object) -> Option<u8> + Send + 'static,
    {
        // For now, this is a placeholder showing the pattern
        // In a full implementation, this would use the getter to extract and set the field
        self.queue_update(object_id, move |obj| {
            // Example: if we're updating background_colour for a WorkingSet
            if let Object::WorkingSet(ws) = obj {
                ws.background_colour = new_value;
            }
            // Add other object types as needed...
        });
    }

    fn queue_u16_update<F>(&self, object_id: ObjectId, _field_name: &'static str, _getter: F, new_value: u16)
    where
        F: Fn(&Object) -> Option<u16> + Send + 'static,
    {
        self.queue_update(object_id, move |obj| {
            // Example: if we're updating width
            if let Some(sized) = obj.as_mut_sized_object() {
                sized.set_width(new_value);
            }
        });
    }

    fn queue_bool_update<F>(&self, object_id: ObjectId, _field_name: &'static str, _getter: F, new_value: bool)
    where
        F: Fn(&Object) -> Option<bool> + Send + 'static,
    {
        self.queue_update(object_id, move |obj| {
            // Example: if we're updating selectable for a WorkingSet
            if let Object::WorkingSet(ws) = obj {
                ws.selectable = new_value;
            }
            // Add other object types as needed...
        });
    }
}

/// Demonstration: Example of how to migrate from direct mutation to queued updates
/// 
/// OLD WAY (current implementation):
/// ```ignore
/// let mut obj = pool.get_mut_pool().borrow_mut().object_mut_by_id(id).unwrap();
/// if let Object::WorkingSet(ws) = obj {
///     ws.background_colour = 128;
/// }
/// ```
///
/// NEW WAY (using UpdateQueue):
/// ```ignore
/// pool.queue_update(object_id, |obj| {
///     if let Object::WorkingSet(ws) = obj {
///         ws.background_colour = 128;
///     }
/// });
/// ```
///
/// The new way:
/// - Avoids holding a mutable borrow across UI code
/// - Allows multiple updates from different parts of the UI
/// - All updates are applied together at frame end
/// - Creates a single undo entry for all changes in a frame
#[allow(dead_code)]
fn example_migration() {
    // This is just documentation - showing the migration pattern
}

/// Example of converting a render_parameters function to use the update queue
/// This shows how existing UI code can be gradually migrated
#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::ObjectPool;
    
    #[test]
    fn test_update_queue_basic() {
        let pool = ObjectPool::new();
        let project = EditorProject::from(pool);
        
        // The update queue starts empty
        assert!(!project.has_pending_updates());
        
        // We can queue multiple updates
        let test_id = project.allocate_object_id();
        project.queue_update(test_id, |_obj| {
            // Update code here
        });
        
        // Now we have pending updates
        assert!(project.has_pending_updates());
    }
}
