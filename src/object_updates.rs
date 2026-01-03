//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

//! This module provides a deferred update mechanism for object pool modifications.
//! Instead of directly mutating objects and cloning the entire pool, updates are queued
//! and applied together at the end of the UI frame, reducing memory usage and improving undo/redo.

use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use std::cell::RefCell;

/// A queued update to apply to an object in the pool
/// This is a simple wrapper around a closure that modifies an object
pub struct ObjectUpdate {
    /// The ID of the object to update
    pub object_id: ObjectId,
    /// The update function to apply
    update_fn: Box<dyn FnOnce(&mut Object) + Send>,
}

impl ObjectUpdate {
    /// Create a new object update
    pub fn new<F>(object_id: ObjectId, update_fn: F) -> Self
    where
        F: FnOnce(&mut Object) + Send + 'static,
    {
        ObjectUpdate {
            object_id,
            update_fn: Box::new(update_fn),
        }
    }

    /// Apply this update to the object in the pool
    pub fn apply(self, pool: &mut ObjectPool) -> Result<(), String> {
        if let Some(obj) = pool.object_mut_by_id(self.object_id) {
            (self.update_fn)(obj);
            Ok(())
        } else {
            Err(format!("Object {:?} not found in pool", self.object_id))
        }
    }
}

impl std::fmt::Debug for ObjectUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectUpdate")
            .field("object_id", &self.object_id)
            .field("update_fn", &"<closure>")
            .finish()
    }
}

/// A queue of pending object updates
#[derive(Default)]
pub struct UpdateQueue {
    updates: RefCell<Vec<ObjectUpdate>>,
}

impl UpdateQueue {
    /// Create a new empty update queue
    pub fn new() -> Self {
        Self {
            updates: RefCell::new(Vec::new()),
        }
    }

    /// Queue an update to be applied later
    pub fn queue<F>(&self, object_id: ObjectId, update_fn: F)
    where
        F: FnOnce(&mut Object) + Send + 'static,
    {
        self.updates
            .borrow_mut()
            .push(ObjectUpdate::new(object_id, update_fn));
    }

    /// Check if there are any pending updates
    pub fn has_updates(&self) -> bool {
        !self.updates.borrow().is_empty()
    }

    /// Get the number of pending updates
    pub fn len(&self) -> usize {
        self.updates.borrow().len()
    }

    /// Apply all queued updates to the pool and clear the queue
    pub fn apply_all(&self, pool: &mut ObjectPool) -> Result<usize, Vec<String>> {
        let mut updates = self.updates.borrow_mut();
        let count = updates.len();
        
        if count == 0 {
            return Ok(0);
        }

        let mut errors = Vec::new();
        
        // Apply all updates
        for update in updates.drain(..) {
            if let Err(e) = update.apply(pool) {
                errors.push(e);
            }
        }

        if errors.is_empty() {
            Ok(count)
        } else {
            Err(errors)
        }
    }

    /// Clear all pending updates without applying them
    pub fn clear(&self) {
        self.updates.borrow_mut().clear();
    }
}

impl std::fmt::Debug for UpdateQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateQueue")
            .field("count", &self.len())
            .finish()
    }
}
