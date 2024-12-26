//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::{cell::RefCell, collections::HashMap};

use ag_iso_stack::object_pool::{object::Object, NullableObjectId, ObjectId, ObjectPool};

use crate::ObjectInfo;

const MAX_UNDO_REDO_POOL: usize = 10;
const MAX_UNDO_REDO_SELECTED: usize = 20;

#[derive(Default, Clone)]
pub struct EditorProject {
    pool: ObjectPool,
    mut_pool: RefCell<ObjectPool>,
    undo_pool_history: Vec<ObjectPool>,
    redo_pool_history: Vec<ObjectPool>,
    selected_object: NullableObjectId,
    mut_selected_object: RefCell<NullableObjectId>,
    undo_selected_history: Vec<NullableObjectId>,
    redo_selected_history: Vec<NullableObjectId>,
    pub mask_size: u16,
    soft_key_size: (u16, u16),
    object_info: RefCell<HashMap<ObjectId, ObjectInfo>>,

    /// Used to keep track of the object that is being renamed
    renaming_object: RefCell<Option<(eframe::egui::Id, ObjectId, String)>>,
}

impl From<ObjectPool> for EditorProject {
    fn from(pool: ObjectPool) -> Self {
        let (mask_size, soft_key_size) = pool.get_minimum_mask_sizes();
        EditorProject {
            mut_pool: RefCell::new(pool.clone()),
            pool,
            undo_pool_history: Default::default(),
            redo_pool_history: Default::default(),
            selected_object: NullableObjectId::default(),
            mut_selected_object: RefCell::new(NullableObjectId::default()),
            undo_selected_history: Default::default(),
            redo_selected_history: Default::default(),
            mask_size,
            soft_key_size,
            object_info: RefCell::new(HashMap::new()),
            renaming_object: RefCell::new(None),
        }
    }
}

impl EditorProject {
    /// Get the current object pool
    pub fn get_pool(&self) -> &ObjectPool {
        &self.pool
    }

    /// Get the current selected object
    pub fn get_selected(&self) -> NullableObjectId {
        self.selected_object
    }

    /// Get the current mutating object pool
    /// This is used to make changes to the pool in the next frame
    /// without affecting the current pool
    pub fn get_mut_pool(&self) -> &RefCell<ObjectPool> {
        &self.mut_pool
    }

    /// Set the mutating selected object
    /// This is used to make changes to the selected object in the next frame
    /// without affecting the current selected object
    pub fn get_mut_selected(&self) -> &RefCell<NullableObjectId> {
        &self.mut_selected_object
    }

    /// If the mutating pool is different from the current pool, add the current pool to the history
    /// and update the current pool with the mutated pool.
    /// Returns true if the pool was updated
    pub fn update_pool(&mut self) -> bool {
        if self.mut_pool.borrow().to_owned() != self.pool {
            self.redo_pool_history.clear();
            self.undo_pool_history.push(self.pool.clone());
            if self.undo_pool_history.len() > MAX_UNDO_REDO_POOL {
                self.undo_pool_history
                    .drain(..self.undo_pool_history.len() - MAX_UNDO_REDO_POOL);
            }
            self.pool = self.mut_pool.borrow().clone();
            return true;
        }
        false
    }

    /// Undo the last action
    pub fn undo(&mut self) {
        if let Some(pool) = self.undo_pool_history.pop() {
            self.redo_pool_history.push(self.pool.clone());

            // Both need to be replaced here because otherwise it will be added to the undo history
            self.pool = pool.clone();
            self.mut_pool.replace(pool);
        }
    }

    /// Check if there are actions available to undo
    pub fn undo_available(&self) -> bool {
        !self.undo_pool_history.is_empty()
    }

    /// Redo the last undone action
    pub fn redo(&mut self) {
        if let Some(pool) = self.redo_pool_history.pop() {
            self.undo_pool_history.push(self.pool.clone());
            // Both need to be replaced here because otherwise the redo history will be cleared
            self.pool = pool.clone();
            self.mut_pool.replace(pool);
        }
    }

    /// Check if there are actions available to redo
    pub fn redo_available(&self) -> bool {
        !self.redo_pool_history.is_empty()
    }

    /// Update the selected object with the mutating selected object if it is different
    /// Returns true if the selected object was updated
    pub fn update_selected(&mut self) -> bool {
        if self.mut_selected_object.borrow().to_owned() != self.selected_object {
            self.redo_selected_history.clear();
            if self.mut_selected_object.borrow().to_owned() != NullableObjectId::NULL {
                self.undo_selected_history.push(self.selected_object);
                if self.undo_selected_history.len() > MAX_UNDO_REDO_SELECTED {
                    self.undo_selected_history
                        .drain(..self.undo_selected_history.len() - MAX_UNDO_REDO_SELECTED);
                }
            }
            self.selected_object = self.mut_selected_object.borrow().clone();
            return true;
        }
        false
    }

    /// Set the selected object to the previous object in the history
    pub fn set_previous_selected(&mut self) {
        if let Some(selected) = self.undo_selected_history.pop() {
            self.redo_selected_history.push(self.selected_object);
            // Both need to be replaced here because otherwise it will be added to the undo history
            self.selected_object = selected.clone();
            self.mut_selected_object.replace(selected);
        }
    }

    /// Set the selected object to the next object in the history
    pub fn set_next_selected(&mut self) {
        if let Some(selected) = self.redo_selected_history.pop() {
            self.undo_selected_history.push(self.selected_object);
            // Both need to be replaced here because otherwise the redo history will be cleared
            self.selected_object = selected.clone();
            self.mut_selected_object.replace(selected);
        }
    }

    /// Change an object id in the object info hashmap
    pub fn update_object_id_for_info(&self, old_id: ObjectId, new_id: ObjectId) {
        let mut object_info = self.object_info.borrow_mut();
        if let Some(info) = object_info.remove(&old_id) {
            object_info.insert(new_id, info);
        }
    }

    /// Get the object info for an object id
    /// If the object id is not mapped, we insert the default object info
    pub fn get_object_info(&self, object: &Object) -> ObjectInfo {
        let mut object_info = self.object_info.borrow_mut();
        object_info
            .entry(object.id())
            .or_insert_with(|| ObjectInfo::new(object))
            .clone()
    }

    /// Start renaming an object
    pub fn set_renaming_object(&self, ui_id: eframe::egui::Id, object_id: ObjectId, name: String) {
        self.renaming_object.replace(Some((ui_id, object_id, name)));
    }

    /// Get the current name of the object that is being renamed
    /// Returns None if no object is being renamed
    pub fn get_renaming_object(&self) -> Option<(eframe::egui::Id, ObjectId, String)> {
        self.renaming_object.borrow().clone()
    }

    /// Finish renaming an object
    /// If store is true, we store the new name in the object info hashmap
    pub fn finish_renaming_object(&self, store: bool) {
        if store {
            if let Some(renaming_object) = self.renaming_object.borrow().as_ref() {
                let mut object_info = self.object_info.borrow_mut();
                if let Some(info) = object_info.get_mut(&renaming_object.1) {
                    info.set_name(renaming_object.2.clone());
                }
            }
        }
        self.renaming_object.replace(None);
    }
}
