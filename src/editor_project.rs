//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::{cell::RefCell, collections::HashMap};

use ag_iso_stack::object_pool::{
    object::Object, NullableObjectId, ObjectId, ObjectPool, ObjectType,
};

use crate::{project_file::ProjectFile, smart_naming, ObjectInfo};

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
    pub object_info: RefCell<HashMap<ObjectId, ObjectInfo>>,

    /// Used to keep track of the object that is being renamed
    renaming_object: RefCell<Option<(eframe::egui::Id, ObjectId, String)>>,

    /// Cached next available ID for efficient allocation
    next_available_id: RefCell<u16>,

    /// Cached default object names for efficient lookup
    default_object_names: RefCell<HashMap<ObjectId, String>>,

    /// Request to open image file dialog for PictureGraphic object
    image_load_request: RefCell<Option<ObjectId>>,
}

impl From<ObjectPool> for EditorProject {
    fn from(pool: ObjectPool) -> Self {
        let (mask_size, soft_key_size) = pool.get_minimum_mask_sizes();

        // Find the highest ID in use to initialize next_available_id
        let max_id = pool
            .objects()
            .iter()
            .map(|obj| obj.id().value())
            .max()
            .unwrap_or(0);

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
            next_available_id: RefCell::new(max_id.saturating_add(1)),
            default_object_names: RefCell::new(HashMap::new()),
            image_load_request: RefCell::new(None),
        }
    }
}

impl EditorProject {
    /// Get the current object pool
    pub fn get_pool(&self) -> &ObjectPool {
        &self.pool
    }

    /// Allocate a new unique object ID efficiently
    pub fn allocate_object_id(&self) -> ObjectId {
        let mut next_id = self.next_available_id.borrow_mut();

        // Find the next available ID starting from our cached value
        while self
            .pool
            .object_by_id(ObjectId::new(*next_id).unwrap_or_default())
            .is_some()
        {
            *next_id = next_id.saturating_add(1);

            if *next_id == u16::MAX {
                // If we've reached the "NULL" object ID, do a full scan to find any gaps
                let mut found = false;
                for id in 1..=u16::MAX {
                    if self
                        .pool
                        .object_by_id(ObjectId::new(id).unwrap_or_default())
                        .is_none()
                    {
                        *next_id = id;
                        found = true;
                        break;
                    }
                }
                if !found {
                    panic!("No available ObjectId: all IDs from 1 to u16::MAX are taken.");
                }
                break;
            }
        }

        let allocated_id = ObjectId::new(*next_id).unwrap_or_default();
        *next_id = next_id.saturating_add(1);
        allocated_id
    }

    /// Update the next available ID cache based on the current pool
    fn update_next_available_id(&self) {
        let max_id = self
            .pool
            .objects()
            .iter()
            .map(|obj| obj.id().value())
            .max()
            .unwrap_or(0);
        self.next_available_id.replace(max_id.saturating_add(1));
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
            // Clear the default names cache since objects may have changed
            self.default_object_names.borrow_mut().clear();
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

            // Update next_available_id based on the new pool state
            self.update_next_available_id();

            // Clear the default names cache since objects may have changed
            self.default_object_names.borrow_mut().clear();
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

            // Update next_available_id based on the new pool state
            self.update_next_available_id();

            // Clear the default names cache since objects may have changed
            self.default_object_names.borrow_mut().clear();
        }
    }

    /// Check if there are actions available to redo
    pub fn redo_available(&self) -> bool {
        !self.redo_pool_history.is_empty()
    }

    /// Update the selected object with the mutating selected object if it is different
    /// Returns true if the selected object was updated
    pub fn update_selected(&mut self) -> bool {
        let mut_selected = self.mut_selected_object.borrow().to_owned();
        if mut_selected != self.selected_object {
            self.redo_selected_history.clear();
            if mut_selected != NullableObjectId::NULL {
                self.undo_selected_history.push(self.selected_object);
                if self.undo_selected_history.len() > MAX_UNDO_REDO_SELECTED {
                    self.undo_selected_history
                        .drain(..self.undo_selected_history.len() - MAX_UNDO_REDO_SELECTED);
                }
            }
            self.selected_object = mut_selected;
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

    pub fn sort_objects_by<F>(&mut self, cmp: F)
    where
        F: Fn(&Object, &Object) -> std::cmp::Ordering,
    {
        self.mut_pool.borrow_mut().objects_mut().sort_by(cmp);
    }

    /// Get all existing object names for validation
    pub fn get_all_object_names(&self) -> HashMap<String, ObjectType> {
        let mut names = HashMap::new();
        let object_info = self.object_info.borrow();
        let mut default_names_cache = self.default_object_names.borrow_mut();

        for object in self.pool.objects() {
            let name = if let Some(info) = object_info.get(&object.id()) {
                info.get_name(object)
            } else {
                // Use cached default name if available, otherwise generate and cache it
                default_names_cache
                    .entry(object.id())
                    .or_insert_with(|| {
                        format!(
                            "Object {} ({})",
                            object.id().value(),
                            smart_naming::get_object_type_name(object.object_type())
                        )
                    })
                    .clone()
            };
            names.entry(name).or_insert(object.object_type());
        }
        names
    }

    /// Generate a smart default name for a new object
    pub fn generate_smart_name_for_new_object(&self, object_type: ObjectType) -> String {
        let existing_names = self.get_all_object_names();
        smart_naming::generate_smart_default_name(object_type, &existing_names)
    }

    /// Apply smart naming to all objects efficiently
    pub fn apply_smart_naming_to_all_objects(&self) {
        let mut object_info = self.object_info.borrow_mut();

        // Build existing names map once for all remaining objects
        let mut existing_names = HashMap::new();
        for (id, info) in object_info.iter() {
            if let Some(name) = &info.name {
                if let Some(object) = self.pool.object_by_id(*id) {
                    existing_names
                        .entry(name.clone())
                        .or_insert(object.object_type());
                }
            }
        }

        // Generate names for remaining objects
        for object in self.pool.objects() {
            let new_name =
                smart_naming::generate_smart_default_name(object.object_type(), &existing_names);

            // Update the count for the new name to ensure uniqueness
            existing_names
                .entry(new_name.clone())
                .or_insert(object.object_type());

            let info = object_info
                .entry(object.id())
                .or_insert_with(|| ObjectInfo::new(object));
            info.set_name(new_name);
        }
    }

    /// Apply smart naming to an existing object if it doesn't have a custom name
    pub fn apply_smart_naming_to_object(&self, object: &Object) {
        let mut object_info = self.object_info.borrow_mut();

        // Check if the object already has a name
        if let Some(info) = object_info.get(&object.id()) {
            if info.name.is_some() {
                return; // Already has a custom name
            }
        }

        // Build names map inline to avoid extra iteration
        let mut existing_names = HashMap::new();
        let mut default_names_cache = self.default_object_names.borrow_mut();
        for obj in self.pool.objects() {
            let name = if let Some(info) = object_info.get(&obj.id()) {
                info.get_name(obj)
            } else if obj.id() == object.id() {
                continue; // Skip the object we're naming
            } else {
                default_names_cache
                    .entry(obj.id())
                    .or_insert_with(|| {
                        format!(
                            "Object {} ({})",
                            obj.id().value(),
                            smart_naming::get_object_type_name(obj.object_type())
                        )
                    })
                    .clone()
            };
            existing_names.entry(name).or_insert(obj.object_type());
        }

        let new_name =
            smart_naming::generate_smart_default_name(object.object_type(), &existing_names);

        let info = object_info
            .entry(object.id())
            .or_insert_with(|| ObjectInfo::new(object));
        info.set_name(new_name);
    }

    /// Save the project to a file
    pub fn save_project(&self) -> Result<Vec<u8>, serde_json::Error> {
        // Make sure we're saving the current state
        let object_info = self.object_info.borrow();
        let selected = if self.mut_selected_object.borrow().0.is_some() {
            self.mut_selected_object.borrow().0
        } else {
            self.selected_object.0
        };

        let project = ProjectFile::new(&self.pool, &object_info, self.mask_size, selected);
        project.to_bytes()
    }

    /// Load a project from file data
    pub fn load_project(data: Vec<u8>) -> Result<Self, String> {
        let project = ProjectFile::from_bytes(&data)
            .map_err(|e| format!("Failed to parse project file: {}", e))?;
        let pool = project.load_pool()?;
        let settings = project.get_settings();

        let mut editor_project = EditorProject::from(pool);
        editor_project.mask_size = settings.mask_size;

        // Restore object metadata
        let metadata = project.get_metadata();
        let mut object_info = editor_project.object_info.borrow_mut();
        let mut_pool = editor_project.get_mut_pool();
        for object in editor_project.pool.objects() {
            if let Some(meta) = metadata.get(&object.id().value()) {
                let info = object_info
                    .entry(object.id())
                    .or_insert_with(|| ObjectInfo::new(object));
                if let Some(name) = &meta.name {
                    info.set_name(name.clone());
                }
            }
        }
        drop(object_info);

        // Apply smart naming to objects without custom names
        for object in editor_project.pool.objects() {
            editor_project.apply_smart_naming_to_object(object);
        }

        // Restore last selected
        if let Some(selected_id) = settings.last_selected {
            if let Ok(id) = ObjectId::new(selected_id) {
                editor_project.selected_object = NullableObjectId(Some(id));
                editor_project
                    .mut_selected_object
                    .replace(NullableObjectId(Some(id)));
            }
        }

        Ok(editor_project)
    }

    /// Request to open image file dialog for a PictureGraphic object
    pub fn request_image_load(&self, object_id: ObjectId) {
        self.image_load_request.replace(Some(object_id));
    }

    /// Take and clear the image load request if any
    pub fn take_image_load_request(&self) -> Option<ObjectId> {
        self.image_load_request.replace(None)
    }
}
