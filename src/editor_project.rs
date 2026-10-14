//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use crate::components::*;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ag_iso_stack::object_pool::{
    object::Object, vt_version::VtVersion, NullableObjectId, ObjectId, ObjectPool, ObjectType,
};

use crate::ai::snapshot::{DesignContextRequest, DesignSnapshot, SnapshotError};
use crate::ai::{AiDesignState, AiProposal, ProjectRevision};
use crate::operations::{
    AppliedTransaction, ObjectTarget, Operation, OperationError, OperationExecutor,
    OperationHistory, OperationTransaction, PreparedTransaction, TransactionDiagnostics,
    ValidationContext, CURRENT_OPERATION_SCHEMA_VERSION,
};
use crate::{project_file::ProjectFile, smart_naming, ObjectIdFormat, ObjectInfo, SoftKeyLayout};

const MAX_UNDO_REDO_SELECTED: usize = 20;

/// Failure to find a free ID inside a type's configured automatic range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectIdAllocationError {
    pub object_type: ObjectType,
    pub range: crate::ObjectIdRange,
}

impl std::fmt::Display for ObjectIdAllocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "No free {:?} object ID in configured range {}",
            self.object_type, self.range
        )
    }
}

impl std::error::Error for ObjectIdAllocationError {}

#[derive(Clone, PartialEq)]
struct ProjectContentCheckpoint {
    pool: ObjectPool,
    object_names: Vec<(u16, String)>,
    allowed_references: Vec<(u16, Vec<ObjectId>)>,
    overridden_ids: Vec<u16>,
    mask_size: u16,
    soft_key_size: (u16, u16),
    soft_key_layout: SoftKeyLayout,
    vt_version: VtVersion,
    object_id_ranges: crate::ObjectIdRanges,
    components: ComponentSource,
    graphics: crate::graphics::GraphicLibrary,
}

#[derive(Clone)]
struct ComponentCheckpoint {
    pool: ObjectPool,
    source: ComponentSource,
    graphics: crate::graphics::GraphicLibrary,
    info: HashMap<ObjectId, ObjectInfo>,
}

#[derive(Debug, Clone, PartialEq)]
struct DesignDimensions {
    mask_size: u16,
    soft_key_size: (u16, u16),
    vt_version: VtVersion,
    soft_key_layout: SoftKeyLayout,
    object_id_ranges: crate::ObjectIdRanges,
}

/// Derived state belongs to one authoritative project state, never to its copies.
#[derive(Default)]
struct ProjectCache {
    state: Option<(ProjectRevision, DesignDimensions)>,
    materialized: Option<ComponentResult<Arc<MaterializedProject>>>,
    property_choices: HashMap<(ObjectId, String, String), bool>,
}

impl Clone for ProjectCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl ComponentCheckpoint {
    fn content_eq(&self, other: &Self) -> bool {
        self.pool == other.pool
            && self.source == other.source
            && self.graphics == other.graphics
            && self.info.len() == other.info.len()
            && self.info.iter().all(|(id, info)| {
                other
                    .info
                    .get(id)
                    .is_some_and(|other| info.content_eq(other))
            })
    }
}

#[derive(Clone)]
pub struct EditorProject {
    cache: RefCell<ProjectCache>,
    pool: ObjectPool,
    components: ComponentSource,
    graphics: crate::graphics::GraphicLibrary,
    component_history: HashMap<String, (ComponentCheckpoint, ComponentCheckpoint)>,
    dimension_history: HashMap<String, (DesignDimensions, DesignDimensions)>,
    selected_object: NullableObjectId,
    component_selection: Cell<Option<(ComponentInstanceId, ComponentNodeId)>>,
    pending_selected_object: Cell<NullableObjectId>,
    undo_selected_history: Vec<NullableObjectId>,
    redo_selected_history: Vec<NullableObjectId>,
    pub mask_size: u16,
    /// Authoritative project VT version. Old projects default to Version 6.
    pub(crate) vt_version: VtVersion,
    soft_key_size: (u16, u16),
    soft_key_layout: SoftKeyLayout,
    /// Application preference, excluded from project content and history.
    object_id_format: ObjectIdFormat,
    /// Application preference, excluded from project content and history.
    show_object_ids: bool,
    object_id_ranges: crate::ObjectIdRanges,
    object_info: RefCell<HashMap<ObjectId, ObjectInfo>>,
    /// Metadata draft edited by the UI and committed through operations.
    mut_object_info: RefCell<HashMap<ObjectId, ObjectInfo>>,

    /// Used to keep track of the object that is being renamed
    renaming_object: RefCell<Option<(eframe::egui::Id, ObjectId, String)>>,

    /// Cached default object names for efficient lookup
    default_object_names: RefCell<HashMap<ObjectId, String>>,

    /// Request to open image file dialog for PictureGraphic object
    image_load_request: RefCell<Option<ObjectId>>,
    graphic_assign_request: Cell<Option<(crate::graphics::GraphicAssetId, ObjectId)>>,
    graphic_options_request: RefCell<Option<(u16, bool, u8, bool)>>,
    graphic_format_request: RefCell<
        Option<(
            u16,
            ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat,
        )>,
    >,
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
    /// Monotonically increasing version of the authoritative project content.
    revision: Cell<ProjectRevision>,

    /// Bounded operation-based undo/redo history.
    pub operation_history: RefCell<OperationHistory>,
}

impl Default for EditorProject {
    fn default() -> Self {
        let mut project = Self::from(ObjectPool::new());
        project.mask_size = 500;
        project.mark_saved();
        project
    }
}

impl From<ObjectPool> for EditorProject {
    fn from(pool: ObjectPool) -> Self {
        let (mask_size, _) = pool.get_minimum_mask_sizes();
        let soft_key_size =
            pool.objects_by_type(ObjectType::Key)
                .iter()
                .fold((60, 60), |(width, height), key| {
                    let size = pool.content_size(key);
                    (width.max(size.0), height.max(size.1))
                });

        let mut project = EditorProject {
            cache: RefCell::default(),
            pool,
            components: Default::default(),
            graphics: Default::default(),
            component_history: Default::default(),
            dimension_history: Default::default(),
            selected_object: NullableObjectId::default(),
            component_selection: Cell::new(None),
            pending_selected_object: Cell::new(NullableObjectId::default()),
            undo_selected_history: Default::default(),
            redo_selected_history: Default::default(),
            mask_size: mask_size.max(1),
            vt_version: VtVersion::Version6,
            soft_key_size,
            soft_key_layout: SoftKeyLayout::default(),
            object_id_format: ObjectIdFormat::default(),
            show_object_ids: false,
            object_id_ranges: crate::ObjectIdRanges::defaults(),
            object_info: RefCell::new(HashMap::new()),
            mut_object_info: RefCell::new(HashMap::new()),
            renaming_object: RefCell::new(None),
            default_object_names: RefCell::new(HashMap::new()),
            image_load_request: RefCell::new(None),
            graphic_assign_request: Cell::new(None),
            graphic_options_request: RefCell::new(None),
            graphic_format_request: RefCell::new(None),
            delete_request: RefCell::new(None),
            rename_request: RefCell::new(None),
            queued_operations: RefCell::new(Vec::new()),
            last_operation_error: RefCell::new(None),
            saved_content: RefCell::new(None),
            dirty: Cell::new(false),
            revision: Cell::new(0),
            operation_history: RefCell::new(OperationHistory::new(10)), // Default 10 transaction history
        };
        project.apply_smart_naming_to_all_objects();
        project.mark_saved();
        project
    }
}

impl EditorProject {
    /// Import target-version IOP records atomically. Structurally valid but
    /// incompatible content is retained for repair and cannot be exported.
    pub fn import_iop(data: Vec<u8>, version: VtVersion) -> Result<Self, String> {
        Self::import_iop_with_id_format(data, version, ObjectIdFormat::default())
    }

    /// Import an IOP using the selected object-ID format for error messages
    /// and the resulting project's UI. The format does not affect wire data.
    pub fn import_iop_with_id_format(
        data: Vec<u8>,
        version: VtVersion,
        object_id_format: ObjectIdFormat,
    ) -> Result<Self, String> {
        if !(2..=6).contains(&u8::from(version)) {
            return Err("Choose VT2 through VT6".into());
        }
        let pool =
            ObjectPool::try_from_iop_for_version(data, version).map_err(|error| {
                match error.object_id {
                    Some(id) => format!(
                        "object {} at byte {}: {:?}",
                        object_id_format.format(id),
                        error.offset,
                        error.source
                    ),
                    None => format!("byte {}: {:?}", error.offset, error.source),
                }
            })?;
        let mut project = Self::from(pool);
        project.vt_version = version;
        project.object_id_format = object_id_format;
        project.mark_saved();
        Ok(project)
    }
    pub(crate) fn from_ai_source(state: &AiDesignState) -> Self {
        let mut project = Self::from(state.canonical_pool.as_ref().unwrap_or(&state.pool).clone());
        project.components = state.components.clone();
        project.graphics = state.graphics.clone();
        project.vt_version = state.vt_version;
        project.object_id_ranges = state.object_id_ranges.clone();
        project.mask_size = state.mask_size;
        project.soft_key_size = state.soft_key_size;
        project.soft_key_layout = state.soft_key_layout;
        project.object_info.replace(state.object_info.clone());
        project.mut_object_info.replace(state.object_info.clone());
        project
    }
    /// Copy authoritative canvas content without history, pending UI edits, or
    /// save checkpoints. Mutating this copy cannot change the source project.
    pub(crate) fn canvas_preview_copy(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            components: self.components.clone(),
            graphics: self.graphics.clone(),
            mask_size: self.mask_size,
            soft_key_size: self.soft_key_size,
            soft_key_layout: self.soft_key_layout,
            vt_version: self.vt_version,
            object_id_format: self.object_id_format,
            show_object_ids: self.show_object_ids,
            object_id_ranges: self.object_id_ranges.clone(),
            object_info: RefCell::new(self.object_info.borrow().clone()),
            mut_object_info: RefCell::new(self.object_info.borrow().clone()),
            ..Self::default()
        }
    }
    pub fn graphic_library(&self) -> &crate::graphics::GraphicLibrary {
        &self.graphics
    }

    /// Current application-level object-ID presentation preference.
    pub fn object_id_format(&self) -> ObjectIdFormat {
        self.object_id_format
    }

    /// Change ID presentation without changing content, revision or history.
    pub fn set_object_id_format(&mut self, format: ObjectIdFormat) {
        self.object_id_format = format;
    }

    /// Configured inclusive automatic allocation ranges.
    pub fn object_id_ranges(&self) -> &crate::ObjectIdRanges {
        &self.object_id_ranges
    }

    /// Apply ID ranges as a single undoable project settings change.
    pub fn set_object_id_ranges(&mut self, ranges: crate::ObjectIdRanges) -> Result<(), String> {
        self.set_project_configuration_with_id_ranges(
            self.mask_size,
            self.soft_key_size,
            self.vt_version,
            self.soft_key_layout,
            ranges,
        )
        .map_err(|error| error.to_string())
    }

    /// Restore all type ranges to their defaults as one undoable change.
    pub fn reset_object_id_ranges(&mut self) -> Result<(), String> {
        self.set_object_id_ranges(crate::ObjectIdRanges::defaults())
    }

    /// Update target version and ID ranges together, validating their combined
    /// constraints before changing project state.
    pub fn set_object_id_configuration(
        &mut self,
        version: VtVersion,
        ranges: crate::ObjectIdRanges,
    ) -> Result<(), String> {
        self.set_project_configuration_with_id_ranges(
            self.mask_size,
            self.soft_key_size,
            version,
            self.soft_key_layout,
            ranges,
        )
        .map_err(|error| error.to_string())
    }

    /// Whether the object's current ID was explicitly assigned by the user.
    pub fn object_id_is_overridden(&self, id: ObjectId) -> bool {
        self.mut_object_info
            .borrow()
            .get(&id)
            .is_some_and(|info| info.id_overridden)
    }

    /// Mark or clear an explicit ID assignment on an existing object.
    pub fn set_object_id_overridden(&mut self, id: ObjectId, overridden: bool) -> bool {
        if self.pool.object_by_id(id).is_none() {
            return false;
        }
        if self.object_id_is_overridden(id) == overridden {
            return true;
        }
        self.execute_ui_transaction(OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some(if overridden {
                format!("Keep object ID {} fixed", id.value())
            } else {
                format!("Return object ID {} to automatic assignment", id.value())
            }),
            operations: vec![Operation::SetObjectIdAssignment {
                object_id: id.value(),
                new_id: id.value(),
                overridden,
            }],
        })
    }

    /// Format an object identifier for the editor UI, including null sentinels.
    pub fn format_object_id(&self, id: u16) -> String {
        self.object_id_format.format(id)
    }

    /// Return the smart object name, optionally followed by its formatted ID.
    pub fn show_object_ids(&self) -> bool {
        self.show_object_ids
    }

    pub fn set_show_object_ids(&mut self, show: bool) {
        self.show_object_ids = show;
    }

    pub fn object_display_name(&self, object: &Object) -> String {
        let info = self
            .mut_object_info
            .borrow()
            .get(&object.id())
            .cloned()
            .unwrap_or_else(|| ObjectInfo::new(object));
        let name = info.name.unwrap_or_else(|| {
            smart_naming::get_object_type_name(object.object_type()).to_string()
        });
        if self.show_object_ids {
            format!("{} ({})", name, self.format_object_id(object.id().value()))
        } else {
            name
        }
    }
    pub fn component_source(&self) -> &ComponentSource {
        &self.components
    }
    pub(crate) fn component_object_info(&self) -> HashMap<ObjectId, ObjectInfo> {
        self.object_info.borrow().clone()
    }
    /// Return an independently mutable materialized pool and source map.
    pub fn materialize(&self) -> ComponentResult<MaterializedProject> {
        self.materialize_shared().map(|result| (*result).clone())
    }

    /// Share a read-only materialized snapshot until authoritative content or
    /// settings change. Errors are retained for the same project state too.
    /// The snapshot remains valid after subsequent edits to the project.
    pub fn materialize_shared(&self) -> ComponentResult<Arc<MaterializedProject>> {
        self.reconcile_cache();
        if let Some(result) = &self.cache.borrow().materialized {
            return result.clone();
        }
        let result = materialize_project(self).map(Arc::new);
        self.cache.borrow_mut().materialized = Some(result.clone());
        result
    }

    fn reconcile_cache(&self) {
        let state = (self.revision(), self.dimensions());
        let mut cache = self.cache.borrow_mut();
        if cache.state != Some(state.clone()) {
            *cache = ProjectCache {
                state: Some(state),
                ..Default::default()
            };
        }
    }

    fn invalidate_cache(&self) {
        self.cache.replace(ProjectCache::default());
    }

    /// Reuse UI choice validation without bypassing validation of committed edits.
    pub(crate) fn property_choice_is_valid(
        &self,
        id: ObjectId,
        property: &str,
        value: serde_json::Value,
    ) -> bool {
        self.reconcile_cache();
        let key = (id, property.to_owned(), value.to_string());
        if let Some(valid) = self.cache.borrow().property_choices.get(&key) {
            return *valid;
        }
        let mut transaction = OperationTransaction::new(None);
        transaction.add_operation(Operation::SetProperty {
            object: ObjectTarget::id(id.value()),
            property: property.to_owned(),
            value,
        });
        let valid = self.validate_transaction(&transaction).is_ok();
        self.cache.borrow_mut().property_choices.insert(key, valid);
        valid
    }
    /// Export the materialized pool, rejecting strings whose encoded byte
    /// length exceeds the ISO field limit.
    pub fn export_iop(&self) -> ComponentResult<Vec<u8>> {
        let materialized = self.materialize_shared()?;
        let diagnostics = self.compatibility_diagnostics_from_pool(&materialized.pool)?;
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == crate::ValidationSeverity::Error)
            .map(|d| crate::pool_validation::format_validation_diagnostic(d, self.object_id_format))
            .collect();
        if !errors.is_empty() {
            return Err(ComponentError::new(
                "VT_EXPORT_INCOMPATIBLE",
                errors.join("\n"),
            ));
        }
        let pool = crate::vt_strings::prepare_pool_for_export(&materialized.pool, self.vt_version)
            .map_err(|message| ComponentError::new("VT_STRING_ENCODING", message))?;
        pool.try_as_iop_for_version(self.vt_version)
            .map_err(|error| ComponentError::new("VT_EXPORT_INVALID", error.to_string()))
    }
    pub fn editor_selection(&self) -> Option<EditorSelection> {
        if let Some((instance_id, node_id)) = self.component_selection.get() {
            return Some(EditorSelection::ComponentNode {
                instance_id,
                node_id,
            });
        }
        let id = self.get_selected().0?.value();
        match self
            .materialize_shared()
            .ok()?
            .source_map
            .by_object_id
            .get(&id)?
        {
            MaterializedSource::Primitive { object_id } => {
                Some(EditorSelection::PrimitiveObject(*object_id))
            }
            MaterializedSource::ComponentNode {
                instance_id,
                node_id,
            } => Some(EditorSelection::ComponentNode {
                instance_id: *instance_id,
                node_id: *node_id,
            }),
        }
    }
    fn component_checkpoint(&self) -> ComponentCheckpoint {
        ComponentCheckpoint {
            pool: self.pool.clone(),
            source: self.components.clone(),
            graphics: self.graphics.clone(),
            info: self.object_info.borrow().clone(),
        }
    }
    fn restore_component_checkpoint(&mut self, state: ComponentCheckpoint) {
        self.pool = state.pool;
        self.components = state.source;
        self.graphics = state.graphics;
        if let Some((instance, node)) = self.component_selection.get() {
            if !self
                .components
                .instances
                .iter()
                .any(|i| i.id == instance && i.compiled_ids.contains_key(&node))
            {
                self.component_selection.set(None);
                if self
                    .selected_object
                    .0
                    .is_some_and(|id| self.pool.object_by_id(id).is_none())
                {
                    self.selected_object = NullableObjectId::NULL;
                    self.pending_selected_object.set(NullableObjectId::NULL);
                }
            }
        }
        self.sync_component_selection();
        self.object_info.replace(state.info.clone());
        self.mut_object_info.replace(state.info);
        self.default_object_names.borrow_mut().clear();
        self.refresh_dirty();
        self.bump_revision();
    }
    pub(crate) fn edit_components(
        &mut self,
        description: &str,
        edit: impl FnOnce(&mut ObjectPool, &mut ComponentSource) -> ComponentResult<()>,
    ) -> ComponentResult<()> {
        self.edit_source(description, |pool, source, _, _| edit(pool, source))
    }
    pub(crate) fn edit_source(
        &mut self,
        description: &str,
        edit: impl FnOnce(
            &mut ObjectPool,
            &mut ComponentSource,
            &mut crate::graphics::GraphicLibrary,
            &mut HashMap<ObjectId, ObjectInfo>,
        ) -> ComponentResult<()>,
    ) -> ComponentResult<()> {
        let before = self.component_checkpoint();
        let mut after = before.clone();
        edit(
            &mut after.pool,
            &mut after.source,
            &mut after.graphics,
            &mut after.info,
        )?;
        prepare_compiled_ids(&after.pool, &mut after.source, &self.object_id_ranges)?;
        let candidate = materialize_source_with_graphics(
            &after.pool,
            &after.source,
            self.vt_version,
            &after.info,
            &after.graphics,
            &self.object_id_ranges,
        )?;
        let baseline_materialized = self.materialize_shared().ok();
        let baseline = baseline_materialized
            .as_ref()
            .map(|m| &m.pool)
            .unwrap_or(&before.pool);
        let baseline_diagnostics = self.compatibility_diagnostics_from_pool(baseline)?;
        let mut candidate_project = self.canvas_preview_copy();
        candidate_project.pool = after.pool.clone();
        candidate_project.components = after.source.clone();
        candidate_project.graphics = after.graphics.clone();
        candidate_project.object_info.replace(after.info.clone());
        let candidate_diagnostics =
            candidate_project.compatibility_diagnostics_from_pool(&candidate.pool)?;
        let new_errors = crate::pool_validation::new_or_worsened_compatibility_diagnostics(
            &baseline_diagnostics,
            &candidate_diagnostics,
        );
        if let Some(error) = new_errors
            .iter()
            .find(|d| d.severity == crate::ValidationSeverity::Error)
        {
            return Err(ComponentError::new(
                "VT_COMPONENT_INCOMPATIBLE",
                error.message.clone(),
            ));
        }
        crate::layout_containment::validate_change_with_soft_key_size(
            baseline,
            &candidate.pool,
            self.mask_size,
            self.mask_size,
            self.soft_key_size,
            self.soft_key_size,
        )
        .map_err(containment_component_error)?;
        after
            .info
            .retain(|id, _| after.pool.object_by_id(*id).is_some());
        if before.content_eq(&after) {
            return Ok(());
        }
        let transaction_id = uuid::Uuid::new_v4().to_string();
        self.component_history
            .insert(transaction_id.clone(), (before, after.clone()));
        self.operation_history
            .borrow_mut()
            .push(AppliedTransaction {
                transaction_id,
                description: Some(description.into()),
                forward_operations: vec![],
                inverse_operations: vec![],
                affected_objects: vec![],
                execution_time_ms: 0,
            });
        self.prune_checkpoint_history();
        self.restore_component_checkpoint(after);
        Ok(())
    }

    /// Start an empty, unsaved pool with a usable editing canvas.
    pub fn new_pool() -> Self {
        let mut project = Self::from(ObjectPool::default());
        project.mask_size = 500;
        project.saved_content.replace(None);
        project.dirty.set(true);
        project
    }

    /// Get the current object pool
    pub fn get_pool(&self) -> &ObjectPool {
        &self.pool
    }

    /// Version of the canonical project content, for stale AI proposal detection.
    pub fn revision(&self) -> ProjectRevision {
        self.revision.get()
    }

    fn bump_revision(&self) {
        self.invalidate_cache();
        self.revision.set(self.revision.get().wrapping_add(1));
    }

    /// IDs already used or reserved by queued create operations, including
    /// deterministic reservations for queued automatic creates.
    fn reserved_object_ids(&self) -> std::collections::BTreeSet<u16> {
        let mut reserved: std::collections::BTreeSet<_> = self
            .pool
            .objects()
            .iter()
            .map(|object| object.id().value())
            .chain(
                self.components
                    .instances
                    .iter()
                    .flat_map(|instance| instance.compiled_ids.values().copied()),
            )
            .collect();
        let queued = self.queued_operations.borrow();
        reserved.extend(queued.iter().filter_map(|operation| match operation {
            Operation::CreateObject {
                object_id: Some(id),
                ..
            } => Some(*id),
            _ => None,
        }));
        for operation in queued.iter() {
            if let Operation::CreateObject {
                object_id: None,
                object_type,
                ..
            } = operation
            {
                let range = self.object_id_ranges.range_for(*object_type);
                if let Some(id) = (range.start..=range.end).find(|id| !reserved.contains(id)) {
                    reserved.insert(id);
                }
            }
        }
        reserved
    }

    /// Return the lowest free ID in this type's inclusive automatic range.
    /// Existing primitive, compiled component and queued creation IDs are
    /// reserved. Exhaustion returns an error without changing the project.
    pub fn allocate_object_id(
        &self,
        object_type: ObjectType,
    ) -> Result<ObjectId, ObjectIdAllocationError> {
        let range = self.object_id_ranges.range_for(object_type);
        let occupied = self.reserved_object_ids();
        let id = (range.start..=range.end)
            .find(|id| !occupied.contains(id))
            .ok_or(ObjectIdAllocationError { object_type, range })?;
        Ok(ObjectId::new(id).expect("validated automatic ID range"))
    }

    /// Build the deterministic global reassignment preview for the current project.
    pub fn plan_id_reassignment(
        &self,
        include_overrides: bool,
    ) -> Result<crate::IdReassignmentPlan, String> {
        let plan = crate::IdReassignmentPlan::build(
            &self.pool,
            &self.components,
            &self.object_info.borrow(),
            &self.object_id_ranges,
            self.vt_version,
            include_overrides,
        )?;
        if !plan.mappings.is_empty() {
            self.prepare_transaction(&OperationTransaction {
                schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
                description: Some("Preview object ID reassignment".into()),
                operations: vec![Operation::RemapObjectIds {
                    mappings: plan.mappings.clone(),
                    target_types: plan.target_types.clone(),
                }],
            })
            .map_err(|diagnostics| diagnostics.legacy_operation_error().to_string())?;
        }
        Ok(plan)
    }

    /// Apply a reassignment preview as one undoable, atomic project change.
    pub fn apply_id_reassignment(
        &mut self,
        plan: crate::IdReassignmentPlan,
    ) -> Result<bool, String> {
        if plan.mappings.is_empty() {
            return Ok(false);
        }
        self.execute_transaction(OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some("Reassign object IDs".into()),
            operations: vec![Operation::RemapObjectIds {
                mappings: plan.mappings,
                target_types: plan.target_types,
            }],
        })
        .map(|_| true)
        .map_err(|error| error.to_string())
    }

    /// Queue an explicit ID assignment from shared-reference UI code.
    pub fn queue_object_id_assignment(&self, id: ObjectId, new_id: ObjectId, overridden: bool) {
        self.queue_operation(Operation::SetObjectIdAssignment {
            object_id: id.value(),
            new_id: new_id.value(),
            overridden,
        });
    }

    /// Queue automatic reassignment to the first free ID in the object's type range.
    pub fn queue_use_automatic_object_id(&self, id: ObjectId) -> Result<ObjectId, String> {
        let object = self
            .pool
            .object_by_id(id)
            .ok_or_else(|| format!("Object {} no longer exists", id.value()))?;
        let range = self.object_id_ranges.range_for(object.object_type());
        let assigned = (range.start..=range.end)
            .find(|candidate| {
                *candidate != id.value()
                    && self
                        .pool
                        .object_by_id(ObjectId::new(*candidate).unwrap())
                        .is_none()
                    && !self.components.instances.iter().any(|instance| {
                        instance
                            .compiled_ids
                            .values()
                            .any(|compiled| compiled == candidate)
                    })
            })
            .ok_or_else(|| {
                format!(
                    "No fresh IDs remain in the {:?} range {}–{}",
                    object.object_type(),
                    range.start,
                    range.end
                )
            })?;
        self.queue_object_id_assignment(id, ObjectId::new(assigned).unwrap(), false);
        Ok(ObjectId::new(assigned).unwrap())
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
        let mut allowed_references: Vec<_> = object_info
            .iter()
            .filter(|(_, info)| !info.allowed_references.is_empty())
            .map(|(id, info)| (id.value(), info.allowed_references.clone()))
            .collect();
        allowed_references.sort_by_key(|(id, _)| *id);
        let mut overridden_ids: Vec<_> = object_info
            .iter()
            .filter(|(_, info)| info.id_overridden)
            .map(|(id, _)| id.value())
            .collect();
        overridden_ids.sort_unstable();

        ProjectContentCheckpoint {
            pool: self.pool.clone(),
            object_names,
            allowed_references,
            overridden_ids,
            mask_size: self.mask_size,
            soft_key_size: self.soft_key_size,
            soft_key_layout: self.soft_key_layout,
            vt_version: self.vt_version,
            object_id_ranges: self.object_id_ranges.clone(),
            components: self.components.clone(),
            graphics: self.graphics.clone(),
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

    /// Change the mask viewport size as one undo action, retaining softkey dimensions.
    ///
    /// Returns an error without changing content or revision if the new size
    /// would violate containment. Existing overflow may be retained or reduced.
    pub fn set_mask_size(&mut self, mask_size: u16) -> ComponentResult<()> {
        self.set_design_dimensions(mask_size, self.soft_key_size)
    }

    /// Saved softkey designator width and height in design pixels. Key contents
    /// use these dimensions as their editing and preview bounds.
    pub fn soft_key_size(&self) -> (u16, u16) {
        self.soft_key_size
    }

    /// Physical Soft Key Mask arrangement used by the authoring preview.
    pub fn soft_key_layout(&self) -> SoftKeyLayout {
        self.soft_key_layout
    }

    /// Authoritative target version used for authoring, preview and IOP export.
    pub fn vt_version(&self) -> VtVersion {
        self.vt_version
    }

    /// Change the target version atomically after compatibility validation.
    /// Failure leaves content, revision and undo history unchanged.
    pub fn set_vt_version(&mut self, version: VtVersion) -> ComponentResult<()> {
        self.set_project_settings(self.mask_size, self.soft_key_size, version)
    }

    /// Compatibility diagnostics for the materialized project, including components.
    pub fn compatibility_diagnostics(&self) -> ComponentResult<Vec<crate::ValidationDiagnostic>> {
        self.compatibility_diagnostics_from_pool(&self.materialize_shared()?.pool)
    }

    fn compatibility_diagnostics_from_pool(
        &self,
        pool: &ObjectPool,
    ) -> ComponentResult<Vec<crate::ValidationDiagnostic>> {
        let mut diagnostics =
            crate::pool_validation::validate_pool_state_for_version(pool, self.vt_version);
        // Master defaults must also be compatible when live instances override
        // them, or when a master is currently unused.
        let baseline =
            crate::pool_validation::validate_pool_state_for_version(&self.pool, self.vt_version);
        for definition in self.components.definitions.values() {
            let (preview, ids) = crate::components::preview_definition(self, definition)?;
            let candidate =
                crate::pool_validation::validate_pool_state_for_version(&preview, self.vt_version);
            for mut diagnostic in crate::pool_validation::new_or_worsened_compatibility_diagnostics(
                &baseline, &candidate,
            ) {
                // Preview IDs are disposable. Normalize them to stable node
                // identities so unrelated primitive allocation cannot change
                // the identity of an existing master-default violation.
                let mut affected_nodes = Vec::new();
                for (node, id) in &ids {
                    if diagnostic
                        .affected_objects
                        .as_ref()
                        .is_some_and(|affected| affected.contains(id))
                    {
                        affected_nodes.push(node);
                    }
                    if let Some(affected) = &mut diagnostic.affected_objects {
                        affected.retain(|affected| affected != id);
                    }
                }
                if !affected_nodes.is_empty() {
                    let nodes = affected_nodes
                        .iter()
                        .map(|node| node.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    diagnostic.message = format!("Node {nodes}: {}", diagnostic.message);
                }
                diagnostic.message = format!("Component {}: {}", definition.id, diagnostic.message);
                diagnostics.push(diagnostic);
            }
        }
        Ok(diagnostics)
    }

    fn dimensions(&self) -> DesignDimensions {
        DesignDimensions {
            mask_size: self.mask_size,
            soft_key_size: self.soft_key_size,
            vt_version: self.vt_version,
            soft_key_layout: self.soft_key_layout,
            object_id_ranges: self.object_id_ranges.clone(),
        }
    }

    fn restore_dimensions(&mut self, dimensions: DesignDimensions) {
        self.mask_size = dimensions.mask_size;
        self.soft_key_size = dimensions.soft_key_size;
        self.vt_version = dimensions.vt_version;
        self.soft_key_layout = dimensions.soft_key_layout;
        self.object_id_ranges = dimensions.object_id_ranges;
        self.bump_revision();
        self.refresh_dirty();
    }

    fn prune_checkpoint_history(&mut self) {
        let history = self.operation_history.borrow();
        self.component_history.retain(|id, _| history.contains(id));
        self.dimension_history.retain(|id, _| history.contains(id));
    }

    /// Atomically apply positive mask and softkey dimensions as one undo action.
    /// Mask or key reductions cannot introduce or increase placement overflow; no
    /// objects are scaled. Failure or a no-op leaves content and revision intact.
    /// Standard terminal minima are advisory, not limits on design pixels.
    pub fn set_design_dimensions(
        &mut self,
        mask_size: u16,
        soft_key_size: (u16, u16),
    ) -> ComponentResult<()> {
        self.set_project_settings(mask_size, soft_key_size, self.vt_version)
    }

    /// Apply dimensions and VT2–VT6 as one undo action. A version change
    /// requires a compatible materialized pool; dimensions never scale objects.
    pub fn set_project_settings(
        &mut self,
        mask_size: u16,
        soft_key_size: (u16, u16),
        vt_version: VtVersion,
    ) -> ComponentResult<()> {
        self.set_project_configuration(mask_size, soft_key_size, vt_version, self.soft_key_layout)
    }

    /// Apply mask dimensions, key dimensions, VT version and softkey layout
    /// atomically as one undo action.
    pub fn set_project_configuration(
        &mut self,
        mask_size: u16,
        soft_key_size: (u16, u16),
        vt_version: VtVersion,
        soft_key_layout: SoftKeyLayout,
    ) -> ComponentResult<()> {
        self.set_project_configuration_with_id_ranges(
            mask_size,
            soft_key_size,
            vt_version,
            soft_key_layout,
            self.object_id_ranges.clone(),
        )
    }

    /// Apply project dimensions, target version, layout, and per-type ID
    /// allocation ranges atomically as one undo action.
    pub fn set_project_configuration_with_id_ranges(
        &mut self,
        mask_size: u16,
        soft_key_size: (u16, u16),
        vt_version: VtVersion,
        soft_key_layout: SoftKeyLayout,
        object_id_ranges: crate::ObjectIdRanges,
    ) -> ComponentResult<()> {
        if !(2..=6).contains(&u8::from(vt_version)) {
            return Err(ComponentError::new(
                "UNSUPPORTED_VT_VERSION",
                "Choose VT2 through VT6",
            ));
        }
        if mask_size == 0 || soft_key_size.0 == 0 || soft_key_size.1 == 0 {
            return Err(ComponentError::new(
                "INVALID_DESIGN_DIMENSIONS",
                "Design dimensions must be greater than zero",
            ));
        }
        soft_key_layout
            .validate()
            .map_err(|message| ComponentError::new("INVALID_SOFT_KEY_LAYOUT", message))?;
        object_id_ranges
            .validate(vt_version)
            .map_err(|message| ComponentError::new("INVALID_OBJECT_ID_RANGES", message))?;
        let before = self.dimensions();
        let after = DesignDimensions {
            mask_size,
            soft_key_size,
            vt_version,
            soft_key_layout,
            object_id_ranges,
        };
        if before == after {
            return Ok(());
        }
        if self.vt_version != vt_version {
            let mut candidate = self.canvas_preview_copy();
            candidate.vt_version = vt_version;
            let materialized = candidate.materialize_shared()?;
            let diagnostics = candidate.compatibility_diagnostics_from_pool(&materialized.pool)?;
            let errors: Vec<_> = diagnostics
                .iter()
                .filter(|d| d.severity == crate::ValidationSeverity::Error)
                .map(|d| {
                    crate::pool_validation::format_validation_diagnostic(d, self.object_id_format)
                })
                .collect();
            if !errors.is_empty() {
                return Err(ComponentError::new(
                    "VT_VERSION_INCOMPATIBLE",
                    errors.join("\n"),
                ));
            }
            crate::vt_strings::prepare_pool_for_export(&materialized.pool, vt_version)
                .and_then(|pool| {
                    pool.try_as_iop_for_version(vt_version)
                        .map_err(|e| e.to_string())
                })
                .map_err(|message| ComponentError::new("VT_VERSION_INCOMPATIBLE", message))?;
        }
        if self.mask_size != mask_size || self.soft_key_size != soft_key_size {
            let materialized = self.materialize_shared()?;
            crate::layout_containment::validate_change_with_soft_key_size(
                &materialized.pool,
                &materialized.pool,
                self.mask_size,
                mask_size,
                self.soft_key_size,
                soft_key_size,
            )
            .map_err(containment_component_error)?;
        }
        let transaction_id = uuid::Uuid::new_v4().to_string();
        self.dimension_history
            .insert(transaction_id.clone(), (before, after.clone()));
        self.operation_history
            .borrow_mut()
            .push(AppliedTransaction {
                transaction_id,
                description: Some(
                    if self.vt_version != vt_version || self.soft_key_layout != soft_key_layout {
                        "Change project settings".into()
                    } else {
                        "Change design dimensions".into()
                    },
                ),
                forward_operations: vec![],
                inverse_operations: vec![],
                affected_objects: vec![],
                execution_time_ms: 0,
            });
        self.prune_checkpoint_history();
        self.restore_dimensions(after);
        Ok(())
    }

    pub fn set_object_name(&self, object: &Object, name: String) {
        let mut object_info = self.mut_object_info.borrow_mut();
        if !name.is_empty()
            && object_info
                .iter()
                .any(|(id, info)| *id != object.id() && info.name.as_deref() == Some(name.as_str()))
        {
            self.last_operation_error
                .replace(Some(format!("Object name already exists: {name}")));
            return;
        }
        let info = object_info
            .entry(object.id())
            .or_insert_with(|| ObjectInfo::new(object));
        let old_name = info.name.clone();
        info.set_name(name);
        if info.name != old_name {
            drop(object_info);
            self.refresh_dirty();
            self.bump_revision();
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
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
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
        let object_id = self.allocate_object_id(object_type).ok()?;
        let transaction = OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some(format!("Create {object_type:?}")),
            operations: vec![Operation::CreateObject {
                object_id: None,
                object_type,
                name,
            }],
        };

        self.execute_ui_transaction(transaction)
            .then_some(object_id)
    }

    /// Duplicate a shared Font, Line, or Fill Attribute as one undoable action.
    ///
    /// Copies writable properties and outgoing references into a uniquely named
    /// object with a fresh automatic ID from its configured range. Consumers keep
    /// referencing `source_id`; editor metadata other than the name is not copied.
    /// Returns the new ID, or an error for a missing/unsupported source, exhausted
    /// range, or invalid copied content. Failure leaves the project unchanged.
    pub fn duplicate_shared_attribute(
        &mut self,
        source_id: ObjectId,
    ) -> Result<ObjectId, OperationError> {
        let source = self
            .pool
            .object_by_id(source_id)
            .ok_or(OperationError::ObjectNotFound(source_id.value()))?
            .clone();
        let object_type = source.object_type();
        if !matches!(
            object_type,
            ObjectType::FontAttributes | ObjectType::LineAttributes | ObjectType::FillAttributes
        ) {
            return Err(OperationError::PropertyError {
                object_id: source_id.value(),
                property: "duplicate".to_owned(),
                reason: "Only Font, Line, and Fill Attributes can be duplicated here".to_owned(),
            });
        }

        let new_id = self.allocate_object_id(object_type).map_err(|error| {
            OperationError::PropertyError {
                object_id: source_id.value(),
                property: "CreateObject".to_owned(),
                reason: error.to_string(),
            }
        })?;
        let source_name = self
            .object_info
            .borrow()
            .get(&source_id)
            .map(|info| info.get_name(&source))
            .unwrap_or_else(|| {
                format!(
                    "{} {}",
                    smart_naming::get_object_type_name(object_type),
                    source_id.value()
                )
            });
        let names = self.get_all_object_names();
        let base_name = format!("Copy of {source_name}");
        let mut name = base_name.clone();
        let mut suffix = 2u32;
        while names.contains_key(&name) {
            name = format!("{base_name} ({suffix})");
            suffix += 1;
        }

        let mut operations = vec![
            Operation::CreateObject {
                object_id: Some(new_id.value()),
                object_type,
                name: name.clone(),
            },
            // An explicit ID is used to reserve the already selected range ID;
            // reset the metadata so it behaves like an automatic assignment.
            Operation::SetObjectIdAssignment {
                object_id: new_id.value(),
                new_id: new_id.value(),
                overridden: false,
            },
        ];

        for descriptor in crate::object_properties::get_property_descriptors(object_type)
            .into_iter()
            .filter(|descriptor| descriptor.writable)
        {
            let value = crate::object_properties::get_object_property(&source, descriptor.name)
                .map_err(|error| OperationError::PropertyError {
                    object_id: source_id.value(),
                    property: descriptor.name.to_owned(),
                    reason: error.to_string(),
                })?;
            let target = ObjectTarget::Name { name: name.clone() };
            if matches!(
                descriptor.semantic,
                crate::object_properties::PropertySemantic::ObjectReference { .. }
            ) {
                let reference = if value.is_null() {
                    None
                } else {
                    let reference_id: ObjectId = serde_json::from_value(value).map_err(|error| {
                        OperationError::PropertyError {
                            object_id: source_id.value(),
                            property: descriptor.name.to_owned(),
                            reason: error.to_string(),
                        }
                    })?;
                    Some(ObjectTarget::id(reference_id.value()))
                };
                operations.push(Operation::SetObjectReference {
                    object: target,
                    property: descriptor.name.to_owned(),
                    reference,
                });
            } else {
                operations.push(Operation::SetProperty {
                    object: target,
                    property: descriptor.name.to_owned(),
                    value,
                });
            }
        }

        if let Some(references) = crate::operations::macro_refs(&source) {
            operations.push(Operation::SetMacroReferences {
                object: ObjectTarget::Name { name: name.clone() },
                macro_refs: references
                    .iter()
                    .map(|reference| crate::operations::MacroReferenceTarget {
                        event_id: reference.event_id,
                        macro_object: ObjectTarget::id(u16::from(reference.macro_id)),
                    })
                    .collect(),
            });
        }

        self.execute_transaction(OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some(format!("Duplicate {source_name}")),
            operations,
        })?;
        Ok(new_id)
    }

    /// Delete an object through a reversible operation.
    pub fn delete_object(&mut self, object_id: ObjectId) -> bool {
        let deleted = self.execute_ui_transaction(OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some(format!("Delete object {}", object_id.value())),
            operations: vec![Operation::DeleteObject {
                object_id: object_id.value(),
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

    /// Defer inspector assignment until same-frame property edits are committed.
    pub fn request_graphic_assignment(
        &self,
        asset: crate::graphics::GraphicAssetId,
        target: ObjectId,
    ) {
        self.graphic_assign_request.set(Some((asset, target)));
    }

    pub fn commit_graphic_assignment(&mut self) -> bool {
        let Some((asset, target)) = self.graphic_assign_request.take() else {
            return false;
        };
        match crate::graphics::integration::GraphicService::convert_asset_into_picture_graphic(
            self,
            asset,
            target.value(),
        ) {
            Ok(()) => true,
            Err(error) => {
                self.last_operation_error.replace(Some(error.to_string()));
                false
            }
        }
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
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some(format!("Rename object {}", object_id.value())),
            operations: vec![Operation::RenameObject {
                object: ObjectTarget::id(object_id.value()),
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
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
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
                self.last_operation_error.replace(Some(error.to_string()));
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
            self.sync_component_selection();
            return true;
        }
        false
    }

    fn sync_component_selection(&self) {
        let map = source_map(&self.components);
        self.component_selection
            .set(
                self.selected_object
                    .0
                    .and_then(|id| match map.by_object_id.get(&id.value()) {
                        Some(MaterializedSource::ComponentNode {
                            instance_id,
                            node_id,
                        }) => Some((*instance_id, *node_id)),
                        _ => None,
                    }),
            );
    }

    /// Set the selected object to the previous object in the history
    pub fn set_previous_selected(&mut self) {
        if let Some(selected) = self.undo_selected_history.pop() {
            self.redo_selected_history.push(self.selected_object);
            // Both need to be replaced here because otherwise it will be added to the undo history
            self.selected_object = selected.clone();
            self.pending_selected_object.set(selected);
            self.sync_component_selection();
        }
    }

    /// Set the selected object to the next object in the history
    pub fn set_next_selected(&mut self) {
        if let Some(selected) = self.redo_selected_history.pop() {
            self.undo_selected_history.push(self.selected_object);
            // Both need to be replaced here because otherwise the redo history will be cleared
            self.selected_object = selected.clone();
            self.pending_selected_object.set(selected);
            self.sync_component_selection();
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

        let mut type_counts = [0usize; 256];
        for object_type in existing_names.values() {
            type_counts[u8::from(*object_type) as usize] += 1;
        }

        // Generate names for remaining objects without recounting the whole map.
        for object in self.pool.objects() {
            let object_type = object.object_type();
            if object_info
                .get(&object.id())
                .is_some_and(|info| info.name.is_some())
            {
                continue;
            }
            let new_name = smart_naming::generate_smart_name_from_count(
                object_type,
                &existing_names,
                type_counts[u8::from(object_type) as usize],
            );

            // Update the count for the new name to ensure uniqueness
            existing_names
                .entry(new_name.clone())
                .or_insert(object_type);
            type_counts[u8::from(object_type) as usize] += 1;

            let info = object_info
                .entry(object.id())
                .or_insert_with(|| ObjectInfo::new(object));
            info.set_name(new_name);
        }
        drop(object_info);
        self.object_info
            .replace(self.mut_object_info.borrow().clone());
        self.invalidate_cache();
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
        self.invalidate_cache();
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

        let mut project = ProjectFile::new_with_id_ranges(
            &self.pool,
            &object_info,
            self.mask_size,
            self.soft_key_size,
            self.soft_key_layout,
            self.vt_version,
            selected,
            self.object_id_ranges.clone(),
        )?;
        project.components = self.components.clone();
        project.graphics = self.graphics.clone();
        project.to_bytes()
    }

    /// Load a project from file data
    pub fn load_project(data: Vec<u8>) -> Result<Self, String> {
        let mut project = ProjectFile::from_bytes(&data)
            .map_err(|e| format!("Failed to parse project file: {}", e))?;
        let mut pool = project.load_pool()?;
        for (object_id, binding) in std::mem::take(&mut project.graphics.legacy_bindings) {
            let asset = project
                .graphics
                .assets
                .get(&binding.asset_id)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "Legacy graphic binding references missing asset {}",
                        binding.asset_id
                    )
                })?;
            let picture = ObjectId::new(object_id)
                .ok()
                .and_then(|id| pool.object_mut_by_id(id));
            let Some(ag_iso_stack::object_pool::object::Object::PictureGraphic(picture)) = picture
            else {
                return Err(format!(
                    "Legacy graphic binding target {object_id} is not a PictureGraphic"
                ));
            };
            crate::graphics::apply_image_bytes_to_picture_graphic(picture, &asset.original_bytes)?;
            if let Some(transparent) = binding.transparent {
                picture.options.transparent = transparent;
            }
            if let Some(colour) = binding.transparency_colour {
                picture.transparency_colour = colour;
            }
            if let Some(flashing) = binding.flashing {
                picture.options.flashing = flashing;
            }
            if let Some(format) = binding.format {
                crate::graphics::convert_picture_graphic_format(picture, format);
            }
            project.graphics.assets.remove(&binding.asset_id);
        }
        let settings = project.get_settings();

        let mut editor_project = EditorProject::from(pool);
        editor_project.mask_size = settings.mask_size;
        if let Some(soft_key_size) = settings.soft_key_size {
            editor_project.soft_key_size = soft_key_size;
        }
        editor_project.soft_key_layout = settings.soft_key_layout;
        editor_project.vt_version = settings.vt_version();
        editor_project.object_id_ranges = settings.object_id_ranges.clone();

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
                info.description = meta.description.clone();
                info.tags = meta.tags.clone();
                info.allowed_references = meta.allowed_references.clone();
                info.id_overridden = meta.id_overridden;
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

        editor_project.components = project.components.clone();
        editor_project.graphics = project.graphics.clone();
        materialize_project(&editor_project).map_err(|e| e.to_string())?;
        editor_project.sync_component_selection();
        editor_project.mark_saved();

        Ok(editor_project)
    }

    pub fn request_graphic_options(&self, id: u16, transparent: bool, colour: u8, flashing: bool) {
        self.graphic_options_request
            .replace(Some((id, transparent, colour, flashing)));
    }
    pub fn take_graphic_options_request(&self) -> Option<(u16, bool, u8, bool)> {
        self.graphic_options_request.take()
    }
    pub fn request_graphic_format(
        &self,
        id: u16,
        format: ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat,
    ) {
        self.graphic_format_request.replace(Some((id, format)));
    }
    pub fn take_graphic_format_request(
        &self,
    ) -> Option<(
        u16,
        ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat,
    )> {
        self.graphic_format_request.take()
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
        let prepared = self
            .prepare_transaction(&transaction)
            .map_err(|diagnostics| diagnostics.legacy_operation_error())?;
        self.commit_prepared_transaction(prepared)
    }

    fn commit_prepared_transaction(
        &mut self,
        prepared: PreparedTransaction,
    ) -> Result<AppliedTransaction, OperationError> {
        let PreparedTransaction {
            pool: modified_pool,
            object_info: modified_object_info,
            applied,
            ..
        } = prepared;

        // A deleted graphic becomes an available PNG. Stage all exports before
        // changing the project, then use the existing full-project history for
        // the pool deletion and library insertion as a single undo step.
        let mut recovered = Vec::new();
        for operation in &applied.inverse_operations {
            if let Operation::RestoreObject {
                object: Object::PictureGraphic(picture),
                info,
            } = operation
            {
                // A newly created, empty PictureGraphic has no image to recover.
                if picture.actual_width == 0 || picture.actual_height == 0 {
                    continue;
                }
                let asset = crate::graphics::integration::recover_picture_graphic(
                    picture,
                    &self.pool,
                    info.as_ref(),
                )
                .map_err(|reason| OperationError::PropertyError {
                    object_id: picture.id.value(),
                    property: "data".into(),
                    reason,
                })?;
                recovered.push(asset);
            }
        }
        let id_remap = crate::id_reassignment::compose_id_mappings(&applied.forward_operations);
        let before =
            (!recovered.is_empty() || !id_remap.is_empty()).then(|| self.component_checkpoint());

        if !id_remap.is_empty() {
            crate::id_reassignment::remap_component_source(
                &mut self.components,
                &id_remap,
                &self.pool,
                self.vt_version,
            )
            .expect("prepared component ID remap must remain valid");
            let map: HashMap<_, _> = id_remap.iter().copied().collect();
            let old_bindings = std::mem::take(&mut self.graphics.legacy_bindings);
            self.graphics.legacy_bindings = old_bindings
                .into_iter()
                .map(|(id, binding)| (map.get(&id).copied().unwrap_or(id), binding))
                .collect();
        }
        for asset in recovered {
            self.graphics.assets.insert(asset.id, asset);
        }

        // Replace pool and metadata with results after component Macro streams
        // have been resolved against the original identities.
        self.pool = modified_pool;

        self.object_info.replace(modified_object_info);
        self.mut_object_info
            .replace(self.object_info.borrow().clone());
        self.default_object_names.borrow_mut().clear();
        self.remap_selection_for_operations(&applied.forward_operations);

        if let Some(before) = before {
            self.component_history.insert(
                applied.transaction_id.clone(),
                (before, self.component_checkpoint()),
            );
        }
        // Record in history (this is a user action)
        self.operation_history.borrow_mut().push(applied.clone());
        self.prune_checkpoint_history();

        // Mark dirty and refresh state
        self.refresh_dirty();
        self.bump_revision();

        Ok(applied)
    }

    /// Prepare a transaction against a cloned candidate state without mutating this project.
    pub fn prepare_transaction(
        &self,
        transaction: &OperationTransaction,
    ) -> Result<PreparedTransaction, TransactionDiagnostics> {
        let mut transaction = transaction.clone();
        for operation in &transaction.operations {
            if let Operation::RemapObjectIds { mappings, .. } = operation {
                let compiled: HashSet<_> = self
                    .components
                    .instances
                    .iter()
                    .flat_map(|instance| instance.compiled_ids.values().copied())
                    .collect();
                if let Some((old_id, _)) = mappings.iter().find(|(old_id, _)| {
                    ObjectId::new(*old_id)
                        .ok()
                        .is_none_or(|id| self.pool.object_by_id(id).is_none())
                        && !compiled.contains(old_id)
                }) {
                    return Err(component_transaction_error(ComponentError::new(
                        "OBJECT_NOT_FOUND",
                        format!("Object ID {old_id} is no longer present"),
                    )));
                }
            }
        }
        let mut reserved = self.reserved_object_ids();
        // Reserve explicit operation IDs before choosing any automatic IDs, so
        // a preceding automatic create cannot consume a later explicit ID.
        reserved.extend(
            transaction
                .operations
                .iter()
                .filter_map(|operation| match operation {
                    Operation::CreateObject {
                        object_id: Some(id),
                        ..
                    } => Some(*id),
                    _ => None,
                }),
        );
        let mut create_id_overrides = HashMap::new();
        for operation in &mut transaction.operations {
            if let Operation::CreateObject {
                object_id,
                object_type,
                ..
            } = operation
            {
                let explicit = object_id.is_some();
                if object_id.is_none() {
                    let range = self.object_id_ranges.range_for(*object_type);
                    *object_id = (range.start..=range.end).find(|id| !reserved.contains(id));
                    if object_id.is_none() {
                        return Err(component_transaction_error(ComponentError::new(
                            "OBJECT_ID_RANGE_EXHAUSTED",
                            ObjectIdAllocationError {
                                object_type: *object_type,
                                range,
                            }
                            .to_string(),
                        )));
                    }
                }
                let object_id = object_id.unwrap();
                create_id_overrides.insert(object_id, explicit);
                reserved.insert(object_id);
            }
        }
        let mut normalized_operations = Vec::with_capacity(transaction.operations.len());
        for operation in std::mem::take(&mut transaction.operations) {
            let auto_assigned = match &operation {
                Operation::CreateObject {
                    object_id: Some(id),
                    ..
                } => create_id_overrides.get(id) == Some(&false),
                _ => false,
            };
            let assigned_id = match &operation {
                Operation::CreateObject {
                    object_id: Some(id),
                    ..
                } if auto_assigned => Some(*id),
                _ => None,
            };
            normalized_operations.push(operation);
            if let Some(id) = assigned_id {
                normalized_operations.push(Operation::SetObjectIdAssignment {
                    object_id: id,
                    new_id: id,
                    overridden: false,
                });
            }
        }
        // Supply authoritative compiled object types to individual remaps too.
        // Keep identity changes sequential across the transaction and simultaneous
        // within each batch, including when command values target component nodes.
        let mut target_types: HashMap<u16, ObjectType> = self
            .pool
            .objects()
            .iter()
            .map(|object| (object.id().value(), object.object_type()))
            .collect();
        for instance in &self.components.instances {
            if let Some(definition) = self.components.definitions.get(&instance.component_id) {
                for node in &definition.nodes {
                    if let Some(id) = instance.compiled_ids.get(&node.id) {
                        target_types.insert(*id, node.object_type);
                    }
                }
            }
        }
        let mut identity_operations = Vec::with_capacity(normalized_operations.len());
        for operation in normalized_operations {
            let (mappings, assignment) = match &operation {
                Operation::ChangeObjectId { old_id, new_id } if old_id != new_id => {
                    (vec![(*old_id, *new_id)], None)
                }
                Operation::SetObjectIdAssignment {
                    object_id,
                    new_id,
                    overridden,
                } if object_id != new_id => {
                    (vec![(*object_id, *new_id)], Some((*new_id, *overridden)))
                }
                Operation::RemapObjectIds { mappings, .. } => (mappings.clone(), None),
                _ => (vec![], None),
            };
            if mappings.is_empty() {
                if let Operation::CreateObject {
                    object_id: Some(id),
                    object_type,
                    ..
                } = &operation
                {
                    target_types.insert(*id, *object_type);
                }
                identity_operations.push(operation);
            } else {
                let types = target_types.iter().map(|(id, ty)| (*id, *ty)).collect();
                identity_operations.push(Operation::RemapObjectIds {
                    mappings: mappings.clone(),
                    target_types: types,
                });
                let map: HashMap<_, _> = mappings.into_iter().collect();
                target_types = target_types
                    .into_iter()
                    .map(|(id, ty)| (map.get(&id).copied().unwrap_or(id), ty))
                    .collect();
                if let Some((id, overridden)) = assignment {
                    identity_operations.push(Operation::SetObjectIdAssignment {
                        object_id: id,
                        new_id: id,
                        overridden,
                    });
                }
            }
        }
        transaction.operations = identity_operations;
        let prepared = OperationExecutor::new().prepare_transaction_with_soft_key_size(
            &self.pool,
            &self.object_info.borrow(),
            &transaction,
            &ValidationContext {
                vt_version: self.vt_version,
                mask_size: self.mask_size,
            },
            self.soft_key_size,
        )?;
        let mut candidate_components = self.components.clone();
        let id_changes = crate::id_reassignment::compose_id_mappings(&transaction.operations);
        if !id_changes.is_empty() {
            crate::id_reassignment::remap_component_source(
                &mut candidate_components,
                &id_changes,
                &self.pool,
                self.vt_version,
            )
            .map_err(|error| {
                component_transaction_error(ComponentError::new("COMPONENT_ID_REMAP", error))
            })?;
        }
        let candidate = materialize_source_with_graphics(
            &prepared.pool,
            &candidate_components,
            self.vt_version,
            &prepared.object_info,
            &self.graphics,
            &self.object_id_ranges,
        )
        .map_err(component_transaction_error)?;
        let baseline_materialized = self.materialize_shared().ok();
        let baseline = baseline_materialized
            .as_ref()
            .map(|m| &m.pool)
            .unwrap_or(&self.pool);
        let mut baseline_diagnostics = self
            .compatibility_diagnostics_from_pool(baseline)
            .map_err(component_transaction_error)?;
        crate::pool_validation::remap_compatibility_diagnostic_ids(
            &mut baseline_diagnostics,
            &id_changes,
        );
        let mut candidate_project = self.canvas_preview_copy();
        candidate_project.pool = prepared.pool.clone();
        candidate_project.components = candidate_components;
        candidate_project
            .object_info
            .replace(prepared.object_info.clone());
        let candidate_diagnostics = candidate_project
            .compatibility_diagnostics_from_pool(&candidate.pool)
            .map_err(component_transaction_error)?;
        let new_errors = crate::pool_validation::new_or_worsened_compatibility_diagnostics(
            &baseline_diagnostics,
            &candidate_diagnostics,
        );
        if let Some(error) = new_errors
            .iter()
            .find(|d| d.severity == crate::ValidationSeverity::Error)
        {
            return Err(component_transaction_error(ComponentError::new(
                "VT_PROJECT_INCOMPATIBLE",
                error.message.clone(),
            )));
        }
        crate::layout_containment::validate_change_with_id_changes_and_soft_key_size(
            baseline,
            &candidate.pool,
            self.mask_size,
            self.mask_size,
            &id_changes,
            Some((self.soft_key_size, self.soft_key_size)),
        )
        .map_err(crate::operations::executor::containment_transaction_error)?;
        Ok(prepared)
    }

    /// Validation is deliberately preparation with its candidate discarded.
    pub fn validate_transaction(
        &self,
        transaction: &OperationTransaction,
    ) -> Result<(), TransactionDiagnostics> {
        self.prepare_transaction(transaction).map(|_| ())
    }

    /// Undo the last transaction via operation history
    /// Does NOT go through normal executor; applies inverse directly
    /// Returns true if undo succeeded, false if no undo available or undo failed
    pub fn undo_operation(&mut self) -> bool {
        let mut history = self.operation_history.borrow_mut();

        if let Some(applied_tx) = history.undo() {
            if let Some((before, _)) = self
                .dimension_history
                .get(&applied_tx.transaction_id)
                .cloned()
            {
                drop(history);
                self.restore_dimensions(before);
                return true;
            }
            if let Some((before, _)) = self
                .component_history
                .get(&applied_tx.transaction_id)
                .cloned()
            {
                drop(history);
                self.restore_component_checkpoint(before);
                self.remap_selection_for_operations(&applied_tx.create_inverse().operations);
                return true;
            }
            let inverse_tx = applied_tx.create_inverse();

            // Apply inverse operations directly without creating new history entry
            let mut pool = self.pool.clone();
            let mut object_info = self.object_info.borrow().clone();
            let mut context = crate::operations::OperationContext {
                vt_version: self.vt_version,
                ..Default::default()
            };

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
            self.default_object_names.borrow_mut().clear();
            drop(history);
            self.remap_selection_for_operations(&inverse_tx.operations);
            self.refresh_dirty();
            self.bump_revision();
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
            if let Some((_, after)) = self
                .dimension_history
                .get(&applied_tx.transaction_id)
                .cloned()
            {
                drop(history);
                self.restore_dimensions(after);
                return true;
            }
            if let Some((_, after)) = self
                .component_history
                .get(&applied_tx.transaction_id)
                .cloned()
            {
                drop(history);
                self.restore_component_checkpoint(after);
                self.remap_selection_for_operations(&applied_tx.forward_operations);
                return true;
            }
            let mut pool = self.pool.clone();
            let mut object_info = self.object_info.borrow().clone();
            let mut context = crate::operations::OperationContext {
                vt_version: self.vt_version,
                ..Default::default()
            };

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
            self.default_object_names.borrow_mut().clear();
            drop(history);
            self.remap_selection_for_operations(&applied_tx.forward_operations);
            self.refresh_dirty();
            self.bump_revision();
            return true;
        }
        false
    }

    fn remap_selection_for_operations(&mut self, operations: &[Operation]) {
        for operation in operations {
            let mappings: Vec<_> = match operation {
                Operation::ChangeObjectId { old_id, new_id }
                | Operation::SetObjectIdAssignment {
                    object_id: old_id,
                    new_id,
                    ..
                } => vec![(*old_id, *new_id)],
                Operation::RemapObjectIds { mappings, .. } => mappings.clone(),
                _ => continue,
            };
            let map_selection = |selection: NullableObjectId| {
                selection.0.map_or(selection, |id| {
                    mappings
                        .iter()
                        .find(|(old_id, _)| *old_id == id.value())
                        .map_or(selection, |(_, new_id)| NullableObjectId::new(*new_id))
                })
            };
            self.selected_object = map_selection(self.selected_object);
            self.pending_selected_object
                .set(map_selection(self.pending_selected_object.get()));
        }
    }

    /// Create a snapshot for AI context
    /// Owned, UI-free input for automation domain operations.
    pub fn ai_design_state(&self) -> AiDesignState {
        AiDesignState {
            revision: self.revision(),
            canonical_pool: Some(self.pool.clone()),
            components: self.components.clone(),
            graphics: self.graphics.clone(),
            pool: self
                .materialize()
                .expect("Committed source must materialize")
                .pool,
            object_info: self.object_info.borrow().clone(),
            vt_version: self.vt_version,
            mask_size: self.mask_size,
            soft_key_size: self.soft_key_size,
            soft_key_layout: self.soft_key_layout,
            object_id_ranges: self.object_id_ranges.clone(),
            selected: self.selected_object.0,
        }
    }

    /// Backwards-compatible default AI snapshot.
    pub fn snapshot(&self) -> Result<DesignSnapshot, SnapshotError> {
        crate::ai::get_design_context(&self.ai_design_state(), DesignContextRequest::default())
            .map_err(|error| SnapshotError::SerializationFailed(format!("{error:?}")))
    }

    /// Apply an AI proposal without allowing a stale prepared pool to overwrite edits.
    pub fn apply_ai_proposal(
        &mut self,
        proposal: AiProposal,
    ) -> Result<AppliedTransaction, TransactionDiagnostics> {
        if proposal.base_revision != self.revision() {
            return Err(component_transaction_error(ComponentError::new(
                "STALE_PROPOSAL",
                "Project changed since this proposal was prepared",
            )));
        }
        if !proposal.component_intents.is_empty() {
            ComponentService::apply_intents(self, &proposal.component_intents)
                .map_err(component_transaction_error)?;
            return Ok(self
                .operation_history
                .borrow()
                .latest()
                .expect("Component action recorded")
                .clone());
        }
        let prepared = self.prepare_transaction(&proposal.transaction)?;
        self.commit_prepared_transaction(prepared).map_err(|error| {
            component_transaction_error(ComponentError::new(
                "GRAPHIC_RECOVERY_FAILED",
                error.to_string(),
            ))
        })
    }
}

fn containment_component_error(
    error: crate::layout_containment::ContainmentError,
) -> ComponentError {
    let mut diagnostic = ComponentError::new(error.code(), error.message);
    diagnostic.object_id = Some(error.child_id);
    diagnostic
}

pub(crate) fn component_transaction_error(error: ComponentError) -> TransactionDiagnostics {
    TransactionDiagnostics(vec![crate::TransactionDiagnostic {
        severity: crate::DiagnosticSeverity::Error,
        code: error.code.clone(),
        message: error.to_string(),
        operation_index: None,
        operation_type: None,
        object_id: error.object_id,
        object_name: None,
        property: error.property,
        related_objects: vec![],
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_object, ChildPlacement, MacroReferenceTarget};

    #[test]
    fn materialized_snapshots_follow_edits_and_history_without_changing_old_snapshots() {
        let mut pool = ObjectPool::default();
        pool.add(object_with_id(ObjectType::NumberVariable, 1));
        let mut project = EditorProject::from(pool);
        let before = project.materialize_shared().unwrap();
        assert!(Arc::ptr_eq(&before, &project.materialize_shared().unwrap()));

        let mut transaction = OperationTransaction::new(None);
        transaction.add_operation(Operation::SetProperty {
            object: ObjectTarget::id(1),
            property: "value".into(),
            value: 42.into(),
        });
        project.execute_transaction(transaction).unwrap();
        let value = |snapshot: &MaterializedProject| match snapshot
            .pool
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::NumberVariable(number) => number.value,
            _ => unreachable!(),
        };
        assert_eq!(value(&project.materialize_shared().unwrap()), 42);
        assert_eq!(value(&before), 0);
        assert!(project.undo_operation());
        assert_eq!(value(&project.materialize_shared().unwrap()), 0);
        assert!(project.redo_operation());
        assert_eq!(value(&project.materialize_shared().unwrap()), 42);

        let mut owned = project.materialize().unwrap();
        owned.pool.remove(ObjectId::new(1).unwrap());
        assert_eq!(value(&project.materialize_shared().unwrap()), 42);

        let mut preview = project.canvas_preview_copy();
        preview.pool.remove(ObjectId::new(1).unwrap());
        assert_eq!(preview.materialize_shared().unwrap().pool.size(), 0);
        assert_eq!(value(&project.materialize_shared().unwrap()), 42);
        let mut copied = project.clone();
        copied.pool.remove(ObjectId::new(1).unwrap());
        assert_eq!(copied.materialize_shared().unwrap().pool.size(), 0);
    }

    #[test]
    fn property_choices_revalidate_when_dimensions_change_without_a_revision() {
        use ag_iso_stack::object_pool::{object_attributes::Point, ObjectRef};
        let mut mask = object_with_id(ObjectType::DataMask, 1);
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(ObjectRef {
                id: ObjectId::new(2).unwrap(),
                offset: Point { x: 20, y: 20 },
            });
        }
        let mut shape = object_with_id(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(shape) = &mut shape {
            shape.width = 40;
            shape.height = 30;
        }
        let mut pool = ObjectPool::default();
        pool.add(mask);
        pool.add(shape);
        let mut project = EditorProject::from(pool);
        project.mask_size = 200;
        let id = ObjectId::new(2).unwrap();
        assert!(project.property_choice_is_valid(id, "width", 150.into()));
        assert!(project.property_choice_is_valid(id, "width", 150.into()));
        let revision = project.revision();
        project.mask_size = 100;
        assert_eq!(project.revision(), revision);
        assert!(!project.property_choice_is_valid(id, "width", 150.into()));
        assert!(project.property_choice_is_valid(id, "width", 50.into()));
    }

    #[test]
    fn cached_materialization_errors_do_not_survive_a_repair() {
        let mut pointer = object_with_id(ObjectType::ObjectPointer, 1);
        if let Object::ObjectPointer(pointer) = &mut pointer {
            pointer.value = NullableObjectId::new(99);
        }
        let mut pool = ObjectPool::default();
        pool.add(pointer);
        let mut project = EditorProject::from(pool);
        assert!(project.materialize_shared().is_err());
        assert!(project.materialize_shared().is_err());
        assert!(project.delete_object(ObjectId::new(1).unwrap()));
        assert_eq!(project.materialize_shared().unwrap().pool.size(), 0);
    }

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
    fn duplicate_shared_attribute_copies_properties_and_macro_references_as_one_history_item() {
        let attribute_type = ObjectType::FontAttributes;
        let source_id = ObjectId::new(20_001).unwrap();
        let macro_id = ObjectId::new(20).unwrap();
        let mut attribute = object_with_id(attribute_type, source_id.value());
        let macro_object = object_with_id(ObjectType::Macro, macro_id.value());
        let mut pool = ObjectPool::default();
        pool.add(attribute.clone());
        pool.add(macro_object);
        let mut project = EditorProject::from(pool);
        let mut setup = OperationTransaction::new(Some("Add attribute macro".into()));
        setup.add_operation(Operation::SetMacroReferences {
            object: ObjectTarget::id(source_id.value()),
            macro_refs: vec![crate::operations::MacroReferenceTarget {
                event_id: ag_iso_stack::object_pool::object_attributes::Event::OnChangeAttribute,
                macro_object: ObjectTarget::id(macro_id.value()),
            }],
        });
        project.execute_transaction(setup).unwrap();
        attribute = project
            .get_pool()
            .object_by_id(source_id)
            .unwrap()
            .clone();

        let descriptors = crate::object_properties::get_property_descriptors(attribute_type);
        let duplicate_id = project.duplicate_shared_attribute(source_id).unwrap();
        assert_eq!(
            duplicate_id,
            ObjectId::new(project.object_id_ranges().range_for(attribute_type).start).unwrap()
        );
        let duplicate = project.get_pool().object_by_id(duplicate_id).unwrap();
        for descriptor in descriptors.iter().filter(|descriptor| descriptor.writable) {
            assert_eq!(
                crate::object_properties::get_object_property(&attribute, descriptor.name).unwrap(),
                crate::object_properties::get_object_property(duplicate, descriptor.name).unwrap(),
                "writable property {} should be copied",
                descriptor.name
            );
        }
        assert_eq!(
            crate::operations::macro_refs(&attribute),
            crate::operations::macro_refs(duplicate)
        );
        assert!(!project.object_id_is_overridden(duplicate_id));
        assert!(project.undo_operation());
        assert!(project.get_pool().object_by_id(duplicate_id).is_none());
        assert!(project.redo_operation());
        assert!(project.get_pool().object_by_id(duplicate_id).is_some());
    }

    #[test]
    fn duplicate_shared_attribute_fails_when_automatic_range_is_exhausted() {
        let attribute_type = ObjectType::LineAttributes;
        let range_start = crate::ObjectIdRanges::defaults()
            .range_for(attribute_type)
            .start;
        let configured_range = crate::ObjectIdRange {
            start: range_start,
            end: range_start,
        };
        let source_id = ObjectId::new(configured_range.start).unwrap();
        let mut pool = ObjectPool::default();
        pool.add(object_with_id(attribute_type, source_id.value()));
        let mut project = EditorProject::from(pool);
        let mut ranges = project.object_id_ranges().clone();
        ranges.set_range(attribute_type, configured_range);
        project.set_object_id_ranges(ranges).unwrap();
        let before_size = project.get_pool().size();

        assert!(project.duplicate_shared_attribute(source_id).is_err());
        assert_eq!(project.get_pool().size(), before_size);
    }

    #[test]
    fn duplicate_fill_attribute_copies_picture_reference_without_reassigning_consumers() {
        let fill_id = ObjectId::new(30_001).unwrap();
        let picture_id = ObjectId::new(30_002).unwrap();
        let consumer_id = ObjectId::new(30_003).unwrap();
        let mut pool = ObjectPool::default();
        pool.add(object_with_id(ObjectType::FillAttributes, fill_id.value()));
        pool.add(object_with_id(ObjectType::PictureGraphic, picture_id.value()));
        pool.add(object_with_id(ObjectType::GraphicsContext, consumer_id.value()));
        let mut project = EditorProject::from(pool);
        let mut references = OperationTransaction::new(Some("Connect fill attribute".into()));
        references.add_operation(Operation::SetObjectReference {
            object: ObjectTarget::id(fill_id.value()),
            property: "fill_pattern".into(),
            reference: Some(ObjectTarget::id(picture_id.value())),
        });
        references.add_operation(Operation::SetObjectReference {
            object: ObjectTarget::id(consumer_id.value()),
            property: "fill_attributes_object".into(),
            reference: Some(ObjectTarget::id(fill_id.value())),
        });
        project.execute_transaction(references).unwrap();

        let duplicate_id = project.duplicate_shared_attribute(fill_id).unwrap();
        let duplicate = project.get_pool().object_by_id(duplicate_id).unwrap();
        assert_eq!(
            crate::object_properties::get_object_property(duplicate, "fill_pattern").unwrap(),
            serde_json::json!(picture_id)
        );
        assert_eq!(
            crate::object_properties::get_object_property(
                project.get_pool().object_by_id(consumer_id).unwrap(),
                "fill_attributes_object"
            )
            .unwrap(),
            serde_json::json!(fill_id)
        );

        let restored = EditorProject::load_project(project.save_project().unwrap()).unwrap();
        assert_eq!(
            crate::object_properties::get_object_property(
                restored.get_pool().object_by_id(duplicate_id).unwrap(),
                "fill_pattern"
            )
            .unwrap(),
            serde_json::json!(picture_id)
        );
        assert_eq!(
            crate::object_properties::get_object_property(
                restored.get_pool().object_by_id(consumer_id).unwrap(),
                "fill_attributes_object"
            )
            .unwrap(),
            serde_json::json!(fill_id)
        );
    }

    #[test]
    fn duplicate_shared_attribute_rejects_non_attribute_without_mutation() {
        let object_id = ObjectId::new(40_001).unwrap();
        let mut pool = ObjectPool::default();
        pool.add(object_with_id(ObjectType::Button, object_id.value()));
        let mut project = EditorProject::from(pool);
        let before = project.get_pool().as_iop();

        assert!(project.duplicate_shared_attribute(object_id).is_err());
        assert_eq!(project.get_pool().as_iop(), before);
        assert!(!project.undo_available());
    }

    #[test]
    fn duplicate_default_shared_attributes_twice_has_unique_names_and_automatic_ids() {
        let types = [
            ObjectType::FontAttributes,
            ObjectType::LineAttributes,
            ObjectType::FillAttributes,
        ];
        let mut pool = ObjectPool::default();
        for (index, object_type) in types.into_iter().enumerate() {
            pool.add(object_with_id(object_type, 50_001 + index as u16));
        }
        let mut project = EditorProject::from(pool);

        for (index, object_type) in types.into_iter().enumerate() {
            let source_id = ObjectId::new(50_001 + index as u16).unwrap();
            let first = project.duplicate_shared_attribute(source_id).unwrap();
            let second = project.duplicate_shared_attribute(source_id).unwrap();
            assert_ne!(first, second);
            assert!(!project.object_id_is_overridden(first));
            assert!(!project.object_id_is_overridden(second));
            let first_name = project
                .get_object_info(project.get_pool().object_by_id(first).unwrap())
                .name
                .unwrap();
            let second_name = project
                .get_object_info(project.get_pool().object_by_id(second).unwrap())
                .name
                .unwrap();
            assert_ne!(first_name, second_name, "{object_type:?} names must be unique");
        }
    }

    #[test]
    fn exported_shared_font_keeps_two_field_references_and_edit_is_undoable() {
        let font_id = ObjectId::new(40_101).unwrap();
        let string_id = ObjectId::new(40_102).unwrap();
        let number_id = ObjectId::new(40_103).unwrap();
        let mut pool = ObjectPool::default();
        pool.add(object_with_id(ObjectType::FontAttributes, font_id.value()));
        pool.add(object_with_id(ObjectType::OutputString, string_id.value()));
        pool.add(object_with_id(ObjectType::OutputNumber, number_id.value()));
        let mut project = EditorProject::from(pool);
        let mut references = OperationTransaction::new(Some("Share font".into()));
        for consumer_id in [string_id, number_id] {
            references.add_operation(Operation::SetObjectReference {
                object: ObjectTarget::id(consumer_id.value()),
                property: "font_attributes".into(),
                reference: Some(ObjectTarget::id(font_id.value())),
            });
        }
        project.execute_transaction(references).unwrap();

        let export = project.export_iop().unwrap();
        let imported = EditorProject::import_iop(export, VtVersion::Version6).unwrap();
        for consumer_id in [string_id, number_id] {
            assert_eq!(
                crate::object_properties::get_object_property(
                    imported.get_pool().object_by_id(consumer_id).unwrap(),
                    "font_attributes"
                )
                .unwrap(),
                serde_json::json!(font_id)
            );
        }

        let before = crate::object_properties::get_object_property(
            project.get_pool().object_by_id(font_id).unwrap(),
            "font_colour",
        )
        .unwrap();
        let changed = serde_json::json!(if before == serde_json::json!(0) { 1 } else { 0 });
        let mut edit = OperationTransaction::new(Some("Edit shared font".into()));
        edit.add_operation(Operation::SetProperty {
            object: ObjectTarget::id(font_id.value()),
            property: "font_colour".into(),
            value: changed.clone(),
        });
        project.execute_transaction(edit).unwrap();
        for consumer_id in [string_id, number_id] {
            assert_eq!(
                crate::object_properties::get_object_property(
                    project.get_pool().object_by_id(consumer_id).unwrap(),
                    "font_attributes"
                )
                .unwrap(),
                serde_json::json!(font_id)
            );
        }
        assert_eq!(
            crate::object_properties::get_object_property(
                project.get_pool().object_by_id(font_id).unwrap(),
                "font_colour"
            )
            .unwrap(),
            changed
        );
        assert!(project.undo_operation());
        assert_eq!(
            crate::object_properties::get_object_property(
                project.get_pool().object_by_id(font_id).unwrap(),
                "font_colour"
            )
            .unwrap(),
            before
        );
        assert!(project.redo_operation());
        assert_eq!(
            crate::object_properties::get_object_property(
                project.get_pool().object_by_id(font_id).unwrap(),
                "font_colour"
            )
            .unwrap(),
            changed
        );
    }

    #[test]
    fn operation_transaction_rename_object_round_trip() {
        use crate::operations::{Operation, OperationTransaction};

        let mut project = EditorProject::from(ObjectPool::default());

        // Add a working set object (ID=1, which is the default for the first object)
        let mut ws = default_object(ObjectType::WorkingSet);
        let ws_id_value = 1u16;
        ws.mut_id().set_value(ws_id_value).unwrap();
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
            object: ObjectTarget::id(ws_id_value),
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
        tx.add_operation(Operation::DeleteObject { object_id: 11 });

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
        tx.add_operation(Operation::DeleteObject { object_id: 999 });

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
            object: ObjectTarget::id(u16::from(button.id())),
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
        project
            .set_mask_size(original_mask_size.saturating_add(1))
            .unwrap();
        assert!(project.is_dirty());
        project.set_mask_size(original_mask_size).unwrap();
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
    fn dimensions_round_trip_and_undo_in_order_with_object_edits() {
        let mut project = EditorProject::new_pool();
        project.mark_saved();
        let original = project.save_project().unwrap();
        let revision = project.revision();
        project.set_object_id_format(ObjectIdFormat::Hexadecimal);
        assert_eq!(project.save_project().unwrap(), original);
        assert_eq!(project.revision(), revision);
        assert!(!project.is_dirty());

        let layout = SoftKeyLayout {
            arrangement: crate::SoftKeyArrangement::Bottom,
            physical_key_count: 8,
            keys_per_line: 4,
            navigation_key_count: 1,
        };
        project
            .set_project_configuration(640, (80, 96), VtVersion::Version6, layout)
            .unwrap();
        assert!(project.is_dirty());
        let revision = project.revision();
        project
            .set_project_configuration(640, (80, 96), VtVersion::Version6, layout)
            .unwrap();
        assert!(project.set_design_dimensions(320, (0, 60)).is_err());
        assert_eq!(project.revision(), revision);
        assert_eq!(
            project.dimensions(),
            DesignDimensions {
                mask_size: 640,
                soft_key_size: (80, 96),
                vt_version: VtVersion::Version6,
                soft_key_layout: layout,
                object_id_ranges: crate::ObjectIdRanges::defaults(),
            }
        );
        let saved = project.save_project().unwrap();
        let restored = EditorProject::load_project(saved).unwrap();
        assert_eq!(restored.mask_size, 640);
        assert_eq!(restored.soft_key_size(), (80, 96));
        assert_eq!(restored.soft_key_layout(), layout);
        assert_eq!(project.canvas_preview_copy().soft_key_size(), (80, 96));
        assert_eq!(
            EditorProject::from_ai_source(&project.ai_design_state()).soft_key_size(),
            (80, 96)
        );

        let id = project
            .create_object(ObjectType::Button, "Button".into())
            .unwrap();
        project.set_design_dimensions(800, (100, 120)).unwrap();
        assert!(project.undo_operation());
        assert_eq!(project.mask_size, 640);
        assert_eq!(project.soft_key_size(), (80, 96));
        assert_eq!(project.soft_key_layout(), layout);
        assert!(project.get_pool().object_by_id(id).is_some());
        assert!(project.undo_operation());
        assert!(project.get_pool().object_by_id(id).is_none());
        assert!(project.undo_operation());
        assert_eq!(project.mask_size, 500);
        assert_eq!(project.soft_key_size(), (60, 60));
        assert_eq!(project.soft_key_layout(), SoftKeyLayout::default());
        assert!(!project.is_dirty());
        assert!(!project.undo_available());
        assert!(project.redo_operation());
        assert!(project.redo_operation());
        assert!(project.redo_operation());
        assert_eq!(project.mask_size, 800);
        assert_eq!(project.soft_key_size(), (100, 120));
        assert_eq!(project.soft_key_layout(), layout);
        assert!(project.get_pool().object_by_id(id).is_some());
    }

    #[test]
    fn legacy_softkey_dimensions_take_independent_key_content_extents() {
        let mut pool = ObjectPool::default();
        for (key_id, child_id, width, height) in [(10, 11, 90, 40), (12, 13, 30, 110)] {
            let mut key = object_with_id(ObjectType::Key, key_id);
            let mut child = object_with_id(ObjectType::Button, child_id);
            if let Object::Key(key) = &mut key {
                key.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
                    id: ObjectId::new(child_id).unwrap(),
                    offset: ag_iso_stack::object_pool::object_attributes::Point::default(),
                });
            }
            if let Object::Button(child) = &mut child {
                child.width = width;
                child.height = height;
            }
            pool.add(key);
            pool.add(child);
        }
        let project = EditorProject::from(pool);
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&project.save_project().unwrap()).unwrap();
        legacy["settings"]
            .as_object_mut()
            .unwrap()
            .remove("soft_key_size");
        legacy["settings"]
            .as_object_mut()
            .unwrap()
            .remove("object_id_ranges");
        for metadata in legacy["object_metadata"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            metadata.as_object_mut().unwrap().remove("id_overridden");
        }
        legacy["settings"]["mask_size"] = serde_json::json!(240);
        let restored = EditorProject::load_project(serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(restored.mask_size, 240);
        assert_eq!(restored.soft_key_size(), (90, 110));
        assert_eq!(restored.soft_key_layout(), SoftKeyLayout::default());
        assert_eq!(
            restored.object_id_ranges(),
            &crate::ObjectIdRanges::defaults()
        );
        assert!(restored
            .get_pool()
            .object_by_id(ObjectId::new(10).unwrap())
            .is_some());
        assert!(!restored.object_id_is_overridden(ObjectId::new(10).unwrap()));
        assert!(!restored.is_dirty());
    }

    #[test]
    fn recovered_projects_remain_dirty_until_marked_saved() {
        let project = EditorProject::from(ObjectPool::default());
        project.mark_recovered();
        assert!(project.is_dirty());
        project.mark_saved();
        assert!(!project.is_dirty());
    }

    #[test]
    fn type_ranges_allocate_independently_persist_and_undo_without_renumbering() {
        let button = object_with_id(ObjectType::Button, 6000);
        let mut pool = ObjectPool::default();
        pool.add(button.clone());
        let mut project = EditorProject::from(pool);
        project.mark_saved();

        assert_eq!(
            project
                .allocate_object_id(ObjectType::Macro)
                .unwrap()
                .value(),
            0
        );
        assert_eq!(
            project
                .allocate_object_id(ObjectType::Button)
                .unwrap()
                .value(),
            6001
        );
        assert!(project.set_object_id_overridden(button.id(), true));
        let mut ranges = project.object_id_ranges().clone();
        ranges.set_range(
            ObjectType::Button,
            crate::ObjectIdRange {
                start: 6100,
                end: 6199,
            },
        );
        project.set_object_id_ranges(ranges.clone()).unwrap();
        assert_eq!(
            project.get_pool().object_by_id(button.id()).unwrap().id(),
            button.id()
        );

        let saved = project.save_project().unwrap();
        let loaded = EditorProject::load_project(saved).unwrap();
        assert_eq!(loaded.object_id_ranges(), &ranges);
        assert!(loaded.object_id_is_overridden(button.id()));
        assert_eq!(
            loaded.get_pool().object_by_id(button.id()).unwrap().id(),
            button.id()
        );

        assert!(project.undo_operation());
        assert_eq!(
            project.object_id_ranges(),
            &crate::ObjectIdRanges::defaults()
        );
        assert_eq!(
            project.get_pool().object_by_id(button.id()).unwrap().id(),
            button.id()
        );
        assert!(project.redo_operation());
        assert_eq!(project.object_id_ranges(), &ranges);
        assert!(project.set_object_id_overridden(button.id(), false));
        assert!(!project.object_id_is_overridden(button.id()));
        assert!(project.undo_operation());
        assert!(project.object_id_is_overridden(button.id()));
        assert!(project.redo_operation());
        assert!(!project.object_id_is_overridden(button.id()));
    }

    #[test]
    fn allocator_reports_type_range_exhaustion() {
        let button = object_with_id(ObjectType::Button, 6000);
        let mut pool = ObjectPool::default();
        pool.add(button);
        let mut project = EditorProject::from(pool);
        let mut ranges = project.object_id_ranges().clone();
        ranges.set_range(
            ObjectType::Button,
            crate::ObjectIdRange {
                start: 6000,
                end: 6000,
            },
        );
        project.set_object_id_ranges(ranges).unwrap();
        let error = project.allocate_object_id(ObjectType::Button).unwrap_err();
        assert_eq!(error.object_type, ObjectType::Button);
        assert_eq!(error.range.start, 6000);
        assert!(error.to_string().contains("No free Button object ID"));
    }

    #[test]
    fn automatic_create_stays_unoverridden_through_redo() {
        let mut project = EditorProject::new_pool();
        let id = project
            .create_object(ObjectType::Button, "Button test".into())
            .unwrap();
        assert!(!project.object_id_is_overridden(id));
        assert!(project.undo_operation());
        assert!(project.redo_operation());
        assert!(!project.object_id_is_overridden(id));
    }

    #[test]
    fn allocator_reserves_ids_for_queued_automatic_creates() {
        let mut project = EditorProject::new_pool();
        project.queue_operation(Operation::CreateObject {
            object_id: None,
            object_type: ObjectType::Button,
            name: "Queued button".into(),
        });
        assert_eq!(
            project
                .allocate_object_id(ObjectType::Button)
                .unwrap()
                .value(),
            6001
        );
        assert!(project.commit_queued("Create queued button"));
        let next = project.allocate_object_id(ObjectType::Button).unwrap();
        assert_eq!(next.value(), 6001);
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
            object: ObjectTarget::id(object.id().value()),
            property: "width".to_owned(),
            value: serde_json::json!(123),
        });
        project.queue_operation(Operation::SetProperty {
            object: ObjectTarget::id(object.id().value()),
            property: "height".to_owned(),
            value: serde_json::json!(45),
        });
        project.queue_operation(Operation::RenameObject {
            object: ObjectTarget::id(object.id().value()),
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
            object: ObjectTarget::id(object.id().value()),
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
        use ag_iso_stack::object_pool::object_attributes::Event;

        let mut parent = object_with_id(ObjectType::Button, 10);
        if let Object::Button(button) = &mut parent {
            button.width = 100;
            button.height = 100;
        }
        let child = object_with_id(ObjectType::PictureGraphic, 11);
        let macro_object = object_with_id(ObjectType::Macro, 12);
        let mut pool = ObjectPool::default();
        pool.add(parent.clone());
        pool.add(child.clone());
        pool.add(macro_object);
        let mut project = EditorProject::from(pool);
        let before = project.get_pool().as_iop();

        project.queue_operation(Operation::SetChildren {
            parent: ObjectTarget::id(parent.id().value()),
            children: vec![ChildPlacement {
                child: ObjectTarget::id(child.id().value()),
                x: 4,
                y: 8,
            }],
        });
        project.queue_operation(Operation::SetMacroReferences {
            object: ObjectTarget::id(parent.id().value()),
            macro_refs: vec![MacroReferenceTarget {
                macro_object: ObjectTarget::id(12),
                event_id: Event::OnKeyPress,
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
                event_id: Event::OnKeyPress,
            });
        }
        let macro_object = object_with_id(ObjectType::Macro, 12);
        let mut pool = ObjectPool::default();
        pool.add(parent);
        pool.add(macro_object);
        let mut project = EditorProject::from(pool);
        let mut transaction = OperationTransaction::new(Some("Delete macro".to_owned()));
        transaction.add_operation(Operation::DeleteObject { object_id: 12 });

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
                event_id: Event::OnKeyPress,
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
    fn simultaneous_id_swap_updates_selection_once_and_round_trips_history() {
        let first = object_with_id(ObjectType::NumberVariable, 10);
        let second = object_with_id(ObjectType::NumberVariable, 20);
        let mut pool = ObjectPool::default();
        pool.add(first);
        pool.add(second);
        let mut project = EditorProject::from(pool);
        project
            .get_mut_selected()
            .replace(NullableObjectId::new(10));
        project.update_selected();

        project
            .execute_transaction(OperationTransaction {
                schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
                description: Some("Swap IDs".into()),
                operations: vec![Operation::RemapObjectIds {
                    mappings: vec![(10, 20), (20, 10)],
                    target_types: vec![
                        (10, ObjectType::NumberVariable),
                        (20, ObjectType::NumberVariable),
                    ],
                }],
            })
            .unwrap();
        assert!(project
            .get_pool()
            .object_by_id(ObjectId::new(10).unwrap())
            .is_some());
        assert!(project
            .get_pool()
            .object_by_id(ObjectId::new(20).unwrap())
            .is_some());
        assert_eq!(project.get_selected(), NullableObjectId::new(20));

        assert!(project.undo_operation());
        assert_eq!(project.get_selected(), NullableObjectId::new(10));
        assert!(project.redo_operation());
        assert_eq!(project.get_selected(), NullableObjectId::new(20));
    }

    #[test]
    fn reassignment_preview_rejects_malformed_macro_without_mutation() {
        let button = object_with_id(ObjectType::Button, 10);
        let mut macro_object = object_with_id(ObjectType::Macro, 11);
        if let Object::Macro(macro_object) = &mut macro_object {
            macro_object.commands = vec![0xFF];
        }
        let mut pool = ObjectPool::default();
        pool.add(button);
        pool.add(macro_object);
        let mut project = EditorProject::from(pool);
        let before = project.get_pool().as_iop();

        assert!(project.plan_id_reassignment(false).is_err());
        assert_eq!(project.get_pool().as_iop(), before);
        assert!(!project.undo_available());
    }

    #[test]
    fn object_id_override_flag_is_persisted_and_undoable() {
        let object = object_with_id(ObjectType::Button, 10);
        let mut pool = ObjectPool::default();
        pool.add(object.clone());
        let mut project = EditorProject::from(pool);
        assert!(project.set_object_id_overridden(object.id(), true));
        assert!(project.object_id_is_overridden(object.id()));
        assert!(project.undo_operation());
        assert!(!project.object_id_is_overridden(object.id()));
        assert!(project.redo_operation());
        assert!(project.object_id_is_overridden(object.id()));

        let saved = project.save_project().unwrap();
        let restored = EditorProject::load_project(saved).unwrap();
        assert!(restored.object_id_is_overridden(object.id()));
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
        project.set_mask_size(500).unwrap();
        let before = project.get_pool().as_iop();

        let mut add = OperationTransaction::new(Some("Add child".to_owned()));
        add.add_operation(Operation::AddChild {
            parent: ObjectTarget::id(10),
            child: ObjectTarget::id(11),
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
            object: ObjectTarget::id(object.id().value()),
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
            object: ObjectTarget::id(button.id().value()),
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
