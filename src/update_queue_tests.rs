//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

//! Integration tests for the UpdateQueue system

#[cfg(test)]
mod tests {
    use crate::EditorProject;
    use ag_iso_stack::object_pool::{
        object::{NumberVariable, Object, StringVariable},
        ObjectId, ObjectPool,
    };

    #[test]
    fn test_number_variable_update_queue() {
        // Create a pool with a number variable
        let mut pool = ObjectPool::new();
        let nv = NumberVariable::new(ObjectId::new(1).unwrap(), 42);
        pool.add(Object::NumberVariable(nv));

        // Create editor project
        let mut project = EditorProject::from(pool);

        // Get the object ID
        let obj_id = ObjectId::new(1).unwrap();

        // Queue an update to change the value
        project.queue_update(obj_id, |obj| {
            if let Object::NumberVariable(nv) = obj {
                nv.value = 100;
            }
        });

        // Verify the update is queued
        assert!(project.has_pending_updates());

        // Apply the update
        assert!(project.update_pool());

        // Verify the value was changed
        if let Some(Object::NumberVariable(nv)) = project.get_pool().object_by_id(obj_id) {
            assert_eq!(nv.value, 100);
        } else {
            panic!("Object not found or wrong type");
        }

        // Verify undo is available
        assert!(project.undo_available());
    }

    #[test]
    fn test_string_variable_update_queue() {
        // Create a pool with a string variable
        let mut pool = ObjectPool::new();
        let sv = StringVariable::new(ObjectId::new(2).unwrap(), "initial".to_string());
        pool.add(Object::StringVariable(sv));

        // Create editor project
        let mut project = EditorProject::from(pool);

        // Get the object ID
        let obj_id = ObjectId::new(2).unwrap();

        // Queue an update to change the value
        let new_value = "updated".to_string();
        project.queue_update(obj_id, move |obj| {
            if let Object::StringVariable(sv) = obj {
                sv.value = new_value;
            }
        });

        // Verify the update is queued
        assert!(project.has_pending_updates());

        // Apply the update
        assert!(project.update_pool());

        // Verify the value was changed
        if let Some(Object::StringVariable(sv)) = project.get_pool().object_by_id(obj_id) {
            assert_eq!(sv.value, "updated");
        } else {
            panic!("Object not found or wrong type");
        }
    }

    #[test]
    fn test_multiple_updates_same_frame() {
        // Create a pool with multiple variables
        let mut pool = ObjectPool::new();
        pool.add(Object::NumberVariable(NumberVariable::new(
            ObjectId::new(1).unwrap(),
            10,
        )));
        pool.add(Object::NumberVariable(NumberVariable::new(
            ObjectId::new(2).unwrap(),
            20,
        )));
        pool.add(Object::NumberVariable(NumberVariable::new(
            ObjectId::new(3).unwrap(),
            30,
        )));

        // Create editor project
        let mut project = EditorProject::from(pool);

        // Queue multiple updates
        project.queue_update(ObjectId::new(1).unwrap(), |obj| {
            if let Object::NumberVariable(nv) = obj {
                nv.value = 100;
            }
        });

        project.queue_update(ObjectId::new(2).unwrap(), |obj| {
            if let Object::NumberVariable(nv) = obj {
                nv.value = 200;
            }
        });

        project.queue_update(ObjectId::new(3).unwrap(), |obj| {
            if let Object::NumberVariable(nv) = obj {
                nv.value = 300;
            }
        });

        // All three updates should be pending
        assert!(project.has_pending_updates());

        // Apply all updates at once
        assert!(project.update_pool());

        // Verify all values were changed
        let pool = project.get_pool();
        if let Some(Object::NumberVariable(nv)) = pool.object_by_id(ObjectId::new(1).unwrap()) {
            assert_eq!(nv.value, 100);
        }
        if let Some(Object::NumberVariable(nv)) = pool.object_by_id(ObjectId::new(2).unwrap()) {
            assert_eq!(nv.value, 200);
        }
        if let Some(Object::NumberVariable(nv)) = pool.object_by_id(ObjectId::new(3).unwrap()) {
            assert_eq!(nv.value, 300);
        }

        // Only one undo entry should be created (for all three changes)
        assert!(project.undo_available());
    }

    #[test]
    fn test_undo_after_queued_updates() {
        // Create a pool with a number variable
        let mut pool = ObjectPool::new();
        let nv = NumberVariable::new(ObjectId::new(1).unwrap(), 42);
        pool.add(Object::NumberVariable(nv));

        // Create editor project
        let mut project = EditorProject::from(pool);
        let obj_id = ObjectId::new(1).unwrap();

        // Queue and apply first update
        project.queue_update(obj_id, |obj| {
            if let Object::NumberVariable(nv) = obj {
                nv.value = 100;
            }
        });
        project.update_pool();

        // Queue and apply second update
        project.queue_update(obj_id, |obj| {
            if let Object::NumberVariable(nv) = obj {
                nv.value = 200;
            }
        });
        project.update_pool();

        // Verify current value is 200
        if let Some(Object::NumberVariable(nv)) = project.get_pool().object_by_id(obj_id) {
            assert_eq!(nv.value, 200);
        }

        // Undo should go back to 100
        project.undo();
        if let Some(Object::NumberVariable(nv)) = project.get_pool().object_by_id(obj_id) {
            assert_eq!(nv.value, 100);
        }

        // Undo again should go back to 42
        project.undo();
        if let Some(Object::NumberVariable(nv)) = project.get_pool().object_by_id(obj_id) {
            assert_eq!(nv.value, 42);
        }

        // Redo should go forward to 100
        project.redo();
        if let Some(Object::NumberVariable(nv)) = project.get_pool().object_by_id(obj_id) {
            assert_eq!(nv.value, 100);
        }
    }

    #[test]
    fn test_no_update_when_queue_empty() {
        // Create a pool
        let pool = ObjectPool::new();
        let mut project = EditorProject::from(pool);

        // Don't queue any updates
        assert!(!project.has_pending_updates());

        // update_pool should return false (no changes)
        assert!(!project.update_pool());

        // No undo should be available
        assert!(!project.undo_available());
    }
}
