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
    selected_object: NullableObjectId,
    pending_selected_object: Cell<NullableObjectId>,
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
    /// A hierarchy context-menu delete, consumed by the app after draft edits
    /// from the same frame have been committed.
    delete_request: RefCell<Option<ObjectId>>,
    /// A hierarchy inline rename, consumed after same-frame draft edits.
    rename_request: RefCell<Option<(ObjectId, String)>>,
    /// Operations queued by UI widgets that only hold a shared project reference.
    queued_operations: RefCell<Vec<Operation>>,
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
            pool,
            selected_object: NullableObjectId::default(),
            pending_selected_object: Cell::new(NullableObjectId::default()),
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
            delete_request: RefCell::new(None),
            rename_request: RefCell::new(None),
            queued_operations: RefCell::new(Vec::new()),
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

    /// Set the mutating selected object
    /// This is used to make changes to the selected object in the next frame
    /// without affecting the current selected object
    pub fn get_mut_selected(&self) -> &Cell<NullableObjectId> {
        &self.pending_selected_object
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

    /// Queue one UI operation for the end-of-frame transaction.
    pub fn queue_operation(&self, operation: Operation) {
        self.queued_operations.borrow_mut().push(operation);
    }

    /// Commit queued UI operations as one reversible transaction.
    pub fn commit_queued(&mut self, description: impl Into<String>) -> bool {
        let operations = self.queued_operations.take();
        if operations.is_empty() {
            return false;
        }
        self.execute_ui_transaction(OperationTransaction {
            schema_version: 1,
            description: Some(description.into()),
            operations,
        })
    }

    /// Commit the UI draft as one reversible operation transaction.
    /// Returns true if the pool was updated
    pub fn update_pool(&mut self) -> bool {
        self.commit_queued("Edit object pool")
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

    /// Create a default object through the operation executor and return its ID.
    ///
    /// UI callers should use this instead of inserting into the draft pool: the
    /// creation is immediately undoable and does not depend on the end-of-frame
    /// draft diff finding the new object again.
    pub fn create_object(&mut self, object_type: ObjectType, name: String) -> Option<ObjectId> {
        let object_id = self.allocate_object_id();
        let transaction = OperationTransaction {
            schema_version: 1,
            description: Some(format!("Create {object_type:?}")),
            operations: vec![Operation::CreateObject {
                handle: None,
                object_id: Some(object_id.value()),
                object_type: format!("{object_type:?}"),
                name: Some(name),
                captured_object: None,
                captured_info: None,
            }],
        };

        self.execute_ui_transaction(transaction)
            .then_some(object_id)
    }

    /// Delete an object through a reversible operation.
    pub fn delete_object(&mut self, object_id: ObjectId) -> bool {
        let deleted = self.execute_ui_transaction(OperationTransaction {
            schema_version: 1,
            description: Some(format!("Delete object {}", object_id.value())),
            operations: vec![Operation::DeleteObject {
                object_id: object_id.value(),
                captured_object: None,
            }],
        });
        if deleted && self.get_selected().0 == Some(object_id) {
            self.pending_selected_object.set(NullableObjectId::NULL);
        }
        deleted
    }

    /// Request deletion from UI code that only has a shared project reference.
    pub fn request_delete_object(&self, object_id: ObjectId) {
        self.delete_request.replace(Some(object_id));
    }

    /// Consume a pending hierarchy delete request.
    pub fn take_delete_request(&self) -> Option<ObjectId> {
        self.delete_request.replace(None)
    }

    /// Rename an object through a reversible metadata operation.
    pub fn rename_object(&mut self, object_id: ObjectId, name: String) -> bool {
        let current_name = self
            .object_info
            .borrow()
            .get(&object_id)
            .and_then(|info| info.name.clone())
            .unwrap_or_default();
        if current_name == name {
            return false;
        }
        self.execute_ui_transaction(OperationTransaction {
            schema_version: 1,
            description: Some(format!("Rename object {}", object_id.value())),
            operations: vec![Operation::RenameObject {
                object_id: object_id.value(),
                name,
            }],
        })
    }

    /// Request a rename from UI code that only has a shared project reference.
    pub fn request_rename_object(&self, object_id: ObjectId, name: String) {
        self.rename_request.replace(Some((object_id, name)));
    }

    /// Consume a pending hierarchy rename request.
    pub fn take_rename_request(&self) -> Option<(ObjectId, String)> {
        self.rename_request.replace(None)
    }

    /// Reorder the canonical pool through a reversible operation.
    pub fn reorder_objects(&mut self, object_ids: Vec<ObjectId>) -> bool {
        if self
            .pool
            .objects()
            .iter()
            .map(Object::id)
            .eq(object_ids.iter().copied())
        {
            return false;
        }
        self.execute_ui_transaction(OperationTransaction {
            schema_version: 1,
            description: Some("Reorder objects".to_owned()),
            operations: vec![Operation::ReorderObjects {
                object_ids: object_ids.into_iter().map(|id| id.value()).collect(),
            }],
        })
    }

    /// Sort objects by their displayed names through `ReorderObjects`.
    pub fn sort_objects_by_name(&mut self) -> bool {
        let mut objects: Vec<_> = self.pool.objects().iter().collect();
        objects.sort_by(|a, b| {
            self.get_object_info(a)
                .get_name(a)
                .cmp(&self.get_object_info(b).get_name(b))
        });
        self.reorder_objects(objects.into_iter().map(Object::id).collect())
    }

    /// Sort objects by ID through `ReorderObjects`.
    pub fn sort_objects_by_id(&mut self) -> bool {
        let mut object_ids: Vec<_> = self.pool.objects().iter().map(Object::id).collect();
        object_ids.sort_by_key(|id| id.value());
        self.reorder_objects(object_ids)
    }

    fn execute_ui_transaction(&mut self, transaction: OperationTransaction) -> bool {
        match self.execute_transaction(transaction) {
            Ok(_) => true,
            Err(error) => {
                log::error!("Failed to execute UI operation: {error:?}");
                self.last_operation_error
                    .replace(Some(format!("Could not apply edit: {error:?}")));
                false
            }
        }
    }

    /// Update the selected object with the mutating selected object if it is different
    /// Returns true if the selected object was updated
    pub fn update_selected(&mut self) -> bool {
        let mut_selected = self.pending_selected_object.get();
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
            self.pending_selected_object.set(selected);
        }
    }

    /// Set the selected object to the next object in the history
    pub fn set_next_selected(&mut self) {
        if let Some(selected) = self.redo_selected_history.pop() {
            self.undo_selected_history.push(self.selected_object);
            // Both need to be replaced here because otherwise the redo history will be cleared
            self.selected_object = selected.clone();
            self.pending_selected_object.set(selected);
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

    /// Finish inline renaming. The operation is submitted after this frame's
    /// draft edits have been committed.
    pub fn finish_renaming_object(&self, store: bool) {
        if store {
            if let Some(renaming_object) = self.renaming_object.borrow().as_ref() {
                self.request_rename_object(renaming_object.1, renaming_object.2.clone());
            }
        }
        self.renaming_object.replace(None);
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
        let selected = if self.pending_selected_object.get().0.is_some() {
            self.pending_selected_object.get().0
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
                    .pending_selected_object
                    .set(NullableObjectId(Some(id)));
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
        self.pool = modified_pool;

        self.object_info.replace(modified_object_info);
        self.mut_object_info
            .replace(self.object_info.borrow().clone());
        self.update_next_available_id();
        self.default_object_names.borrow_mut().clear();
        self.remap_selection_for_operations(&applied.forward_operations);

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

            for op in &inverse_tx.operations {
                match op.apply(&mut pool, &mut object_info, &mut context) {
                    Ok(_) => {}
                    Err(_) => {
                        history.rollback_undo();
                        return false;
                    }
                }
            }

            // Success: update editor state
            self.pool = pool;
            self.object_info.replace(object_info);
            self.mut_object_info
                .replace(self.object_info.borrow().clone());
            self.update_next_available_id();
            self.default_object_names.borrow_mut().clear();
            drop(history);
            self.remap_selection_for_operations(&inverse_tx.operations);
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
            self.pool = pool;
            self.object_info.replace(object_info);
            self.mut_object_info
                .replace(self.object_info.borrow().clone());
            self.update_next_available_id();
            self.default_object_names.borrow_mut().clear();
            drop(history);
            self.remap_selection_for_operations(&applied_tx.forward_operations);
            self.refresh_dirty();
            return true;
        }
        false
    }

    fn remap_selection_for_operations(&mut self, operations: &[Operation]) {
        for operation in operations {
            if let Operation::ChangeObjectId { old_id, new_id } = operation {
                if self.selected_object.0.map(|id| id.value()) == Some(*old_id) {
                    let selected = NullableObjectId::new(*new_id);
                    self.selected_object = selected;
                    self.pending_selected_object.set(selected);
                }
            }
        }
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

        project
            .create_object(ObjectType::StringVariable, "String variable".to_owned())
            .expect("CreateObject should succeed");
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
        project.pool.add(ws);
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
        assert!(project.rename_object(object.id(), "Temporary".to_owned()));
        let unique_id = project.get_object_info(&object).get_unique_id();
        project.mark_saved();
        let before = project.get_pool().as_iop();

        assert!(project.delete_object(object.id()));
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
        let object_id = project
            .create_object(ObjectType::Button, "Created button".to_owned())
            .expect("CreateObject should succeed");
        let object = project.get_pool().object_by_id(object_id).unwrap().clone();
        let unique_id = project.get_object_info(&object).get_unique_id();

        assert!(project.undo_operation());
        assert!(project.redo_operation());
        let restored = project.get_pool().object_by_id(object_id).unwrap();
        let info = project.get_object_info(restored);
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
        project.pool.add(button.clone());
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
    fn property_and_name_operations_round_trip_as_one_transaction() {
        let object = object_with_id(ObjectType::Button, 10);
        let mut pool = ObjectPool::default();
        pool.add(object.clone());
        let mut project = EditorProject::from(pool);
        project.mark_saved();
        let before = project.get_pool().as_iop();

        project.queue_operation(Operation::SetProperty {
            object_id: object.id().value(),
            property: "width".to_owned(),
            value: serde_json::json!(123),
        });
        project.queue_operation(Operation::SetProperty {
            object_id: object.id().value(),
            property: "height".to_owned(),
            value: serde_json::json!(45),
        });
        project.queue_operation(Operation::RenameObject {
            object_id: object.id().value(),
            name: "Apply button".to_owned(),
        });
        assert!(project.commit_queued("Edit button"));
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
    fn queued_field_edit_is_committed_as_a_property_operation() {
        let object = object_with_id(ObjectType::Button, 10);
        let mut pool = ObjectPool::default();
        pool.add(object.clone());
        let mut project = EditorProject::from(pool);

        project.queue_operation(Operation::SetProperty {
            object_id: object.id().value(),
            property: "width".to_owned(),
            value: serde_json::json!(123),
        });
        assert!(project.commit_queued("Set width on object 10"));
        assert_eq!(
            project.undo_description().as_deref(),
            Some("Set width on object 10")
        );
        assert!(project.undo_operation());
        assert!(matches!(
            project.get_pool().object_by_id(object.id()),
            Some(Object::Button(button)) if button.width == 0
        ));
    }

    #[test]
    fn queued_structural_edits_use_reversible_structural_operations() {
        use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef, ObjectRef, Point};

        let parent = object_with_id(ObjectType::Button, 10);
        let child = object_with_id(ObjectType::PictureGraphic, 11);
        let macro_object = object_with_id(ObjectType::Macro, 12);
        let mut pool = ObjectPool::default();
        pool.add(parent.clone());
        pool.add(child.clone());
        pool.add(macro_object);
        let mut project = EditorProject::from(pool);
        let before = project.get_pool().as_iop();

        project.queue_operation(Operation::SetChildren {
            parent_id: parent.id().value(),
            children: vec![ObjectRef {
                id: child.id(),
                offset: Point { x: 4, y: 8 },
            }],
        });
        project.queue_operation(Operation::SetMacroReferences {
            object_id: parent.id().value(),
            macro_refs: vec![MacroRef {
                macro_id: 12,
                event_id: Event::OnActivate,
            }],
        });

        assert!(project.commit_queued("Edit button structure"));
        let after = project.get_pool().as_iop();
        assert_ne!(after, before);
        assert!(project.undo_operation());
        assert_eq!(project.get_pool().as_iop(), before);
        assert!(project.redo_operation());
        assert_eq!(project.get_pool().as_iop(), after);
    }

    #[test]
    fn deleting_a_macro_referenced_by_an_event_is_rejected() {
        use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef};

        let mut parent = object_with_id(ObjectType::Button, 10);
        if let Object::Button(button) = &mut parent {
            button.macro_refs.push(MacroRef {
                macro_id: 12,
                event_id: Event::OnActivate,
            });
        }
        let macro_object = object_with_id(ObjectType::Macro, 12);
        let mut pool = ObjectPool::default();
        pool.add(parent);
        pool.add(macro_object);
        let mut project = EditorProject::from(pool);
        let mut transaction = OperationTransaction::new(Some("Delete macro".to_owned()));
        transaction.add_operation(Operation::DeleteObject {
            object_id: 12,
            captured_object: None,
        });

        assert!(matches!(
            project.execute_transaction(transaction),
            Err(OperationError::DeleteReferencedObject(12))
        ));
    }

    #[test]
    fn changing_an_object_id_rewrites_all_reference_kinds_and_selection() {
        use ag_iso_stack::object_pool::object_attributes::{
            Event, MacroRef, ObjectLabel, ObjectRef, Point,
        };

        let target = object_with_id(ObjectType::Macro, 10);
        let mut positioned_parent = object_with_id(ObjectType::Button, 20);
        if let Object::Button(button) = &mut positioned_parent {
            button.object_refs.push(ObjectRef {
                id: target.id(),
                offset: Point::default(),
            });
            button.macro_refs.push(MacroRef {
                macro_id: 10,
                event_id: Event::OnActivate,
            });
        }
        let mut required_list = object_with_id(ObjectType::SoftKeyMask, 21);
        if let Object::SoftKeyMask(mask) = &mut required_list {
            mask.objects.push(target.id());
        }
        let mut nullable_list = object_with_id(ObjectType::InputList, 22);
        if let Object::InputList(list) = &mut nullable_list {
            list.list_items.push(target.id().into());
        }
        let mut labels = object_with_id(ObjectType::ObjectLabelReferenceList, 23);
        if let Object::ObjectLabelReferenceList(list) = &mut labels {
            list.object_labels.push(ObjectLabel {
                id: target.id(),
                string_variable_reference: target.id().into(),
                font_type: 0,
                graphic_representation: target.id().into(),
            });
        }
        let mut property_parent = object_with_id(ObjectType::ObjectPointer, 24);
        if let Object::ObjectPointer(pointer) = &mut property_parent {
            pointer.value = target.id().into();
        }

        let mut pool = ObjectPool::default();
        for object in [
            target.clone(),
            positioned_parent,
            required_list,
            nullable_list,
            labels,
            property_parent,
        ] {
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        project
            .get_mut_selected()
            .replace(NullableObjectId::new(target.id().value()));
        project.update_selected();
        assert!(project.rename_object(target.id(), "Renumbered macro".to_owned()));

        let mut transaction = OperationTransaction::new(Some("Renumber macro".to_owned()));
        transaction.add_operation(Operation::ChangeObjectId {
            old_id: 10,
            new_id: 30,
        });
        project.execute_transaction(transaction).unwrap();

        let new_id = ObjectId::new(30).unwrap();
        assert!(project.get_pool().object_by_id(target.id()).is_none());
        assert!(project.get_pool().object_by_id(new_id).is_some());
        assert_eq!(project.get_selected(), NullableObjectId::new(30));
        assert!(matches!(
            project
                .get_pool()
                .object_by_id(ObjectId::new(20).unwrap()),
            Some(Object::Button(button))
                if button.macro_refs.first().map(|reference| reference.macro_id) == Some(30)
        ));
        assert_eq!(
            project
                .get_pool()
                .objects()
                .iter()
                .filter(|object| object.id() != new_id)
                .flat_map(Object::referenced_objects)
                .filter(|id| *id == target.id())
                .count(),
            0
        );
        assert_eq!(
            project
                .get_object_info(project.get_pool().object_by_id(new_id).unwrap())
                .name
                .as_deref(),
            Some("Renumbered macro")
        );

        assert!(project.undo_operation());
        assert!(project.get_pool().object_by_id(target.id()).is_some());
        assert_eq!(project.get_selected(), NullableObjectId::new(10));
        assert!(project.redo_operation());
        assert!(project.get_pool().object_by_id(new_id).is_some());
        assert_eq!(project.get_selected(), NullableObjectId::new(30));
    }

    #[test]
    fn change_id_operation_rewrites_inbound_references() {
        let target = object_with_id(ObjectType::PictureGraphic, 10);
        let mut parent = object_with_id(ObjectType::Button, 20);
        if let Object::Button(button) = &mut parent {
            button
                .object_refs
                .push(ag_iso_stack::object_pool::ObjectRef {
                    id: target.id(),
                    offset: Default::default(),
                });
        }
        let mut pool = ObjectPool::default();
        pool.add(target.clone());
        pool.add(parent);
        let mut project = EditorProject::from(pool);

        let mut transaction =
            OperationTransaction::new(Some("Change object ID 10 to 30".to_owned()));
        transaction.add_operation(Operation::ChangeObjectId {
            old_id: 10,
            new_id: 30,
        });
        project.execute_transaction(transaction).unwrap();
        assert_eq!(
            project.undo_description().as_deref(),
            Some("Change object ID 10 to 30")
        );
        assert_eq!(
            project
                .get_pool()
                .object_by_id(ObjectId::new(20).unwrap())
                .unwrap()
                .referenced_objects(),
            vec![ObjectId::new(30).unwrap()]
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

        project.queue_operation(Operation::SetProperty {
            object_id: object.id().value(),
            property: "width".to_owned(),
            value: serde_json::json!(99),
        });
        assert!(project.commit_queued("Set button width"));
        // Corrupt only the live state to make the inverse's target unavailable.
        project.pool.remove(object.id());

        assert!(!project.undo_operation());
        assert!(project.undo_available());
        assert!(!project.redo_available());
    }

    #[test]
    fn command_create_and_reorder_are_undoable() {
        let mut pool = ObjectPool::default();
        pool.add(object_with_id(ObjectType::Button, 20));
        pool.add(object_with_id(ObjectType::Button, 10));
        let mut project = EditorProject::from(pool);

        let created = project
            .create_object(ObjectType::StringVariable, "Created variable".to_owned())
            .expect("CreateObject should succeed");
        assert!(project.get_pool().object_by_id(created).is_some());
        assert_eq!(
            project
                .get_object_info(project.get_pool().object_by_id(created).unwrap())
                .name
                .as_deref(),
            Some("Created variable")
        );
        assert!(project.undo_operation());
        assert!(project.get_pool().object_by_id(created).is_none());
        assert!(project.redo_operation());
        assert!(project.get_pool().object_by_id(created).is_some());

        assert!(project.rename_object(created, "Renamed variable".to_owned()));
        assert_eq!(
            project
                .get_object_info(project.get_pool().object_by_id(created).unwrap())
                .name
                .as_deref(),
            Some("Renamed variable")
        );
        assert!(project.undo_operation());
        assert_eq!(
            project
                .get_object_info(project.get_pool().object_by_id(created).unwrap())
                .name
                .as_deref(),
            Some("Created variable")
        );
        assert!(project.redo_operation());

        assert!(project.sort_objects_by_id());
        assert_eq!(
            project
                .get_pool()
                .objects()
                .iter()
                .map(|object| object.id().value())
                .collect::<Vec<_>>(),
            vec![10, 20, created.value()]
        );
        assert!(project.undo_operation());
        assert_eq!(
            project
                .get_pool()
                .objects()
                .iter()
                .map(|object| object.id().value())
                .collect::<Vec<_>>(),
            vec![20, 10, created.value()]
        );
    }

    #[test]
    fn queued_property_operation_round_trips_through_history() {
        let button = object_with_id(ObjectType::Button, 10);
        let mut pool = ObjectPool::default();
        pool.add(button.clone());
        let mut project = EditorProject::from(pool);

        project.queue_operation(Operation::SetProperty {
            object_id: button.id().value(),
            property: "width".to_owned(),
            value: serde_json::json!(30),
        });

        assert!(project.commit_queued("Edit button"));
        assert_eq!(project.undo_description().as_deref(), Some("Edit button"));
        assert!(matches!(
            project.get_pool().object_by_id(button.id()),
            Some(Object::Button(button)) if button.width == 30
        ));

        assert!(project.undo_operation());
        assert!(matches!(
            project.get_pool().object_by_id(button.id()),
            Some(Object::Button(button)) if button.width == 0
        ));
    }
}
