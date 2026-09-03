//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use ag_iso_stack::object_pool::{
    object::Object, NullableObjectId, ObjectId, ObjectPool, ObjectType,
};

use crate::ai::snapshot::{PoolSnapshot, SnapshotError};
use crate::operations::{
    AppliedTransaction, Operation, OperationError, OperationExecutor, OperationHistory,
    OperationTransaction,
};
use crate::{project_file::ProjectFile, smart_naming, ObjectInfo};

const MAX_UNDO_REDO_SELECTED: usize = 20;

#[derive(Clone, PartialEq)]
struct ProjectContentCheckpoint {
    pool: ObjectPool,
    object_names: Vec<(u16, String)>,
    mask_size: u16,
}

#[derive(Default, Clone)]
pub struct EditorProject {
    pool: ObjectPool,
    mut_pool: RefCell<ObjectPool>,
    selected_object: NullableObjectId,
    mut_selected_object: RefCell<NullableObjectId>,
    undo_selected_history: Vec<NullableObjectId>,
    redo_selected_history: Vec<NullableObjectId>,
    pub mask_size: u16,
    soft_key_size: (u16, u16),
    object_info: RefCell<HashMap<ObjectId, ObjectInfo>>,
    /// Metadata draft edited by the UI and committed through operations.
    mut_object_info: RefCell<HashMap<ObjectId, ObjectInfo>>,

    /// Used to keep track of the object that is being renamed
    renaming_object: RefCell<Option<(eframe::egui::Id, ObjectId, String)>>,

    /// Cached next available ID for efficient allocation
    next_available_id: RefCell<u16>,

    /// Cached default object names for efficient lookup
    default_object_names: RefCell<HashMap<ObjectId, String>>,

    /// Request to open image file dialog for PictureGraphic object
    image_load_request: RefCell<Option<ObjectId>>,
    last_operation_error: RefCell<Option<String>>,

    /// Last project content that was opened or successfully saved.
    saved_content: RefCell<Option<ProjectContentCheckpoint>>,
    dirty: Cell<bool>,

    /// Bounded operation-based undo/redo history.
    pub operation_history: RefCell<OperationHistory>,
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

        let project = EditorProject {
            mut_pool: RefCell::new(pool.clone()),
            pool,
            selected_object: NullableObjectId::default(),
            mut_selected_object: RefCell::new(NullableObjectId::default()),
            undo_selected_history: Default::default(),
            redo_selected_history: Default::default(),
            mask_size,
            soft_key_size,
            object_info: RefCell::new(HashMap::new()),
            mut_object_info: RefCell::new(HashMap::new()),
            renaming_object: RefCell::new(None),
            next_available_id: RefCell::new(max_id.saturating_add(1)),
            default_object_names: RefCell::new(HashMap::new()),
            image_load_request: RefCell::new(None),
            last_operation_error: RefCell::new(None),
            saved_content: RefCell::new(None),
            dirty: Cell::new(false),
            operation_history: RefCell::new(OperationHistory::new(10)), // Default 10 transaction history
        };
        project.mark_saved();
        project
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

    fn content_checkpoint(&self) -> ProjectContentCheckpoint {
        let object_info = self.mut_object_info.borrow();
        let mut object_names: Vec<_> = object_info
            .iter()
            .filter_map(|(id, info)| info.name.clone().map(|name| (id.value(), name)))
            .collect();
        object_names.sort_by_key(|(id, _)| *id);

        ProjectContentCheckpoint {
            pool: self.pool.clone(),
            object_names,
            mask_size: self.mask_size,
        }
    }

    fn refresh_dirty(&self) {
        let dirty = self
            .saved_content
            .borrow()
            .as_ref()
            .is_none_or(|saved| *saved != self.content_checkpoint());
        self.dirty.set(dirty);
    }

    /// Whether meaningful project content differs from the last open/save checkpoint.
    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    /// Establish the current content as the last successfully saved state.
    pub fn mark_saved(&self) {
        self.saved_content.replace(Some(self.content_checkpoint()));
        self.dirty.set(false);
    }

    /// Mark a restored recovery snapshot as content that still needs an explicit save.
    pub fn mark_recovered(&self) {
        self.saved_content.replace(None);
        self.dirty.set(true);
    }

    pub fn set_mask_size(&mut self, mask_size: u16) {
        if self.mask_size != mask_size {
            self.mask_size = mask_size;
            self.refresh_dirty();
        }
    }

    pub fn set_object_name(&self, object: &Object, name: String) {
        let mut object_info = self.mut_object_info.borrow_mut();
        let info = object_info
            .entry(object.id())
            .or_insert_with(|| ObjectInfo::new(object));
        let old_name = info.name.clone();
        info.set_name(name);
        if info.name != old_name {
            drop(object_info);
            self.refresh_dirty();
        }
    }

    /// Commit the UI draft as one reversible operation transaction.
    /// Returns true if the pool was updated
    pub fn update_pool(&mut self) -> bool {
        let draft_pool = self.mut_pool.borrow().clone();
        let draft_info = self.mut_object_info.borrow().clone();
        let mut transaction = OperationTransaction::new(Some("Edit object pool".to_owned()));
        let mut consumed_new_ids = std::collections::HashSet::new();
        let old_ids: std::collections::HashSet<_> = self
            .pool
            .objects()
            .iter()
            .map(|object| object.id())
            .collect();
        let new_ids: std::collections::HashSet<_> = draft_pool
            .objects()
            .iter()
            .map(|object| object.id())
            .collect();

        // Match in-place ID changes by vector slot and object type. Object editors
        // mutate one draft object in place, so this is deterministic.
        for (old, new) in self.pool.objects().iter().zip(draft_pool.objects()) {
            if old.id() != new.id()
                && old.object_type() == new.object_type()
                && !new_ids.contains(&old.id())
                && !old_ids.contains(&new.id())
            {
                transaction.add_operation(Operation::ReplaceObject {
                    object_id: old.id().value(),
                    object: new.clone(),
                });
                consumed_new_ids.insert(new.id());
            }
        }

        for old in self.pool.objects() {
            if let Some(new) = draft_pool.object_by_id(old.id()) {
                if old != new {
                    transaction.add_operation(Operation::ReplaceObject {
                        object_id: old.id().value(),
                        object: new.clone(),
                    });
                }
            } else if !transaction.operations.iter().any(|operation| {
                matches!(operation, Operation::ReplaceObject { object_id, .. } if *object_id == old.id().value())
            }) {
                transaction.add_operation(Operation::DeleteObject {
                    object_id: old.id().value(),
                    captured_object: None,
                });
            }
        }

        for new in draft_pool.objects() {
            if !old_ids.contains(&new.id()) && !consumed_new_ids.contains(&new.id()) {
                transaction.add_operation(Operation::CreateObject {
                    handle: None,
                    object_id: Some(new.id().value()),
                    object_type: format!("{:?}", new.object_type()),
                    name: draft_info.get(&new.id()).and_then(|info| info.name.clone()),
                    captured_object: serde_json::to_value(new).ok(),
                    captured_info: draft_info.get(&new.id()).cloned(),
                });
            }
        }

        let created_ids: std::collections::HashSet<_> = transaction
            .operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::CreateObject {
                    object_id: Some(id),
                    ..
                } => ObjectId::new(*id).ok(),
                _ => None,
            })
            .collect();
        for (id, info) in &draft_info {
            if created_ids.contains(id) {
                continue;
            }
            let new_name = info.name.clone().unwrap_or_default();
            let old_name = self
                .object_info
                .borrow()
                .get(id)
                .and_then(|old| old.name.clone())
                .unwrap_or_default();
            if new_name != old_name && new_ids.contains(id) {
                transaction.add_operation(Operation::RenameObject {
                    object_id: id.value(),
                    name: new_name,
                });
            }
        }

        let old_order: Vec<_> = self
            .pool
            .objects()
            .iter()
            .map(|object| object.id())
            .collect();
        let new_order: Vec<_> = draft_pool
            .objects()
            .iter()
            .map(|object| object.id())
            .collect();
        if old_ids == new_ids && old_order != new_order {
            transaction.add_operation(Operation::ReorderObjects {
                object_ids: new_order.iter().map(|id| id.value()).collect(),
            });
        }

        if transaction.operations.is_empty() {
            return false;
        }

        transaction.description = Some(if transaction.operations.len() == 1 {
            match &transaction.operations[0] {
                Operation::CreateObject { object_type, .. } => format!("Create {object_type}"),
                Operation::DeleteObject { object_id, .. } => format!("Delete object {object_id}"),
                Operation::ReplaceObject { object_id, .. } => format!("Edit object {object_id}"),
                Operation::RenameObject { object_id, .. } => format!("Rename object {object_id}"),
                Operation::ReorderObjects { .. } => "Reorder objects".to_owned(),
                _ => "Edit object pool".to_owned(),
            }
        } else {
            "Edit object pool".to_owned()
        });

        match self.execute_transaction(transaction) {
            Ok(_) => true,
            Err(error) => {
                log::error!("Failed to commit UI transaction: {:?}", error);
                self.last_operation_error
                    .replace(Some(format!("Could not apply edit: {:?}", error)));
                self.mut_pool.replace(self.pool.clone());
                self.mut_object_info
                    .replace(self.object_info.borrow().clone());
                false
            }
        }
    }

    /// Undo the last action
    pub fn undo(&mut self) {
        self.undo_operation();
    }

    /// Check if there are actions available to undo
    pub fn undo_available(&self) -> bool {
        self.operation_history.borrow().can_undo()
    }

    pub fn undo_description(&self) -> Option<String> {
        self.operation_history
            .borrow()
            .undo_description()
            .map(str::to_owned)
    }

    /// Redo the last undone action
    pub fn redo(&mut self) {
        self.redo_operation();
    }

    /// Check if there are actions available to redo
    pub fn redo_available(&self) -> bool {
        self.operation_history.borrow().can_redo()
    }

    pub fn redo_description(&self) -> Option<String> {
        self.operation_history
            .borrow()
            .redo_description()
            .map(str::to_owned)
    }

    pub fn take_operation_error(&self) -> Option<String> {
        self.last_operation_error.replace(None)
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
        let mut object_info = self.mut_object_info.borrow_mut();
        if let Some(info) = object_info.remove(&old_id) {
            object_info.insert(new_id, info);
        }
    }

    /// Get the object info for an object id
    /// If the object id is not mapped, we insert the default object info
    pub fn get_object_info(&self, object: &Object) -> ObjectInfo {
        let mut object_info = self.mut_object_info.borrow_mut();
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
                let mut object_info = self.mut_object_info.borrow_mut();
                if let Some(info) = object_info.get_mut(&renaming_object.1) {
                    let old_name = info.name.clone();
                    info.set_name(renaming_object.2.clone());
                    if info.name != old_name {
                        drop(object_info);
                        self.refresh_dirty();
                    }
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
        let object_info = self.mut_object_info.borrow();
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
        let mut object_info = self.mut_object_info.borrow_mut();

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
        drop(object_info);
        self.object_info
            .replace(self.mut_object_info.borrow().clone());
    }

    /// Apply smart naming to an existing object if it doesn't have a custom name
    pub fn apply_smart_naming_to_object(&self, object: &Object) {
        let mut object_info = self.mut_object_info.borrow_mut();

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
        drop(object_info);
        self.object_info
            .replace(self.mut_object_info.borrow().clone());
    }

    /// Save the project to a file
    pub fn save_project(&self) -> Result<Vec<u8>, serde_json::Error> {
        // Make sure we're saving the current state
        let object_info = self.mut_object_info.borrow();
        let selected = if self.mut_selected_object.borrow().0.is_some() {
            self.mut_selected_object.borrow().0
        } else {
            self.selected_object.0
        };

        let draft_pool = self.mut_pool.borrow();
        let project = ProjectFile::new(&draft_pool, &object_info, self.mask_size, selected);
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
        editor_project
            .mut_object_info
            .replace(editor_project.object_info.borrow().clone());

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

        editor_project.mark_saved();

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

    /// Execute a transaction and integrate with undo/redo
    /// Returns the applied transaction or an error
    pub fn execute_transaction(
        &mut self,
        transaction: OperationTransaction,
    ) -> Result<AppliedTransaction, OperationError> {
        let executor = OperationExecutor::new();

        let (modified_pool, modified_object_info, applied) =
            executor.execute(transaction, &self.pool, &self.object_info.borrow())?;

        // Replace pool and metadata with results
        self.pool = modified_pool.clone();
        self.mut_pool.replace(modified_pool);

        self.object_info.replace(modified_object_info);
        self.mut_object_info
            .replace(self.object_info.borrow().clone());
        self.update_next_available_id();
        self.default_object_names.borrow_mut().clear();

        // Record in history (this is a user action)
        self.operation_history.borrow_mut().push(applied.clone());

        // Mark dirty and refresh state
        self.refresh_dirty();

        Ok(applied)
    }

    /// Undo the last transaction via operation history
    /// Does NOT go through normal executor; applies inverse directly
    /// Returns true if undo succeeded, false if no undo available or undo failed
    pub fn undo_operation(&mut self) -> bool {
        let mut history = self.operation_history.borrow_mut();

        if let Some(applied_tx) = history.undo() {
            let inverse_tx = applied_tx.create_inverse();

            // Apply inverse operations directly without creating new history entry
            let mut pool = self.pool.clone();
            let mut object_info = self.object_info.borrow().clone();
            let mut context = crate::operations::OperationContext::default();

            for op in inverse_tx.operations {
                match op.apply(&mut pool, &mut object_info, &mut context) {
                    Ok(_) => {}
                    Err(_) => {
                        history.rollback_undo();
                        return false;
                    }
                }
            }

            // Success: update editor state
            let pool_clone = pool.clone();
            self.pool = pool;
            self.mut_pool.replace(pool_clone);
            self.object_info.replace(object_info);
            self.mut_object_info
                .replace(self.object_info.borrow().clone());
            self.update_next_available_id();
            self.default_object_names.borrow_mut().clear();
            self.refresh_dirty();
            return true;
        }
        false
    }

    /// Redo the last undone transaction via operation history
    /// Does NOT go through normal executor; applies forward operations directly
    /// Returns true if redo succeeded, false if no redo available or redo failed
    pub fn redo_operation(&mut self) -> bool {
        let mut history = self.operation_history.borrow_mut();

        if let Some(applied_tx) = history.redo() {
            let mut pool = self.pool.clone();
            let mut object_info = self.object_info.borrow().clone();
            let mut context = crate::operations::OperationContext::default();

            for op in &applied_tx.forward_operations {
                match op.apply(&mut pool, &mut object_info, &mut context) {
                    Ok(_) => {}
                    Err(_) => {
                        history.rollback_redo();
                        return false;
                    }
                }
            }

            // Success: update editor state
            let pool_clone = pool.clone();
            self.pool = pool;
            self.mut_pool.replace(pool_clone);
            self.object_info.replace(object_info);
            self.mut_object_info
                .replace(self.object_info.borrow().clone());
            self.update_next_available_id();
            self.default_object_names.borrow_mut().clear();
            self.refresh_dirty();
            return true;
        }
        false
    }

    /// Create a snapshot for AI context
    pub fn snapshot(&self) -> Result<PoolSnapshot, SnapshotError> {
        use ag_iso_stack::object_pool::vt_version::VtVersion;

        // TODO: Get VT version from pool once that API is available
        // For now use Version6 as default
        let vt_version = VtVersion::Version6;

        PoolSnapshot::new(
            &self.pool,
            &self.object_info.borrow(),
            self.selected_object.0,
            (self.mask_size, self.soft_key_size.1),
            vt_version,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_object;

    #[test]
    fn content_changes_and_undo_update_dirty_state() {
        let mut project = EditorProject::from(ObjectPool::default());
        assert!(!project.is_dirty());

        let object = default_object(ObjectType::WorkingSet);
        project.get_mut_pool().borrow_mut().add(object);
        assert!(project.update_pool());
        assert!(project.is_dirty());

        project.undo();
        assert!(!project.is_dirty());
    }

    #[test]
    fn operation_transaction_rename_object_round_trip() {
        use crate::operations::{Operation, OperationTransaction};

        let mut project = EditorProject::from(ObjectPool::default());

        // Add a working set object (ID=1, which is the default for the first object)
        let mut ws = default_object(ObjectType::WorkingSet);
        let ws_id_value = 1u16;
        if let Ok(id) = ObjectId::new(ws_id_value) {
            ws = match ws {
                Object::WorkingSet(mut ws_obj) => {
                    // The object is already created with ID 1
                    Object::WorkingSet(ws_obj)
                }
                _ => unreachable!(),
            };
        }
        project.get_mut_pool().borrow_mut().add(ws);
        project.update_pool();

        // Create initial object info entry
        if let Ok(id) = ObjectId::new(ws_id_value) {
            let mut info = ObjectInfo::new(&default_object(ObjectType::WorkingSet));
            info.name = Some("Original Name".to_string());
            project.object_info.borrow_mut().insert(id, info);
        }

        // Create a rename transaction
        let mut tx = OperationTransaction::new(Some("Rename WorkingSet".to_string()));
        tx.add_operation(Operation::RenameObject {
            object_id: ws_id_value,
            name: "New Name".to_string(),
        });

        // Execute the transaction
        let applied = project
            .execute_transaction(tx)
            .expect("Transaction should succeed");

        // Verify the name changed
        if let Ok(id) = ObjectId::new(ws_id_value) {
            let info = project.object_info.borrow();
            assert_eq!(
                info.get(&id).and_then(|i| i.name.clone()),
                Some("New Name".to_string())
            );
        }

        // Undo the transaction
        assert!(project.undo_operation(), "Undo should succeed");
        if let Ok(id) = ObjectId::new(ws_id_value) {
            let info = project.object_info.borrow();
            assert_eq!(
                info.get(&id).and_then(|i| i.name.clone()),
                Some("Original Name".to_string())
            );
        }

        // Redo the transaction
        assert!(project.redo_operation(), "Redo should succeed");
        if let Ok(id) = ObjectId::new(ws_id_value) {
            let info = project.object_info.borrow();
            assert_eq!(
                info.get(&id).and_then(|i| i.name.clone()),
                Some("New Name".to_string())
            );
        }
    }

    #[test]
    fn operation_transaction_object_clone_check() {
        // Test to verify that Object implements Clone
        let obj = default_object(ObjectType::WorkingSet);
        let _cloned = obj.clone();
        // If this compiles and runs, Object is Clone
    }

    #[test]
    fn operation_delete_object_rejects_referenced() {
        use crate::operations::{Operation, OperationTransaction};
        use ag_iso_stack::object_pool::object_attributes::{ObjectRef, Point};

        let mut parent = object_with_id(ObjectType::DataMask, 10);
        let child = object_with_id(ObjectType::Button, 11);
        if let Object::DataMask(mask) = &mut parent {
            mask.object_refs.push(ObjectRef {
                id: child.id(),
                offset: Point::default(),
            });
        }
        let mut pool = ObjectPool::default();
        pool.add(parent);
        pool.add(child);
        let mut project = EditorProject::from(pool);

        let mut tx = OperationTransaction::new(Some("Delete referenced button".to_string()));
        tx.add_operation(Operation::DeleteObject {
            object_id: 11,
            captured_object: None,
        });

        let result = project.execute_transaction(tx);
        assert!(matches!(
            result,
            Err(OperationError::DeleteReferencedObject(11))
        ));
    }

    #[test]
    fn operation_delete_object_round_trip_restores_name() {
        let object = object_with_id(ObjectType::Button, 10);
        let mut pool = ObjectPool::default();
        pool.add(object.clone());
        let mut project = EditorProject::from(pool);
        project.set_object_name(&object, "Temporary".to_owned());
        assert!(project.update_pool());
        let unique_id = project.get_object_info(&object).get_unique_id();
        project.mark_saved();
        let before = project.get_pool().as_iop();

        project.get_mut_pool().borrow_mut().remove(object.id());
        assert!(project.update_pool());
        assert!(project.get_pool().object_by_id(object.id()).is_none());

        assert!(project.undo_operation());
        assert_eq!(project.get_pool().as_iop(), before);
        assert_eq!(
            project.get_object_info(&object).name.as_deref(),
            Some("Temporary")
        );
        assert_eq!(project.get_object_info(&object).get_unique_id(), unique_id);
        assert!(project.redo_operation());
        assert!(project.get_pool().object_by_id(object.id()).is_none());
    }

    #[test]
    fn ui_created_object_keeps_metadata_identity_after_redo() {
        let mut project = EditorProject::from(ObjectPool::default());
        let object = object_with_id(ObjectType::Button, 10);
        project.get_mut_pool().borrow_mut().add(object.clone());
        project.set_object_name(&object, "Created button".to_owned());
        let unique_id = project.get_object_info(&object).get_unique_id();

        assert!(project.update_pool());
        assert!(project.undo_operation());
        assert!(project.redo_operation());
        let info = project.get_object_info(&object);
        assert_eq!(info.name.as_deref(), Some("Created button"));
        assert_eq!(info.get_unique_id(), unique_id);
    }

    #[test]
    fn operation_delete_nonexistent_object_fails() {
        use crate::operations::{Operation, OperationTransaction};

        let mut project = EditorProject::from(ObjectPool::default());

        let mut tx = OperationTransaction::new(Some("Delete nonexistent".to_string()));
        tx.add_operation(Operation::DeleteObject {
            object_id: 999,
            captured_object: None,
        });

        let result = project.execute_transaction(tx);
        assert!(
            result.is_err(),
            "DeleteObject should fail for nonexistent object"
        );
    }

    #[test]
    fn operation_set_button_property() {
        use crate::operations::{Operation, OperationTransaction};
        use serde_json::json;

        let mut project = EditorProject::from(ObjectPool::default());

        // Add a button object
        let mut button = default_object(ObjectType::Button);
        project.get_mut_pool().borrow_mut().add(button.clone());
        project.update_pool();

        // Set button width via SetProperty operation
        let mut tx = OperationTransaction::new(Some("Set button width".to_string()));
        tx.add_operation(Operation::SetProperty {
            object_id: u16::from(button.id()),
            property: "width".to_string(),
            value: json!(100),
        });

        let result = project.execute_transaction(tx);
        assert!(
            result.is_ok(),
            "SetProperty should succeed for button width"
        );

        // Verify width changed
        if let Some(updated_button) = project.get_pool().object_by_id(button.id()) {
            if let ag_iso_stack::object_pool::object::Object::Button(b) = updated_button {
                assert_eq!(b.width, 100, "Button width should be 100");
            }
        }

        // Undo should restore original width
        assert!(project.undo_operation(), "Undo should succeed");
        if let Some(restored_button) = project.get_pool().object_by_id(button.id()) {
            if let ag_iso_stack::object_pool::object::Object::Button(b) = restored_button {
                assert_eq!(b.width, 0, "Button width should be restored to 0");
            }
        }
    }

    #[test]
    fn mask_size_and_names_are_tracked_but_selection_is_not() {
        let mut pool = ObjectPool::default();
        let object = default_object(ObjectType::WorkingSet);
        let object_id = object.id();
        pool.add(object);
        let mut project = EditorProject::from(pool);

        project
            .get_mut_selected()
            .replace(NullableObjectId(Some(object_id)));
        project.update_selected();
        assert!(!project.is_dirty());

        // Lazily-created display metadata without a custom name is not project content.
        let object = project.get_pool().object_by_id(object_id).unwrap().clone();
        project.get_object_info(&object);

        let original_mask_size = project.mask_size;
        project.set_mask_size(original_mask_size.saturating_add(1));
        assert!(project.is_dirty());
        project.set_mask_size(original_mask_size);
        assert!(!project.is_dirty());

        project.set_object_name(&object, "First name".to_owned());
        assert!(project.is_dirty());
        project.mark_saved();
        project.set_object_name(&object, "Second name".to_owned());
        assert!(project.is_dirty());
        project.set_object_name(&object, "First name".to_owned());
        assert!(!project.is_dirty());
    }

    #[test]
    fn recovered_projects_remain_dirty_until_marked_saved() {
        let project = EditorProject::from(ObjectPool::default());
        project.mark_recovered();
        assert!(project.is_dirty());
        project.mark_saved();
        assert!(!project.is_dirty());
    }

    fn object_with_id(object_type: ObjectType, id: u16) -> Object {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        object
    }

    #[test]
    fn ui_draft_property_and_name_round_trip_through_history() {
        let object = object_with_id(ObjectType::Button, 10);
        let mut pool = ObjectPool::default();
        pool.add(object.clone());
        let mut project = EditorProject::from(pool);
        project.mark_saved();
        let before = project.get_pool().as_iop();

        if let Some(Object::Button(button)) = project
            .get_mut_pool()
            .borrow_mut()
            .object_mut_by_id(object.id())
        {
            button.width = 123;
            button.height = 45;
        }
        project.set_object_name(&object, "Apply button".to_owned());
        assert!(project.update_pool());
        let after = project.get_pool().as_iop();
        assert_ne!(after, before);
        assert_eq!(
            project.get_object_info(&object).name.as_deref(),
            Some("Apply button")
        );

        assert!(project.undo_operation());
        assert_eq!(project.get_pool().as_iop(), before);
        assert_ne!(
            project.get_object_info(&object).name.as_deref(),
            Some("Apply button")
        );

        assert!(project.redo_operation());
        assert_eq!(project.get_pool().as_iop(), after);
        assert_eq!(
            project.get_object_info(&object).name.as_deref(),
            Some("Apply button")
        );
    }

    #[test]
    fn add_move_remove_child_are_reversible() {
        use crate::operations::Operation;

        let parent = object_with_id(ObjectType::DataMask, 10);
        let child = object_with_id(ObjectType::Button, 11);
        let mut pool = ObjectPool::default();
        pool.add(parent.clone());
        pool.add(child.clone());
        let mut project = EditorProject::from(pool);
        let before = project.get_pool().as_iop();

        let mut add = OperationTransaction::new(Some("Add child".to_owned()));
        add.add_operation(Operation::AddChild {
            parent_id: 10,
            child_id: 11,
            x: 7,
            y: 9,
        });
        project.execute_transaction(add).unwrap();
        let after_add = project.get_pool().as_iop();
        assert_ne!(after_add, before);

        let mut movement = OperationTransaction::new(Some("Move child".to_owned()));
        movement.add_operation(Operation::SetChildPosition {
            parent_id: 10,
            child_id: 11,
            x: 30,
            y: 40,
        });
        project.execute_transaction(movement).unwrap();
        let after_move = project.get_pool().as_iop();

        assert!(project.undo_operation());
        assert_eq!(project.get_pool().as_iop(), after_add);
        assert!(project.undo_operation());
        assert_eq!(project.get_pool().as_iop(), before);
        assert!(project.redo_operation());
        assert_eq!(project.get_pool().as_iop(), after_add);
        assert!(project.redo_operation());
        assert_eq!(project.get_pool().as_iop(), after_move);
    }

    #[test]
    fn failed_undo_restores_history_position() {
        let object = object_with_id(ObjectType::Button, 10);
        let mut pool = ObjectPool::default();
        pool.add(object.clone());
        let mut project = EditorProject::from(pool);

        if let Some(Object::Button(button)) = project
            .get_mut_pool()
            .borrow_mut()
            .object_mut_by_id(object.id())
        {
            button.width = 99;
        }
        assert!(project.update_pool());
        // Corrupt only the live state to make the inverse's target unavailable.
        project.pool.remove(object.id());
        project.mut_pool.replace(project.pool.clone());

        assert!(!project.undo_operation());
        assert!(project.undo_available());
        assert!(!project.redo_available());
    }
}
