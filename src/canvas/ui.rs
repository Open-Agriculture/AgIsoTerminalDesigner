use super::layout::*;
use super::MaskViewState;
use super::{
    edits::{self, position, CanvasEdit, PlacementTarget},
    geometry::{self, Scene, SceneItem},
};
use crate::object_rendering::Colorable;
use crate::{components::MaterializedProject, EditorProject};
use ag_iso_stack::object_pool::{
    object::Object, object_attributes::Point, NullableObjectId, ObjectId,
};
use egui::{Key, Modifiers, Pos2, Rect, Stroke, Vec2};
use std::collections::HashMap;

/// Transient viewport, occurrence selection, and gesture drafts for one project.
/// Reset this state when replacing the project; no canvas state is serialized.
pub struct CanvasState {
    mask: Option<u16>,
    softkey_mask: Option<u16>,
    softkey_page: usize,
    revision: u64,
    selected: Vec<Vec<usize>>,
    scene_identity: Vec<(Vec<usize>, u16, u16, Option<PlacementTarget>)>,
    scene_origins: HashMap<Vec<usize>, Pos2>,
    placement_owner: Option<u16>,
    context: Vec<usize>,
    inspector: Option<u16>,
    hierarchy_selection: Option<(u64, u16, Vec<usize>, u16)>,
    hierarchy_frame_revision: Option<u64>,
    hierarchy_visible: Vec<HierarchyOccurrence>,
    hierarchy_selected: Vec<HierarchyOccurrence>,
    hierarchy_anchor: Option<(u16, Vec<usize>)>,
    hierarchy_selection_explicit: bool,
    all_hierarchy_references: Option<u16>,
    zoom: f32,
    pan: Vec2,
    viewport: Rect,
    field_draft: Option<FieldDraft>,
    gesture: Option<Gesture>,
    error: Option<String>,
    snap_objects: bool,
    snap_spacing: bool,
    grid_visible: bool,
    secondary_panned: bool,
    /// What was under the last right click, where Add object here places the object
    add_target: Option<AddTarget>,
    add_object_query: String,
    fit: Option<bool>,
    fit_all: bool,
    initially_fitted: bool,
    pages: HashMap<u16, (Option<u16>, usize)>,
    preferred_slot: Option<(u16, usize, Option<u16>)>,
    pair_layout: Vec<(u16, Vec2, Rect)>,
    preview: Option<super::preview::PreviewMode>,
}

/// Asks the app to create an object of `object_type` as a child of `parent` at
/// `x`, `y`, after the user names it. Stored in the egui context's temporary data
/// under [`AddObjectRequest::id`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddObjectRequest {
    pub object_type: ag_iso_stack::object_pool::ObjectType,
    /// The mask shown on the canvas.
    pub root: u16,
    pub parent: u16,
    pub x: i16,
    pub y: i16,
}

/// Asks the app to group placements under a new parent of `object_type`,
/// after the user names it. Stored in the egui context's temporary data under
/// [`GroupObjectsRequest::id`].
#[derive(Clone, Debug)]
pub struct GroupObjectsRequest {
    pub session: uuid::Uuid,
    pub revision: u64,
    /// The mask shown on the canvas.
    pub root: u16,
    pub targets: Vec<PlacementTarget>,
    pub object_type: ag_iso_stack::object_pool::ObjectType,
}

impl GroupObjectsRequest {
    pub fn id() -> egui::Id {
        egui::Id::new("canvas_group_objects_request")
    }
}

/// The object under a right click that accepts new children, and the click's
/// position in it. Soft Key Masks list their keys without positions.
#[derive(Clone, Debug)]
struct AddTarget {
    parent: u16,
    x: i16,
    y: i16,
}

impl AddObjectRequest {
    pub fn id() -> egui::Id {
        egui::Id::new("canvas_add_object_request")
    }
}

/// Identity and edit information for one visible hierarchy occurrence.
/// Paths distinguish repeated references; component headers use synthetic paths
/// and resolve their canvas placement through `target` and `parent_path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HierarchyOccurrence {
    /// Mask containing the hierarchy occurrence, including its Soft Key slots.
    pub root: u16,
    /// Exact row path within that mask's hierarchy.
    pub path: Vec<usize>,
    /// Object shown in the inspector, or the materialized component root.
    pub object: u16,
    /// Editable placement; roots and pointer-target aliases have none.
    pub target: Option<PlacementTarget>,
    /// Path of the containing occurrence, used to identify exact siblings.
    pub parent_path: Option<Vec<usize>>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            mask: None,
            softkey_mask: None,
            softkey_page: 0,
            revision: 0,
            selected: vec![],
            scene_identity: vec![],
            scene_origins: HashMap::new(),
            placement_owner: None,
            context: vec![],
            inspector: None,
            hierarchy_selection: None,
            hierarchy_frame_revision: None,
            hierarchy_visible: Vec::new(),
            hierarchy_selected: Vec::new(),
            hierarchy_anchor: None,
            hierarchy_selection_explicit: false,
            all_hierarchy_references: None,
            zoom: 1.0,
            pan: egui::vec2(24.0, 24.0),
            viewport: Rect::NOTHING,
            field_draft: None,
            gesture: None,
            error: None,
            snap_objects: true,
            snap_spacing: true,
            grid_visible: false,
            secondary_panned: false,
            add_target: None,
            add_object_query: String::new(),
            fit: None,
            fit_all: false,
            initially_fitted: false,
            pages: HashMap::new(),
            preferred_slot: None,
            pair_layout: Vec::new(),
            preview: None,
        }
    }
}

#[derive(Clone)]
enum GestureKind {
    Move,
    Resize(Vec2),
    PolygonVertex(usize),
    LineEndpoint(usize),
    Marquee,
    Pan,
}
struct FieldDraft {
    identity: (Vec<usize>, u64),
    position: Option<(i16, i16)>,
    width: Option<u16>,
    height: Option<u16>,
    changed: bool,
    finish_pending: bool,
}
struct Gesture {
    kind: GestureKind,
    start: Pos2,
    current: Pos2,
    items: Vec<SceneItem>,
    edits: Vec<CanvasEdit>,
    draft: Option<(MaterializedProject, Vec<PlacementTarget>)>,
    guides: Vec<(Pos2, Pos2)>,
    changed: bool,
    fingerprint: Vec<CanvasEdit>,
    base_selection: Vec<Vec<usize>>,
}

impl CanvasState {
    /// Whether the non-snapping, mask-coordinate grid overlay is visible.
    pub fn grid_visible(&self) -> bool {
        self.grid_visible
    }

    /// Toggle the grid without changing placement or snapping behavior.
    pub fn set_grid_visible(&mut self, visible: bool) {
        self.grid_visible = visible;
    }

    /// Return the single selected editable occurrence, revalidated against the project.
    pub fn selected_placement(&self, project: &EditorProject) -> Option<PlacementTarget> {
        if self.selected.len() != 1 {
            return None;
        }
        let materialized = project.materialize().ok()?;
        let scene = geometry::build_page(project, &materialized, self.mask?, self.softkey_page);
        let selected = self.reconciled_selection(&scene);
        let path = selected.first()?;
        let item = scene.items.iter().find(|i| &i.path == path)?;
        if project.get_selected().0.map(|id| id.value()) != Some(item.selection_id)
            && !self.has_placement_selection(project)
        {
            return None;
        }
        item.target.clone()
    }

    /// Whether the inspector still refers to a selected placement, even if a
    /// structural edit made its occurrence stale. Such a selection must be
    /// explicitly selected again rather than treated as an unplaced definition.
    pub fn has_placement_selection(&self, project: &EditorProject) -> bool {
        self.placement_owner.is_some()
            && self.placement_owner == project.get_selected().0.map(|id| id.value())
    }

    /// Start a hierarchy rendering pass. Rows registered afterward define the
    /// visible order used by Shift selection.
    pub fn begin_hierarchy_frame(&mut self, revision: u64) {
        if self
            .hierarchy_frame_revision
            .is_some_and(|old| old != revision)
        {
            self.hierarchy_selected.clear();
            self.hierarchy_anchor = None;
            self.hierarchy_selection = None;
        }
        self.hierarchy_frame_revision = Some(revision);
        self.hierarchy_visible.clear();
    }

    /// Register a row in its hierarchy display order. Duplicate registration
    /// of the same exact row updates its data without changing its range order.
    pub fn register_hierarchy_occurrence(&mut self, occurrence: HierarchyOccurrence) {
        if let Some(existing) = self
            .hierarchy_visible
            .iter_mut()
            .find(|row| row.root == occurrence.root && row.path == occurrence.path)
        {
            *existing = occurrence;
        } else {
            self.hierarchy_visible.push(occurrence);
        }
    }

    /// Return the exact selected hierarchy rows, independent of inspector ID.
    pub fn selected_hierarchy_occurrences(&self) -> &[HierarchyOccurrence] {
        &self.hierarchy_selected
    }

    /// Revision against which the current registered hierarchy rows were resolved.
    pub fn hierarchy_selection_revision(&self) -> Option<u64> {
        self.hierarchy_frame_revision
    }

    /// Return selected rows only when they are siblings under the same exact
    /// parent occurrence. A one-row selection is valid for parent creation.
    pub fn selected_hierarchy_siblings(&self) -> Option<Vec<HierarchyOccurrence>> {
        let first = self.hierarchy_selected.first()?;
        let parent = first.parent_path.as_ref()?;
        self.hierarchy_selected
            .iter()
            .all(|row| row.root == first.root && row.parent_path.as_ref() == Some(parent))
            .then(|| self.hierarchy_selected.clone())
    }

    /// Whether an exact hierarchy row belongs to the explicit occurrence selection.
    pub fn hierarchy_occurrence_selected(&self, root: u16, path: &[usize]) -> bool {
        self.hierarchy_selected
            .iter()
            .any(|row| row.root == root && row.path == path)
    }

    /// Apply normal, toggle, range, or context-menu selection to a registered row.
    /// The clicked object remains the inspector selection while all editable
    /// selected rows are reflected in the canvas scene selection.
    pub fn hierarchy_click(
        &mut self,
        project: &EditorProject,
        root: u16,
        path: &[usize],
        modifiers: Modifiers,
        right_click: bool,
    ) -> bool {
        let Some(clicked_index) = self
            .hierarchy_visible
            .iter()
            .position(|row| row.root == root && row.path == path)
        else {
            return false;
        };
        let clicked = self.hierarchy_visible[clicked_index].clone();
        self.switch_mask(Some(root));
        let already_selected = self.hierarchy_occurrence_selected(root, path);
        if right_click && already_selected {
            // Keep the block intact for a context menu.
        } else if right_click || (!modifiers.ctrl && !modifiers.command && !modifiers.shift) {
            self.hierarchy_selected = vec![clicked.clone()];
            self.hierarchy_anchor = Some((root, path.to_vec()));
        } else if modifiers.shift {
            let anchor_index = self
                .hierarchy_anchor
                .as_ref()
                .and_then(|(anchor_root, anchor_path)| {
                    (*anchor_root == root)
                        .then(|| {
                            self.hierarchy_visible.iter().position(|row| {
                                row.root == *anchor_root && row.path == *anchor_path
                            })
                        })
                        .flatten()
                })
                .unwrap_or(clicked_index);
            let (start, end) = if anchor_index <= clicked_index {
                (anchor_index, clicked_index)
            } else {
                (clicked_index, anchor_index)
            };
            self.hierarchy_selected = self.hierarchy_visible[start..=end].to_vec();
        } else if modifiers.ctrl || modifiers.command {
            if already_selected {
                self.hierarchy_selected
                    .retain(|row| !(row.root == root && row.path == path));
            } else {
                self.hierarchy_selected.push(clicked.clone());
                self.hierarchy_anchor = Some((root, path.to_vec()));
            }
            self.hierarchy_selected.sort_by_key(|selected| {
                self.hierarchy_visible
                    .iter()
                    .position(|row| row.root == selected.root && row.path == selected.path)
                    .unwrap_or(usize::MAX)
            });
        }
        self.hierarchy_selection_explicit = true;
        self.sync_hierarchy_selection(project, &clicked);
        true
    }

    /// Restore a batch selection after a structural edit using the fresh
    /// placement targets and their exact destination parent occurrence.
    pub fn select_hierarchy_placements(
        &mut self,
        project: &EditorProject,
        root: u16,
        parent_path: &[usize],
        targets: &[PlacementTarget],
    ) -> bool {
        let selected: Vec<_> = targets
            .iter()
            .filter_map(|target| {
                self.hierarchy_visible
                    .iter()
                    .find(|row| {
                        row.root == root
                            && row.parent_path.as_deref() == Some(parent_path)
                            && row.target.as_ref() == Some(target)
                    })
                    .cloned()
            })
            .collect();
        let Some(clicked) = selected.last().cloned() else {
            return false;
        };
        self.switch_mask(Some(root));
        self.hierarchy_selected = selected;
        self.hierarchy_anchor = Some((clicked.root, clicked.path.clone()));
        self.hierarchy_selection_explicit = true;
        self.sync_hierarchy_selection(project, &clicked);
        true
    }

    fn sync_hierarchy_selection(&mut self, project: &EditorProject, clicked: &HierarchyOccurrence) {
        self.switch_mask(Some(clicked.root));
        self.hierarchy_selection = None;
        self.all_hierarchy_references = None;
        if self.hierarchy_selected.is_empty() {
            self.selected.clear();
            self.inspector = None;
            self.placement_owner = None;
            project.get_mut_selected().set(NullableObjectId::default());
        }
        let Ok(materialized) = project.materialize_shared() else {
            return;
        };
        let page = self.pages.get(&clicked.root).map_or(0, |page| page.1);
        let scene = geometry::build_page(project, &materialized, clicked.root, page);
        let keys = super::softkeys::build(project, &materialized.pool, clicked.root, page);
        self.pages.insert(clicked.root, (keys.mask_id, keys.page));
        self.softkey_mask = keys.mask_id;
        self.softkey_page = keys.page;
        self.selected = self
            .hierarchy_selected
            .iter()
            .filter_map(|row| {
                if row.root != clicked.root {
                    return None;
                }
                row_scene_item(project, &materialized, &scene, row)
                    .filter(|item| item.target.is_some())
                    .map(|item| item.path.clone())
            })
            .collect();
        self.revision = project.revision();
        if !self.hierarchy_selected.is_empty() {
            self.inspector = Some(clicked.object);
            self.placement_owner = clicked.target.as_ref().map(|_| clicked.object);
        }
        self.remember_scene(&scene);
        if !self.hierarchy_selected.is_empty() {
            project
                .get_mut_selected()
                .set(NullableObjectId::new(clicked.object));
        }
    }

    /// Preserve the identity of a clicked hierarchy row independently of the
    /// object rendered at its placement (in particular, an ObjectPointer).
    pub fn select_hierarchy_object(
        &mut self,
        project: &EditorProject,
        root: u16,
        path: &[usize],
        object: u16,
    ) {
        self.switch_mask(Some(root));
        self.selected.clear();
        self.placement_owner = None;
        if let Ok(materialized) = project.materialize_shared() {
            let scene = geometry::build_page(project, &materialized, root, self.softkey_page);
            if let Some(scene_path) = hierarchy_scene_path(project, &materialized, root, path) {
                if let Some(item) = scene.items.iter().find(|item| item.path == scene_path) {
                    self.selected.push(scene_path);
                    self.placement_owner = item.target.as_ref().map(|_| object);
                }
            }
            self.remember_scene(&scene);
        }
        self.revision = project.revision();
        self.hierarchy_selection = Some((project.revision(), root, path.to_vec(), object));
        self.all_hierarchy_references = None;
        self.hierarchy_selected = vec![HierarchyOccurrence {
            root,
            path: path.to_vec(),
            object,
            target: self.selected.first().and_then(|scene_path| {
                self.scene_identity
                    .iter()
                    .find(|row| &row.0 == scene_path)
                    .and_then(|row| row.3.clone())
            }),
            parent_path: path.len().checked_sub(1).map(|end| path[..end].to_vec()),
        }];
        self.hierarchy_anchor = Some((root, path.to_vec()));
        self.hierarchy_selection_explicit = true;
        self.inspector = Some(object);
        project
            .get_mut_selected()
            .set(NullableObjectId::new(object));
    }

    /// Highlight every hierarchy occurrence after an object search, without
    /// making the other placements part of the canvas edit selection.
    pub fn select_all_hierarchy_references(&mut self, object: Option<u16>) {
        self.hierarchy_selection = None;
        self.hierarchy_selected.clear();
        self.hierarchy_anchor = None;
        self.hierarchy_selection_explicit = false;
        self.all_hierarchy_references = object;
    }

    /// Whether a matching hierarchy row receives the full selection highlight.
    /// A search or a selection without an exact occurrence highlights all rows.
    pub fn hierarchy_occurrence_is_primary(
        &self,
        project: &EditorProject,
        root: u16,
        path: &[usize],
        object: u16,
    ) -> bool {
        if self.all_hierarchy_references == Some(object) {
            return true;
        }
        if self.hierarchy_selection_explicit {
            return self
                .hierarchy_selected
                .iter()
                .any(|row| row.root == root && row.path == path && row.object == object);
        }
        if !self.hierarchy_selected.is_empty() {
            return self
                .hierarchy_selected
                .iter()
                .any(|row| row.root == root && row.path == path && row.object == object);
        }
        if let Some((revision, selected_root, selected_path, selected_object)) =
            &self.hierarchy_selection
        {
            if *revision == project.revision() && *selected_object == object {
                return *selected_root == root && selected_path == path;
            }
        }
        if self.inspector != Some(object) || self.mask != Some(root) || self.selected.is_empty() {
            return true;
        }
        let Ok(materialized) = project.materialize_shared() else {
            return true;
        };
        hierarchy_scene_path(project, &materialized, root, path)
            .is_some_and(|scene_path| self.selected.contains(&scene_path))
    }

    fn scene_identity(scene: &Scene) -> Vec<(Vec<usize>, u16, u16, Option<PlacementTarget>)> {
        scene
            .items
            .iter()
            .map(|item| {
                (
                    item.path.clone(),
                    item.object_id,
                    item.selection_id,
                    item.target.clone(),
                )
            })
            .collect()
    }

    fn remember_scene(&mut self, scene: &Scene) {
        self.scene_identity = Self::scene_identity(scene);
        self.scene_origins = scene
            .items
            .iter()
            .map(|item| (item.path.clone(), item.rect.min))
            .collect();
    }

    fn reconciled_selection(&self, scene: &Scene) -> Vec<Vec<usize>> {
        if self.scene_identity == Self::scene_identity(scene) {
            return self.selected.clone();
        }
        self.selected
            .iter()
            .filter_map(|path| {
                let (_, _object, selection_id, target) = self
                    .scene_identity
                    .iter()
                    .find(|(old, _, _, _)| old == path)?;
                let origin = self.scene_origins.get(path)?;
                let mut matches = scene.items.iter().filter(|item| {
                    let same_parent = match (target, &item.target) {
                        (
                            Some(PlacementTarget::Primitive { parent_id: old, .. }),
                            Some(PlacementTarget::Primitive { parent_id: new, .. }),
                        ) => old == new,
                        (
                            Some(PlacementTarget::Component { instance_id: old }),
                            Some(PlacementTarget::Component { instance_id: new }),
                        ) => old == new,
                        (None, None) => true,
                        _ => false,
                    };
                    item.selection_id == *selection_id
                        && item.rect.min == *origin
                        && same_parent
                        && super::softkeys::area(path) == super::softkeys::area(&item.path)
                });
                let matched = matches.next()?;
                matches.next().is_none().then(|| matched.path.clone())
            })
            .collect()
    }

    /// Return a tree parent's mask-local child origin, including soft-key slots.
    /// Tree paths begin with zero for the data/alarm mask or `slot + 1` for
    /// a linked soft-key occurrence. Remaining indices are canonical primitive
    /// child-list indices; `usize::MAX` enters an ObjectPointer target.
    pub fn hierarchy_child_origin(
        &self,
        project: &EditorProject,
        root: u16,
        path: &[usize],
    ) -> Result<Pos2, String> {
        let Some((&branch, children)) = path.split_first() else {
            return edits::hierarchy_child_origin(project, root, &[]);
        };
        if branch == 0 {
            return edits::hierarchy_child_origin(project, root, children);
        }
        let slot_index = branch - 1;
        let page = self.pages.get(&root).map_or(0, |entry| entry.1);
        let mut keys = super::softkeys::build(project, project.get_pool(), root, page);
        let capacity = usize::from(project.soft_key_layout().physical_key_count)
            - usize::from(keys.navigation);
        if !keys.slots.iter().any(|slot| {
            slot.index == slot_index
                && matches!(slot.content, super::softkeys::SlotContent::Key { .. })
        }) {
            keys = super::softkeys::build(project, project.get_pool(), root, slot_index / capacity);
        }
        let slot = keys
            .slots
            .iter()
            .find(|slot| {
                slot.index == slot_index
                    && matches!(slot.content, super::softkeys::SlotContent::Key { .. })
            })
            .ok_or("Missing soft-key occurrence")?;
        let mask_id = keys
            .mask_id
            .and_then(|id| ObjectId::new(id).ok())
            .ok_or("Missing soft-key mask")?;
        let Some(ag_iso_stack::object_pool::object::Object::SoftKeyMask(mask)) =
            project.get_pool().object_by_id(mask_id)
        else {
            return Err("Invalid soft-key mask".into());
        };
        let key = mask
            .objects
            .get(slot_index)
            .ok_or("Missing soft-key reference")?;
        let local = edits::hierarchy_child_origin(project, key.value(), children)?;
        Ok(slot.rect.min + local.to_vec2())
    }

    /// Reparent a tree occurrence while preserving its mask-local canvas position.
    /// Paths use the synthetic mask/soft-key branch prefix documented by
    /// `hierarchy_child_origin`, and may cross between mask and key content.
    pub fn reparent_in_hierarchy(
        &self,
        project: &mut EditorProject,
        target: &PlacementTarget,
        source_root: u16,
        source_parent_path: &[usize],
        dest_root: u16,
        dest_parent_path: &[usize],
        new_parent: u16,
    ) -> Result<PlacementTarget, String> {
        self.reparent_in_hierarchy_at(
            project,
            target,
            source_root,
            source_parent_path,
            dest_root,
            dest_parent_path,
            new_parent,
            None,
        )
    }

    /// Reparent an occurrence at a displayed sibling slot while preserving its
    /// mask-local position. Parent paths use `[0, ...]` for the mask branch,
    /// `[slot + 1, ...]` for soft keys, `usize::MAX` for pointer targets, and
    /// canonical primitive child indexes. Soft-key branch paths use the linked
    /// mask's slot indexes. `Some(index)` selects a slot in the combined
    /// primitive/component order before source removal; `None`
    /// appends. Same-parent reorders keep their coordinates, while cross-parent
    /// moves retain the existing position-preservation and clamping rules.
    pub fn reparent_in_hierarchy_at(
        &self,
        project: &mut EditorProject,
        target: &PlacementTarget,
        source_root: u16,
        source_parent_path: &[usize],
        dest_root: u16,
        dest_parent_path: &[usize],
        new_parent: u16,
        insertion_index: Option<usize>,
    ) -> Result<PlacementTarget, String> {
        let source = self.hierarchy_child_origin(project, source_root, source_parent_path)?;
        let dest = self.hierarchy_child_origin(project, dest_root, dest_parent_path)?;
        let (x, y) = position(project, target).ok_or("Missing source occurrence")?;
        let local = source + egui::vec2(f32::from(x), f32::from(y)) - dest;
        edits::reparent_at_index(
            project,
            target,
            new_parent,
            local.x.round() as i32,
            local.y.round() as i32,
            insertion_index,
        )
    }

    /// Validate a hierarchy drop against a lightweight disposable project copy.
    pub fn validate_hierarchy_drop(
        &self,
        project: &EditorProject,
        target: &PlacementTarget,
        source_root: u16,
        source_parent_path: &[usize],
        dest_root: u16,
        dest_parent_path: &[usize],
        new_parent: u16,
        insertion_index: Option<usize>,
    ) -> Result<(), String> {
        let mut candidate = project.canvas_preview_copy();
        self.reparent_in_hierarchy_at(
            &mut candidate,
            target,
            source_root,
            source_parent_path,
            dest_root,
            dest_parent_path,
            new_parent,
            insertion_index,
        )
        .map(|_| ())
    }

    /// Resolve a visible hierarchy path to its exact editable placement.
    pub fn occurrence_target(
        &self,
        project: &EditorProject,
        root: u16,
        path: &[usize],
    ) -> Option<PlacementTarget> {
        let materialized = project.materialize().ok()?;
        geometry::build_page(
            project,
            &materialized,
            root,
            self.pages.get(&root).map_or(0, |page| page.1),
        )
        .items
        .into_iter()
        .find(|item| item.path == path)?
        .target
    }

    /// Select a visible hierarchy occurrence, retaining repeated-object identity.
    pub fn select_occurrence(
        &mut self,
        project: &EditorProject,
        root: u16,
        path: &[usize],
    ) -> bool {
        let Ok(materialized) = project.materialize() else {
            return false;
        };
        let page = self.pages.get(&root).map_or(0, |page| page.1);
        let scene = geometry::build_page(project, &materialized, root, page);
        if !scene
            .items
            .iter()
            .any(|item| item.path == path && item.target.is_some())
        {
            return false;
        }
        let keys = super::softkeys::build(project, &materialized.pool, root, page);
        self.pages.insert(root, (keys.mask_id, keys.page));
        self.switch_mask(Some(root));
        self.softkey_mask = keys.mask_id;
        self.softkey_page = keys.page;
        self.revision = project.revision();
        self.selected = vec![path.to_vec()];
        self.sync_selection(project, &scene);
        true
    }

    /// Select an exact editable occurrence within a mask after hierarchy navigation.
    pub fn select_placement(
        &mut self,
        project: &EditorProject,
        root: u16,
        target: PlacementTarget,
    ) -> bool {
        let Ok(materialized) = project.materialize() else {
            return false;
        };
        let page = self.pages.get(&root).map_or(0, |page| page.1);
        let scene = geometry::build_page(project, &materialized, root, page);
        let Some(item) = scene
            .items
            .iter()
            .find(|item| item.target.as_ref() == Some(&target))
        else {
            return false;
        };
        let keys = super::softkeys::build(project, &materialized.pool, root, page);
        self.pages.insert(root, (keys.mask_id, keys.page));
        self.switch_mask(Some(root));
        self.softkey_mask = keys.mask_id;
        self.softkey_page = keys.page;
        self.revision = project.revision();
        self.selected = vec![item.path.clone()];
        self.sync_selection(project, &scene);
        true
    }

    /// Select a placement beneath the exact hierarchy occurrence identified
    /// by its canonical path. This disambiguates shared parents placed more
    /// than once in the same mask and reveals the page containing a soft-key
    /// branch when needed.
    pub fn select_hierarchy_placement(
        &mut self,
        project: &EditorProject,
        root: u16,
        parent_path: &[usize],
        target: PlacementTarget,
    ) -> bool {
        if let Some(&tree_slot) = parent_path.first().filter(|slot| **slot > 0) {
            self.select_softkey_slot(project, root, tree_slot - 1);
        }
        let Ok(materialized) = project.materialize() else {
            return false;
        };
        let page = self.pages.get(&root).map_or(0, |page| page.1);
        let scene = geometry::build_page(project, &materialized, root, page);

        let Some(scene_path) = hierarchy_scene_path(project, &materialized, root, parent_path)
        else {
            return false;
        };

        let Some(item) = scene.items.iter().find(|item| {
            item.target.as_ref() == Some(&target)
                && item.path.len() == scene_path.len() + 1
                && item.path.starts_with(&scene_path)
        }) else {
            return false;
        };
        self.select_occurrence(project, root, &item.path)
    }

    /// Duplicate the selected placement through the canvas's undoable copy operation.
    pub fn duplicate_selection(&mut self, project: &mut EditorProject) -> bool {
        let Some(target) = self.selected_placement(project) else {
            return false;
        };
        let Some((x, y)) = position(project, &target) else {
            return false;
        };
        self.commit(
            project,
            "Duplicate placements",
            vec![CanvasEdit::Duplicate { target, x, y }],
        );
        true
    }

    /// Remove only the selected occurrence, retaining its underlying object definition.
    pub fn remove_selection(&mut self, project: &mut EditorProject) -> bool {
        let Some(target) = self.selected_placement(project) else {
            return false;
        };
        self.commit(
            project,
            "Remove placement",
            vec![CanvasEdit::Remove { target }],
        );
        true
    }

    /// Keep the occurrence selected after replacing its child at the same slot.
    pub fn replaced_placement(
        &mut self,
        project: &EditorProject,
        target: &PlacementTarget,
        replacement_id: u16,
    ) {
        let Some(root) = self.mask else {
            return;
        };
        let Ok(materialized) = project.materialize() else {
            return;
        };
        let scene = geometry::build_page(project, &materialized, root, self.softkey_page);
        let Some(item) = scene.items.iter().find(|item| {
            item.target.as_ref() == Some(target)
                && (item.selection_id == replacement_id || item.object_id == replacement_id)
        }) else {
            return;
        };
        self.selected = vec![item.path.clone()];
        self.sync_selection(project, &scene);
    }

    /// Cancel unfinished canvas edits when its pane is hidden or moved.
    /// Completed edits, selection, zoom, and pan remain intact.
    pub fn cancel_interaction(&mut self) {
        self.gesture = None;
        self.field_draft = None;
    }

    /// Show the active mask editor. All coordinates committed to the project are
    /// parent-relative integer pool pixels; preview transformations stay local.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut EditorProject,
        materialized: &MaterializedProject,
        root: u16,
    ) {
        let mut view = MaskViewState::default();
        view.visible = vec![root];
        view.focused = Some(root);
        self.show_frame(ui, project, materialized, &mut view, false);
    }

    /// Render visible mask/soft-key pairs in a shared viewport. View choices and
    /// key pages are transient; placement edits retain parent-local coordinates.
    pub fn show_masks(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut EditorProject,
        materialized: &MaterializedProject,
        view: &mut MaskViewState,
    ) {
        view.reconcile(project);
        self.show_frame(ui, project, materialized, view, true);
    }

    /// Reveal a soft-key occurrence selected in the hierarchy. Slot indices are
    /// from the linked SoftKeyMask, before paging or pointer resolution.
    pub fn select_softkey_slot(&mut self, project: &EditorProject, root: u16, slot: usize) {
        let keys = super::softkeys::build(project, project.get_pool(), root, 0);
        let capacity = usize::from(project.soft_key_layout().physical_key_count)
            - usize::from(keys.navigation);
        let page = (slot / capacity).min(keys.pages - 1);
        let retains_occurrence = self.mask == Some(root)
            && self.softkey_mask == keys.mask_id
            && self.softkey_page == page
            && self
                .selected
                .iter()
                .any(|path| path.starts_with(&[super::softkeys::PATH_PREFIX, slot]));
        self.pages.insert(root, (keys.mask_id, page));
        if self.mask == Some(root)
            && (self.softkey_mask != keys.mask_id || self.softkey_page != page)
        {
            self.softkey_mask = keys.mask_id;
            self.change_softkey_page(page);
        }
        self.preferred_slot = Some((
            root,
            slot,
            project.get_mut_selected().get().0.map(|id| id.value()),
        ));
        if !retains_occurrence {
            self.inspector = None;
        }
    }

    /// Reveal the first soft-key page containing an externally selected object,
    /// retaining the current page when the object already occurs there.
    pub fn navigate_to_object(
        &mut self,
        project: &EditorProject,
        materialized: &MaterializedProject,
        root: u16,
        object: u16,
    ) {
        // Explicit hierarchy slot selection takes precedence over ID navigation.
        if self
            .preferred_slot
            .is_some_and(|(owner, _, selected)| owner == root && selected == Some(object))
        {
            return;
        }
        self.preferred_slot = None;
        let current = self.pages.get(&root).map(|p| p.1).unwrap_or(0);
        let keys = super::softkeys::build(project, &materialized.pool, root, current);
        let contains = |page| {
            super::softkeys::build(project, &materialized.pool, root, page)
                .slots
                .iter()
                .any(|slot| match slot.content {
                    super::softkeys::SlotContent::Key {
                        id: key,
                        selection_id,
                    } => {
                        selection_id == object
                            || super::mask_view::subtree_contains(&materialized.pool, key, object)
                    }
                    _ => false,
                })
        };
        if !contains(keys.page) {
            if let Some(page) = (0..keys.pages).find(|page| contains(*page)) {
                self.pages.insert(root, (keys.mask_id, page));
                if self.mask == Some(root) {
                    self.softkey_mask = keys.mask_id;
                    self.change_softkey_page(page);
                }
            }
        }
    }

    fn switch_mask(&mut self, mask: Option<u16>) {
        if self.mask == mask {
            return;
        }
        self.cancel_interaction();
        self.selected.clear();
        self.hierarchy_selection = None;
        self.hierarchy_selected.clear();
        self.hierarchy_anchor = None;
        self.hierarchy_selection_explicit = false;
        self.context.clear();
        self.inspector = None;
        self.error = None;
        self.mask = mask;
        self.scene_identity.clear();
        self.scene_origins.clear();
        self.placement_owner = None;
        let (linked, page) = mask
            .and_then(|id| self.pages.get(&id).copied())
            .unwrap_or((None, 0));
        self.softkey_mask = linked;
        self.softkey_page = page;
        self.scene_identity.clear();
        if let Some(root) = self.mask {
            self.pages.insert(root, (self.softkey_mask, page));
        }
    }

    fn show_frame(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut EditorProject,
        materialized: &MaterializedProject,
        view: &mut MaskViewState,
        captions: bool,
    ) {
        let mut enter_preview = false;
        let mut leave_preview = false;
        ui.horizontal(|ui| {
            leave_preview = ui
                .selectable_label(self.preview.is_none(), "Design")
                .clicked();
            enter_preview = ui
                .selectable_label(self.preview.is_some(), "Preview")
                .clicked();
        });
        if leave_preview {
            self.preview = None;
        }
        if enter_preview && self.preview.is_none() {
            self.cancel_interaction();
            self.preview = Some(super::preview::PreviewMode::new(
                project,
                &materialized.pool,
                view.focused,
            ));
        }
        if let Some(preview) = &mut self.preview {
            preview.show(ui, project, &materialized.pool);
            return;
        }
        let frame_revision = project.revision();
        if !ui.input(|i| i.focused) {
            self.cancel_interaction();
        }
        self.pages.retain(|id, _| {
            ObjectId::new(*id)
                .ok()
                .and_then(|id| project.get_pool().object_by_id(id))
                .is_some()
        });
        let mut pairs = Vec::new();
        let mut x = 0.0;
        let mask_rect = Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(project.mask_size as f32, project.mask_size as f32),
        );
        for &root in &view.visible {
            let (linked, page) = self.pages.get(&root).copied().unwrap_or((None, 0));
            let mut keys = super::softkeys::build(project, &materialized.pool, root, page);
            if linked != keys.mask_id {
                keys = super::softkeys::build(project, &materialized.pool, root, 0);
                if self.mask == Some(root) {
                    self.cancel_interaction();
                    self.selected.clear();
                    self.context.clear();
                }
            }
            self.pages.insert(root, (keys.mask_id, keys.page));
            let rect = if keys.rect.is_positive() {
                mask_rect.union(keys.rect)
            } else {
                mask_rect
            };
            let offset = egui::vec2(x, if captions { 24.0 } else { 0.0 });
            pairs.push((root, offset, rect));
            x += rect.width() + 32.0;
            if let Some(error) = keys.error {
                crate::ui_style::error_text(
                    ui,
                    &format!("{}: {error}", project.format_object_id(root)),
                );
            }
        }
        if self.pair_layout != pairs {
            self.cancel_interaction();
            self.pair_layout.clone_from(&pairs);
        }
        let focused_mask = view.focused.filter(|id| view.visible.contains(id));
        let navigating_mask = focused_mask.is_some() && self.mask != focused_mask;
        self.switch_mask(focused_mask);
        if let Some(root) = self.mask {
            let (linked, page) = self.pages[&root];
            if self.softkey_mask != linked || self.softkey_page != page {
                self.softkey_mask = linked;
                self.change_softkey_page(page);
            }
        }
        let mut root = self.mask.unwrap_or(u16::MAX);
        if self.revision != project.revision() {
            self.gesture = None;
            self.field_draft = None;
            self.revision = project.revision();
        }
        let mut softkeys =
            super::softkeys::build(project, &materialized.pool, root, self.softkey_page);
        if self.softkey_mask != softkeys.mask_id {
            self.softkey_mask = softkeys.mask_id;
            self.change_softkey_page(0);
            softkeys = super::softkeys::build(project, &materialized.pool, root, 0);
        }
        if self.softkey_page != softkeys.page {
            self.change_softkey_page(softkeys.page);
        }
        if softkeys.pages > 1 {
            let mut page = softkeys.page;
            ui.horizontal(|ui| {
                if softkeys.navigation == 0
                    && ui
                        .add_enabled(page > 0, egui::Button::new("Previous keys"))
                        .clicked()
                {
                    page -= 1;
                }
                ui.label(format!(
                    "Softkeys · page {} / {}",
                    softkeys.page + 1,
                    softkeys.pages
                ));
                if softkeys.navigation == 0
                    && ui
                        .add_enabled(page + 1 < softkeys.pages, egui::Button::new("Next keys"))
                        .clicked()
                {
                    page += 1;
                }
            });
            if page != self.softkey_page {
                self.change_softkey_page(page);
                softkeys = super::softkeys::build(project, &materialized.pool, root, page);
            }
        }
        let mut scene = geometry::build_page(project, materialized, root, self.softkey_page);
        let identity = Self::scene_identity(&scene);
        if !self.scene_identity.is_empty() && self.scene_identity != identity {
            self.selected = self.reconciled_selection(&scene);
            self.context.clear();
            // Keep inspector identity and placement ownership, so an invalidated
            // occurrence cannot silently become a pool-definition action.
        }
        self.remember_scene(&scene);
        self.selected
            .retain(|p| scene.items.iter().any(|i| &i.path == p));
        if !scene.items.iter().any(|i| i.path == self.context) {
            self.context.clear();
        }
        let inspector = project.get_selected().0.map(|i| i.value());
        if inspector != self.inspector {
            self.inspector = inspector;
            self.hierarchy_selected.clear();
            self.hierarchy_anchor = None;
            self.hierarchy_selection = None;
            self.hierarchy_selection_explicit = false;
            self.selected = scene
                .items
                .iter()
                .filter(|i| self.preferred_slot.is_none_or(|(owner, slot, selected)| {
                    owner != root || selected != inspector ||
                        i.path.starts_with(&[super::softkeys::PATH_PREFIX, slot])
                }))
                .find(|i| (i.target.is_some() || i.path.first() == Some(&super::softkeys::PATH_PREFIX)) && (Some(i.selection_id) == inspector || inspector.and_then(|id| materialized.source_map.by_object_id.get(&id)).is_some_and(|source| matches!((source, &i.target), (crate::components::MaterializedSource::ComponentNode { instance_id, .. }, Some(PlacementTarget::Component { instance_id: target })) if instance_id == target))))
                .map(|i| vec![i.path.clone()])
                .unwrap_or_default();
            self.placement_owner = self
                .selected
                .last()
                .and_then(|path| {
                    scene
                        .items
                        .iter()
                        .find(|item| &item.path == path && item.target.is_some())
                })
                .map(|item| item.selection_id);
        }
        let mut items = self.selection(&scene);
        let mut commands = vec![];
        self.toolbar(ui, project, &scene, &items, &mut commands);
        if let Some(error) = &self.error {
            crate::ui_style::error_text(ui, error);
        }
        let size = ui.available_size().max(egui::vec2(100.0, 100.0));
        let (viewport, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let response = ui.interact(
            viewport,
            ui.id().with("layout_canvas"),
            egui::Sense::click_and_drag(),
        );
        if self.viewport.is_finite()
            && (self.viewport.min - viewport.min).length_sq() > 0.25
            && (self.gesture.is_some() || self.field_draft.is_some())
        {
            self.cancel_interaction();
        }
        self.viewport = viewport;
        let visuals = ui.visuals();
        let selection_color = visuals.selection.stroke.color;
        let guide_color = visuals.warn_fg_color;
        let workspace_fill = visuals.faint_bg_color;
        let mut offset = pairs
            .iter()
            .find(|(id, _, _)| *id == root)
            .map(|(_, offset, _)| *offset)
            .unwrap_or(Vec2::ZERO);
        let explicit_fit = self.fit_all || self.fit.is_some();
        if captions && !self.initially_fitted && !pairs.is_empty() {
            self.fit_all = true;
            self.initially_fitted = true;
        }
        let all_bounds = pairs
            .iter()
            .map(|(_, offset, rect)| {
                let mut rect = rect.translate(*offset);
                if captions {
                    rect.min.y -= 24.0;
                }
                rect
            })
            .reduce(|a, b| a.union(b));
        if self.fit_all || self.fit.is_some() || navigating_mask {
            let fit_rect = if self.fit_all {
                all_bounds
            } else if self.fit == Some(true) {
                bounds(&items).map(|rect| rect.translate(offset))
            } else {
                pairs
                    .iter()
                    .find(|(id, _, _)| *id == root)
                    .map(|(_, offset, rect)| rect.translate(*offset))
            };
            self.fit_all = false;
            self.fit = None;
            if let Some(fit_rect) = fit_rect {
                let fit_zoom = ((viewport.width() - 48.0) / fit_rect.width().max(1.0))
                    .min((viewport.height() - 48.0) / fit_rect.height().max(1.0))
                    .clamp(0.1, 8.0);
                self.zoom = if explicit_fit {
                    fit_zoom
                } else {
                    self.zoom.min(fit_zoom)
                };
                self.pan = viewport.size() * 0.5 - fit_rect.center().to_vec2() * self.zoom;
            }
        }
        let input = ui.input(|i| {
            (
                i.pointer.clone(),
                i.modifiers,
                i.key_down(Key::Space),
                i.raw_scroll_delta,
                i.focused,
            )
        });
        let (pointer, modifiers, space, scroll, focused) = input;
        if !focused {
            self.gesture = None;
        }
        if let Some(gesture) = self.gesture.as_ref() {
            let shape_point = match &gesture.kind {
                GestureKind::PolygonVertex(index) => Some(*index),
                _ => None,
            };
            let shape_object = match &gesture.kind {
                GestureKind::PolygonVertex(_) | GestureKind::LineEndpoint(_) => {
                    gesture.items.first().map(|item| item.object_id)
                }
                _ => None,
            };
            if shape_object.is_some_and(|object_id| {
                gesture.base_selection != self.selected
                    || !crate::shape_editing::editing(ui.ctx(), project, object_id)
                    || shape_point.is_some_and(|point| {
                        crate::shape_editing::selected_point(ui.ctx(), project, object_id)
                            != Some(point)
                    })
            }) {
                self.gesture = None;
            }
        }
        if response.hovered() && self.gesture.is_none() && scroll != Vec2::ZERO {
            if modifiers.ctrl || modifiers.command {
                let anchor = pointer.hover_pos().unwrap_or(viewport.center()) - viewport.min;
                let previous = self.zoom;
                self.zoom = (self.zoom * (scroll.y * 0.003).exp()).clamp(0.1, 8.0);
                self.pan = anchor - (anchor - self.pan) * (self.zoom / previous);
            } else {
                self.pan += scroll;
            }
        }
        if pointer.button_pressed(egui::PointerButton::Primary) && response.hovered() && !space {
            if let Some(p) = pointer.interact_pos() {
                let world = Pos2::ZERO + (p - viewport.min - self.pan) / self.zoom;
                if let Some(&(clicked, clicked_offset, _)) =
                    pairs.iter().find(|(_, offset, rect)| {
                        rect.translate(*offset).contains(world)
                            || captions && {
                                let origin = viewport.min + self.pan + *offset * self.zoom;
                                Rect::from_min_size(
                                    origin - egui::vec2(0.0, 26.0),
                                    egui::vec2(rect.width() * self.zoom, 24.0),
                                )
                                .contains(p)
                            }
                    })
                {
                    if self.mask != Some(clicked) {
                        // Losing a numeric field's focus must not commit its
                        // unfinished draft when the pointer changes mask owner.
                        commands.clear();
                        view.focus(clicked);
                        self.switch_mask(Some(clicked));
                        root = clicked;
                        offset = clicked_offset;
                        softkeys = super::softkeys::build(
                            project,
                            &materialized.pool,
                            root,
                            self.softkey_page,
                        );
                        scene =
                            geometry::build_page(project, materialized, root, self.softkey_page);
                        items.clear();
                    }
                }
            }
        }
        let origin = viewport.min + self.pan + offset * self.zoom;
        let zoom = self.zoom;
        let to_pool = |p: Pos2| Pos2::ZERO + (p - origin) / zoom;
        let hover = pointer
            .hover_pos()
            .filter(|p| viewport.contains(*p))
            .map(to_pool);
        let hit = hover.and_then(|p| self.hit(&scene, p, modifiers.ctrl || modifiers.command));
        let can_key = self.mask.is_some() && (response.has_focus() || self.gesture.is_some());
        if can_key {
            self.keys(ui, project, &scene, modifiers, &mut commands);
        } else if self.mask.is_some() && response.hovered() && !ui.ctx().wants_keyboard_input() {
            // Arrow keys also move the selection while the pointer is over the canvas, so an
            // object selected in the hierarchy or Properties can be nudged without clicking it
            self.nudge(ui, project, &scene, modifiers, &mut commands);
        }
        if pointer.button_pressed(egui::PointerButton::Primary)
            && response.hovered()
            && (self.mask.is_some() || space)
        {
            response.request_focus();
            self.preferred_slot = None;
            if let Some(p) = pointer.interact_pos() {
                let pool_pos = to_pool(p);
                if let Some(slot) = softkeys
                    .slots
                    .iter()
                    .find(|slot| slot.rect.contains(pool_pos))
                {
                    let page = match slot.content {
                        super::softkeys::SlotContent::Previous => {
                            self.softkey_page.saturating_sub(1)
                        }
                        super::softkeys::SlotContent::Next if softkeys.navigation == 1 => {
                            (self.softkey_page + 1) % softkeys.pages
                        }
                        super::softkeys::SlotContent::Next => {
                            (self.softkey_page + 1).min(softkeys.pages - 1)
                        }
                        _ => self.softkey_page,
                    };
                    if page != self.softkey_page {
                        self.change_softkey_page(page);
                        ui.ctx().request_repaint();
                        return;
                    }
                }
                let handle = if items.len() == 1 {
                    resize_handle(
                        &items[0],
                        p,
                        origin,
                        self.zoom,
                        dimensions(project, &items[0]),
                    )
                } else {
                    None
                };
                let shape_handle = if items.len() == 1 {
                    shape_handle(project, ui.ctx(), &items[0], p, origin, self.zoom)
                } else {
                    None
                };
                let kind = if space {
                    GestureKind::Pan
                } else if let Some(kind) = shape_handle {
                    if let (Some(item), GestureKind::PolygonVertex(index)) = (items.first(), &kind)
                    {
                        crate::shape_editing::select_point(
                            ui.ctx(),
                            project,
                            item.object_id,
                            *index,
                        );
                    }
                    kind
                } else if let Some(h) = handle {
                    GestureKind::Resize(h)
                } else if let Some(hit) = hit {
                    if modifiers.shift {
                        if let Some(index) = self.selected.iter().position(|path| *path == hit.path)
                        {
                            self.selected.remove(index);
                        } else {
                            self.selected.push(hit.path.clone());
                        }
                    } else if !self.selected.contains(&hit.path) {
                        self.selected = vec![hit.path.clone()];
                    }
                    if hit.target.is_none()
                        && hit.path.first() == Some(&super::softkeys::PATH_PREFIX)
                    {
                        self.context = hit.path.clone();
                    }
                    GestureKind::Move
                } else {
                    if !modifiers.shift {
                        self.selected.clear();
                    }
                    if !modifiers.shift
                        && (mask_rect.contains(pool_pos)
                            || captions
                                && pool_pos.y < 0.0
                                && pool_pos.y >= -26.0 / self.zoom
                                && pool_pos.x >= 0.0
                                && pool_pos.x <= mask_rect.width())
                    {
                        self.selected = vec![Vec::new()];
                    }
                    GestureKind::Marquee
                };
                self.sync_selection(project, &scene);
                self.gesture = Some(Gesture {
                    kind,
                    start: pool_pos,
                    current: pool_pos,
                    items: root_items(self.selection(&scene)),
                    edits: vec![],
                    draft: None,
                    guides: vec![],
                    changed: false,
                    fingerprint: vec![],
                    base_selection: self.selected.clone(),
                });
            }
        }
        if pointer.button_pressed(egui::PointerButton::Secondary) {
            self.secondary_panned = false;
            if response.hovered() {
                self.add_target =
                    hover.and_then(|p| Self::add_target(project, &scene, &softkeys, root, p));
                self.add_object_query.clear();
            }
        }
        if (pointer.button_pressed(egui::PointerButton::Middle)
            || pointer.button_pressed(egui::PointerButton::Secondary))
            && response.hovered()
        {
            if let Some(p) = pointer.interact_pos() {
                self.gesture = Some(Gesture {
                    kind: GestureKind::Pan,
                    start: to_pool(p),
                    current: to_pool(p),
                    items: vec![],
                    edits: vec![],
                    draft: None,
                    guides: vec![],
                    changed: false,
                    fingerprint: vec![],
                    base_selection: vec![],
                });
            }
        }
        let owns_pointer = response.hovered()
            || response.dragged_by(egui::PointerButton::Primary)
            || response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || response.drag_stopped_by(egui::PointerButton::Primary)
            || response.drag_stopped_by(egui::PointerButton::Secondary)
            || response.drag_stopped_by(egui::PointerButton::Middle);
        if let Some(p) = owns_pointer
            .then(|| pointer.interact_pos())
            .flatten()
            .or_else(|| {
                (owns_pointer && self.gesture.is_some())
                    .then(|| {
                        self.gesture
                            .as_ref()
                            .map(|g| origin + g.current.to_vec2() * zoom)
                    })
                    .flatten()
            })
        {
            if let Some(mut gesture) = self.gesture.take() {
                gesture.current = to_pool(p);
                let raw = gesture.current - gesture.start;
                gesture.changed |= raw.length() * self.zoom > 3.0;
                if gesture.changed {
                    gesture.edits.clear();
                    match gesture.kind {
                        GestureKind::Pan => {
                            self.pan += pointer.delta();
                            self.secondary_panned |= pointer
                                .button_down(egui::PointerButton::Secondary)
                                || pointer.button_released(egui::PointerButton::Secondary);
                        }
                        GestureKind::Marquee => {
                            let marquee = Rect::from_two_pos(gesture.start, gesture.current);
                            self.selected = gesture.base_selection.clone();
                            for item in scene.items.iter().filter(|i| {
                                i.target.is_some()
                                    && marquee.intersects(i.hit_rect)
                                    && i.hit_rect.is_positive()
                            }) {
                                if (modifiers.ctrl
                                    || modifiers.command
                                    || item.path.len() == self.context.len() + 1
                                        && item.path.starts_with(&self.context))
                                    && !self.selected.contains(&item.path)
                                {
                                    self.selected.push(item.path.clone());
                                }
                            }
                            self.sync_selection(project, &scene);
                        }
                        GestureKind::Move => {
                            let delta = constrain(raw, modifiers.shift);
                            let (delta, guides) = snap_delta(
                                &gesture.items,
                                &scene,
                                delta,
                                self.zoom,
                                self.snap_objects && !modifiers.ctrl,
                                self.snap_spacing && !modifiers.ctrl,
                                modifiers.shift,
                            );
                            gesture.guides = guides;
                            gesture.edits =
                                move_edits(project, &gesture.items, delta, modifiers.alt);
                        }
                        GestureKind::Resize(handle) => {
                            if let Some(item) = gesture.items.first() {
                                let (delta, guides) = snap_resize_delta(
                                    project,
                                    item,
                                    &scene,
                                    handle,
                                    raw,
                                    modifiers,
                                    self.zoom,
                                    self.snap_objects,
                                );
                                gesture.guides = guides;
                                gesture.edits =
                                    resize_edits(project, item, handle, delta, modifiers);
                            }
                        }
                        GestureKind::PolygonVertex(index) => {
                            if let Some(item) = gesture.items.first() {
                                if let Some(Object::OutputPolygon(polygon)) =
                                    ObjectId::new(item.object_id)
                                        .ok()
                                        .and_then(|id| project.get_pool().object_by_id(id))
                                {
                                    let brush = f32::from(line_brush(
                                        project.get_pool(),
                                        polygon.line_attributes,
                                    )) * 0.5;
                                    let x = (gesture.current.x - item.rect.min.x - brush)
                                        .round()
                                        .clamp(0.0, f32::from(polygon.width.saturating_sub(1)))
                                        as u16;
                                    let y = (gesture.current.y - item.rect.min.y - brush)
                                        .round()
                                        .clamp(0.0, f32::from(polygon.height.saturating_sub(1)))
                                        as u16;
                                    if let Some(target) = item.target.clone() {
                                        gesture.edits.push(CanvasEdit::SetPolygonPoint {
                                            object_id: item.object_id,
                                            target,
                                            index,
                                            x,
                                            y,
                                        });
                                    }
                                }
                            }
                        }
                        GestureKind::LineEndpoint(endpoint) => {
                            if let Some(item) = gesture.items.first() {
                                if let Some(edit) =
                                    line_endpoint_edit(project, item, endpoint, gesture.current)
                                {
                                    gesture.edits.push(edit);
                                }
                            }
                        }
                    }
                    if gesture.edits.is_empty() {
                        gesture.draft = None;
                        gesture.fingerprint.clear();
                    }
                    if !gesture.edits.is_empty() {
                        let fingerprint = gesture.edits.clone();
                        if fingerprint != gesture.fingerprint {
                            let mut draft = project.canvas_preview_copy();
                            match edits::apply(&mut draft, "Layout preview", &gesture.edits)
                                .and_then(|targets| {
                                    draft
                                        .materialize()
                                        .map(|m| (m, targets))
                                        .map_err(|e| e.to_string())
                                }) {
                                Ok(draft) => {
                                    gesture.draft = Some(draft);
                                    self.error = None;
                                }
                                Err(error) => {
                                    gesture.draft = None;
                                    self.error = Some(error);
                                }
                            }
                            gesture.fingerprint = fingerprint;
                        }
                    }
                }
                let stopped = response.drag_stopped_by(egui::PointerButton::Primary)
                    || response.drag_stopped_by(egui::PointerButton::Secondary)
                    || response.drag_stopped_by(egui::PointerButton::Middle)
                    || response.hovered()
                        && (pointer.button_released(egui::PointerButton::Primary)
                            || pointer.button_released(egui::PointerButton::Secondary)
                            || pointer.button_released(egui::PointerButton::Middle));
                if stopped {
                    if gesture.changed && gesture.draft.is_some() && !gesture.edits.is_empty() {
                        let description = match gesture.kind {
                            GestureKind::Resize(_) => "Resize object",
                            GestureKind::PolygonVertex(_) => "Move polygon point",
                            GestureKind::LineEndpoint(_) => "Move line endpoint",
                            _ if modifiers.alt => "Duplicate placements",
                            _ => "Move placements",
                        };
                        self.commit(project, description, gesture.edits);
                    }
                } else {
                    self.gesture = Some(gesture);
                }
            }
        }
        if response.double_clicked() {
            if let Some(item) = hit {
                self.context = item.path.clone();
                if let Some(child) = scene.items.iter().rev().find(|i| {
                    i.path.starts_with(&item.path)
                        && i.path.len() == item.path.len() + 1
                        && hover.is_some_and(|p| i.hit_rect.contains(p))
                }) {
                    self.selected = vec![child.path.clone()];
                    self.sync_selection(project, &scene);
                }
            }
        }
        if !commands.is_empty() {
            self.commit(project, "Edit layout", commands);
        }
        // Disable simulated VT interactions and transform only the mask shapes.
        // Canvas overlays stay in screen coordinates on the same paint layer.
        let display = self
            .gesture
            .as_ref()
            .and_then(|g| g.draft.as_ref())
            .map(|d| &d.0)
            .unwrap_or(materialized);
        let workspace_painter = ui.painter().with_clip_rect(viewport);
        workspace_painter.rect_filled(viewport, 0.0, workspace_fill);
        for &(pair_root, pair_offset, _) in &pairs {
            let pair_transform = egui::emath::TSTransform::new(
                viewport.min.to_vec2() + self.pan + pair_offset * self.zoom,
                self.zoom,
            );
            let page = self.pages.get(&pair_root).map(|p| p.1).unwrap_or(0);
            let pair_keys = super::softkeys::build(project, &display.pool, pair_root, page);
            self.paint_pair(
                ui,
                project,
                display,
                pair_root,
                viewport,
                mask_rect,
                pair_transform,
                &pair_keys,
            );
            if self.selected.iter().any(Vec::is_empty) && self.inspector == Some(pair_root) {
                workspace_painter.rect_stroke(
                    pair_transform * mask_rect,
                    0.0,
                    Stroke::new(1.5, selection_color),
                    egui::StrokeKind::Inside,
                );
            }
            if self.grid_visible {
                let mask_screen = pair_transform * mask_rect;
                let grid_painter =
                    workspace_painter.with_clip_rect(viewport.intersect(mask_screen));
                let minimum = (8.0 / self.zoom).max(1.0);
                let magnitude = 10.0_f32.powf(minimum.log10().floor());
                let step = [1.0, 2.0, 5.0, 10.0]
                    .into_iter()
                    .map(|factor| factor * magnitude)
                    .find(|step| *step >= minimum)
                    .unwrap_or(10.0 * magnitude);
                let color = ui.visuals().text_color().gamma_multiply(0.25);
                let visible = (pair_transform.inverse() * viewport).intersect(mask_rect);
                if visible.is_positive() {
                    for horizontal in [false, true] {
                        let (start, end) = if horizontal {
                            (visible.top(), visible.bottom())
                        } else {
                            (visible.left(), visible.right())
                        };
                        for index in (start / step).ceil() as i32..=(end / step).floor() as i32 {
                            let coordinate = index as f32 * step;
                            let (a, b) = if horizontal {
                                (
                                    egui::pos2(visible.left(), coordinate),
                                    egui::pos2(visible.right(), coordinate),
                                )
                            } else {
                                (
                                    egui::pos2(coordinate, visible.top()),
                                    egui::pos2(coordinate, visible.bottom()),
                                )
                            };
                            grid_painter.line_segment(
                                [pair_transform * a, pair_transform * b],
                                Stroke::new(1.0, color),
                            );
                        }
                    }
                }
            }
            if captions {
                if let Some(object) = ObjectId::new(pair_root)
                    .ok()
                    .and_then(|id| display.pool.object_by_id(id))
                {
                    let name = project.get_object_info(object).get_name(object);
                    workspace_painter.text(
                        pair_transform * egui::pos2(0.0, -6.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{} · {}", name, project.format_object_id(pair_root)),
                        egui::FontId::proportional(14.0),
                        if self.mask == Some(pair_root) {
                            selection_color
                        } else {
                            ui.visuals().text_color()
                        },
                    );
                }
            }
        }
        let transform = egui::emath::TSTransform::new(
            viewport.min.to_vec2() + self.pan + offset * self.zoom,
            self.zoom,
        );
        if pairs.is_empty() {
            workspace_painter.text(
                viewport.center(),
                egui::Align2::CENTER_CENTER,
                "Check a mask in the Objects pane to preview it.",
                egui::FontId::proportional(14.0),
                ui.visuals().weak_text_color(),
            );
        }
        let painter = ui.painter().with_clip_rect(viewport);
        let screen = |r: Rect| transform * r;
        let draft_scene = self
            .gesture
            .as_ref()
            .and_then(|g| g.draft.as_ref())
            .map(|(m, _)| geometry::build_page(project, m, root, self.softkey_page));
        let display_scene = draft_scene.as_ref().unwrap_or(&scene);
        let display_items =
            if let Some((_, targets)) = self.gesture.as_ref().and_then(|g| g.draft.as_ref()) {
                display_scene
                    .items
                    .iter()
                    .filter(|i| {
                        i.target.as_ref().is_some_and(|t| targets.contains(t))
                            && self.selected.iter().any(|path| {
                                super::softkeys::area(path) == super::softkeys::area(&i.path)
                            })
                    })
                    .cloned()
                    .collect()
            } else {
                self.selection(display_scene)
            };
        for item in &display_items {
            painter.rect_stroke(
                screen(item.rect),
                0.0,
                Stroke::new(1.5, selection_color),
                egui::StrokeKind::Outside,
            );
        }
        if display_items.len() == 1 {
            let item = &display_items[0];
            if crate::shape_editing::editing(ui.ctx(), project, item.object_id) {
                paint_shape_handles(
                    &painter,
                    &display.pool,
                    item,
                    transform,
                    selection_color,
                    crate::shape_editing::selected_point(ui.ctx(), project, item.object_id),
                );
            }
        }
        if self.gesture.is_none() {
            if let Some(item) = hit {
                painter.rect_stroke(
                    screen(item.rect),
                    0.0,
                    Stroke::new(1.0, selection_color),
                    egui::StrokeKind::Outside,
                );
            }
            if display_items.len() == 1 {
                paint_handles(
                    &painter,
                    &display_items[0],
                    screen,
                    dimensions(project, &display_items[0]),
                    selection_color,
                );
                let rect = display_items[0].rect;
                label(
                    &painter,
                    screen(rect).left_bottom() + egui::vec2(0.0, 17.0),
                    format!("{} × {}", rect.width().round(), rect.height().round()),
                    selection_color,
                );
            }
        }
        if let Some(g) = &self.gesture {
            if matches!(g.kind, GestureKind::Marquee) && g.changed {
                let r = screen(Rect::from_two_pos(g.start, g.current));
                painter.rect_filled(r, 0.0, selection_color.gamma_multiply(0.12));
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.0, selection_color),
                    egui::StrokeKind::Inside,
                );
            }
            for (a, b) in &g.guides {
                painter.line_segment(
                    [transform * *a, transform * *b],
                    Stroke::new(1.0, guide_color),
                );
            }
            for item in &display_items {
                let parent_path = &item.path[..item.path.len().saturating_sub(1)];
                let parent_bounds = display_scene
                    .items
                    .iter()
                    .find(|p| p.path == parent_path)
                    .map(|p| p.rect)
                    .unwrap_or(item.parent_rect);
                painter.rect_stroke(
                    screen(parent_bounds),
                    0.0,
                    Stroke::new(1.0, guide_color),
                    egui::StrokeKind::Outside,
                );
                label(
                    &painter,
                    screen(parent_bounds).left_top(),
                    format!(
                        "Parent {} × {}",
                        parent_bounds.width().round(),
                        parent_bounds.height().round()
                    ),
                    guide_color,
                );
                parent_measure(
                    &painter,
                    screen(item.rect),
                    screen(item.parent_rect),
                    self.zoom,
                    guide_color,
                    selection_color,
                );
            }
        } else if modifiers.alt {
            if let Some(selected) = display_items.first() {
                measure(
                    &painter,
                    screen(selected.rect),
                    screen(
                        hit.filter(|h| h.path != selected.path)
                            .map(|h| h.rect)
                            .unwrap_or(selected.parent_rect),
                    ),
                    self.zoom,
                    guide_color,
                );
            }
        }
        let hit_tolerance = 3.0 / self.zoom;
        if !self.secondary_panned {
            response.context_menu(|ui| {
                self.add_object_menu(ui, project);
                self.group_menu(ui, project, &items);
                ui.menu_button("Select layer", |ui| {
                    if let Some(pos) = hover {
                        for item in scene.items.iter().rev().filter(|i| {
                            i.target.is_some()
                                && i.hit_rect.width() >= 0.0
                                && i.hit_rect.height() >= 0.0
                                && i.hit_rect.expand(hit_tolerance).contains(pos)
                        }) {
                            let label = scene_item_name(project, item);
                            if ui
                                .button(format!("{} · depth {}", label, item.path.len()))
                                .clicked()
                            {
                                self.selected = vec![item.path.clone()];
                                self.context =
                                    item.path[..item.path.len().saturating_sub(1)].to_vec();
                                self.sync_selection(project, &scene);
                                ui.close();
                            }
                        }
                    }
                });
                if ui
                    .add_enabled(!items.is_empty(), egui::Button::new("Duplicate · Ctrl+D"))
                    .clicked()
                {
                    self.commit(
                        project,
                        "Duplicate placements",
                        move_edits(project, &root_items(items.clone()), Vec2::ZERO, true),
                    );
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !items.is_empty(),
                        egui::Button::new("Remove placements · Delete"),
                    )
                    .clicked()
                {
                    self.commit(
                        project,
                        "Remove placements",
                        root_items(items.clone())
                            .into_iter()
                            .filter_map(|i| i.target.map(|target| CanvasEdit::Remove { target }))
                            .collect(),
                    );
                    ui.close();
                }
                if ui
                    .add_enabled(self.mask.is_some(), egui::Button::new("Fit mask · Shift+1"))
                    .clicked()
                {
                    self.fit = Some(false);
                    ui.close();
                }
                if ui.button("Fit selection · Shift+2").clicked() {
                    self.fit = Some(true);
                    ui.close();
                }
            });
        }
        if matches!(
            self.gesture.as_ref().map(|g| &g.kind),
            Some(GestureKind::Pan)
        ) || space && response.hovered()
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if let Some(p) = pointer.hover_pos() {
            if items.len() == 1 {
                if let Some(h) = resize_handle(
                    &items[0],
                    p,
                    viewport.min + self.pan + offset * self.zoom,
                    self.zoom,
                    dimensions(project, &items[0]),
                ) {
                    ui.ctx().set_cursor_icon(handle_cursor(h));
                }
            }
        }
        if project.revision() != frame_revision {
            ui.ctx().request_repaint();
        }
    }

    fn paint_pair(
        &self,
        ui: &mut egui::Ui,
        project: &EditorProject,
        display: &MaterializedProject,
        root: u16,
        viewport: Rect,
        mask_rect: Rect,
        transform: egui::emath::TSTransform,
        softkeys: &super::softkeys::SoftKeys,
    ) {
        let render_scale_id = egui::Id::new("vt_render_scale");
        let previous_scale = ui.ctx().data_mut(|data| {
            let previous = data.get_temp::<f32>(render_scale_id);
            data.insert_temp(render_scale_id, transform.scaling);
            previous
        });
        let workspace_painter = ui.painter().with_clip_rect(viewport);
        let mask_fill = ui.visuals().extreme_bg_color;
        let mask_screen_rect = transform * mask_rect;
        workspace_painter.rect_filled(mask_screen_rect.intersect(viewport), 1.0, mask_fill);
        workspace_painter.rect_stroke(
            mask_screen_rect,
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("mask_objects", root))
                .max_rect(mask_rect),
        );
        child.set_clip_rect(
            (transform.inverse() * viewport.intersect(ui.clip_rect())).intersect(mask_rect),
        );
        child.disable();
        child.set_opacity(1.0);
        child.with_visual_transform(transform, |child| {
            if let Some(object) = ObjectId::new(root)
                .ok()
                .and_then(|id| display.pool.object_by_id(id))
            {
                crate::object_rendering::render_object_with_settings(
                    child,
                    &display.pool,
                    object,
                    Point::default(),
                    project.vt_version,
                    project.soft_key_size(),
                );
            }
        });
        if softkeys.rect.is_positive() {
            workspace_painter.rect_filled(
                transform * softkeys.rect,
                0.0,
                crate::effective_colour::colour(&display.pool, softkeys.background).convert(),
            );
        }
        for slot in &softkeys.slots {
            let rect = transform * slot.rect;
            workspace_painter.rect_stroke(
                rect,
                0.0,
                ui.visuals().widgets.noninteractive.bg_stroke,
                egui::StrokeKind::Outside,
            );
            match slot.content {
                super::softkeys::SlotContent::Key { id, .. } => {
                    let mut key_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("softkey", root, slot.index))
                            .max_rect(slot.rect),
                    );
                    key_ui.set_clip_rect(
                        (transform.inverse() * viewport.intersect(ui.clip_rect()))
                            .intersect(slot.rect),
                    );
                    key_ui.disable();
                    key_ui.set_opacity(1.0);
                    key_ui.with_visual_transform(transform, |key_ui| {
                        if let Some(object) = ObjectId::new(id)
                            .ok()
                            .and_then(|id| display.pool.object_by_id(id))
                        {
                            crate::object_rendering::render_object_with_settings(
                                key_ui,
                                &display.pool,
                                object,
                                Point::default(),
                                project.vt_version,
                                project.soft_key_size(),
                            );
                        }
                    });
                }
                super::softkeys::SlotContent::Previous | super::softkeys::SlotContent::Next => {
                    workspace_painter.rect_filled(rect, 0.0, ui.visuals().widgets.inactive.bg_fill);
                    let previous = matches!(slot.content, super::softkeys::SlotContent::Previous);
                    let enabled = if previous {
                        softkeys.page > 0
                    } else {
                        softkeys.navigation == 1 || softkeys.page + 1 < softkeys.pages
                    };
                    workspace_painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if previous { "◀" } else { "▶" },
                        egui::FontId::proportional(18.0 * self.zoom),
                        if enabled {
                            ui.visuals().text_color()
                        } else {
                            ui.visuals().weak_text_color()
                        },
                    );
                }
                super::softkeys::SlotContent::Invalid => {
                    workspace_painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Invalid key",
                        egui::FontId::proportional(10.0 * self.zoom),
                        crate::ui_style::palette(ui).error,
                    );
                }
                super::softkeys::SlotContent::Empty => {}
            }
        }
        ui.ctx().data_mut(|data| {
            if let Some(scale) = previous_scale {
                data.insert_temp(render_scale_id, scale);
            } else {
                data.remove::<f32>(render_scale_id);
            }
        });
    }

    fn change_softkey_page(&mut self, page: usize) {
        self.cancel_interaction();
        self.softkey_page = page;
        if let Some(root) = self.mask {
            self.pages.insert(root, (self.softkey_mask, page));
        }
        self.selected.clear();
        self.context.clear();
        self.preferred_slot = None;
    }

    fn selection(&self, scene: &Scene) -> Vec<SceneItem> {
        scene
            .items
            .iter()
            .filter(|i| !i.path.is_empty() && self.selected.contains(&i.path))
            .cloned()
            .collect()
    }
    fn hit<'a>(&self, scene: &'a Scene, pos: Pos2, deep: bool) -> Option<&'a SceneItem> {
        let leaf = scene.items.iter().rev().find(|i| {
            (i.target.is_some()
                || i.path.first() == Some(&super::softkeys::PATH_PREFIX) && i.path.len() == 2)
                && i.hit_rect.width() >= 0.0
                && i.hit_rect.height() >= 0.0
                && i.hit_rect.expand(3.0 / self.zoom).contains(pos)
        })?;
        if deep {
            return Some(leaf);
        }
        // Retain the current selected level when dragging its descendants.
        if let Some(selected) = scene.items.iter().find(|i| {
            self.selected.contains(&i.path)
                && i.path.len() > self.context.len()
                && leaf.path.starts_with(&i.path)
        }) {
            return Some(selected);
        }
        let base_depth = if leaf.path.first() == Some(&super::softkeys::PATH_PREFIX) {
            3.min(leaf.path.len())
        } else {
            1
        };
        let depth = if leaf.path.starts_with(&self.context) {
            (self.context.len() + 1).max(base_depth)
        } else {
            base_depth
        };
        scene
            .items
            .iter()
            .find(|i| i.path == leaf.path[..leaf.path.len().min(depth)])
    }
    fn sync_selection(&mut self, project: &EditorProject, scene: &Scene) {
        self.hierarchy_selection = None;
        self.hierarchy_selected.clear();
        self.hierarchy_anchor = None;
        self.hierarchy_selection_explicit = false;
        self.all_hierarchy_references = None;
        let id = self
            .selected
            .last()
            .and_then(|p| scene.items.iter().find(|i| &i.path == p))
            .map(|i| i.selection_id);
        self.inspector = id;
        self.remember_scene(scene);
        self.placement_owner = self
            .selected
            .last()
            .and_then(|path| {
                scene
                    .items
                    .iter()
                    .find(|item| &item.path == path && item.target.is_some())
            })
            .map(|item| item.selection_id);
        project
            .get_mut_selected()
            .set(NullableObjectId(id.and_then(|id| ObjectId::new(id).ok())));
    }
    fn commit(
        &mut self,
        project: &mut EditorProject,
        description: &str,
        commands: Vec<CanvasEdit>,
    ) {
        match edits::apply(project, description, &commands) {
            Ok(targets) => {
                self.error = None;
                self.revision = project.revision();
                if let (Some(root), Ok(m)) = (self.mask, project.materialize()) {
                    let scene = geometry::build_page(project, &m, root, self.softkey_page);
                    self.selected = scene
                        .items
                        .iter()
                        .filter(|i| {
                            i.target.as_ref().is_some_and(|t| targets.contains(t))
                                && self.selected.iter().any(|path| {
                                    super::softkeys::area(path) == super::softkeys::area(&i.path)
                                })
                        })
                        .map(|i| i.path.clone())
                        .collect();
                    self.sync_selection(project, &scene);
                }
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        project: &EditorProject,
        scene: &Scene,
        items: &[SceneItem],
        commands: &mut Vec<CanvasEdit>,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("VIEW");
            if ui.small_button("−").clicked() {
                self.zoom = (self.zoom / 1.25).max(0.1);
            }
            ui.label(format!("{:.0}%", self.zoom * 100.0));
            if ui.small_button("+").clicked() {
                self.zoom = (self.zoom * 1.25).min(8.0);
            }
            if ui.small_button("100%").clicked() {
                self.zoom = 1.0;
            }
            if ui
                .add_enabled(self.mask.is_some(), egui::Button::new("Fit mask"))
                .clicked()
            {
                self.fit = Some(false);
            }
            if ui
                .add_enabled(!items.is_empty(), egui::Button::new("Fit selection"))
                .clicked()
            {
                self.fit = Some(true);
            }
            if ui.small_button("Fit all").clicked() {
                self.fit_all = true;
            }
            ui.separator();
            ui.checkbox(&mut self.grid_visible, "Grid");
            ui.strong("SNAP");
            ui.checkbox(&mut self.snap_objects, "Snap objects");
            ui.checkbox(&mut self.snap_spacing, "Snap spacing");
            ui.menu_button("Shortcuts", |ui| {
                ui.label("Shift-click: toggle selection · drag empty area: marquee");
                ui.label("Double-click / Enter: enter children · Shift+Enter: parent");
                ui.label("Ctrl-click: deep select · Ctrl-drag: bypass snapping");
                ui.label("Shift-drag: constrain axis · Alt-drag: independent copy");
                ui.label("Alt-hover: measure · Arrow: 1 px · Shift+Arrow: 10 px");
                ui.label("Resize: Shift = aspect ratio · Alt = from center");
                ui.label("Ctrl+D: duplicate · Delete: remove placement · Esc: cancel");
                ui.label("Space-drag / middle / right mouse: pan · Ctrl-wheel: zoom");
                ui.label("Shift+1: fit mask · Shift+2: fit selection");
            });
        });
        if items.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.weak("Select a placement to edit its position and dimensions.");
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.strong("SELECTION");
                let path = items.last().unwrap();
                let breadcrumb = scene
                    .items
                    .iter()
                    .filter(|i| path.path.starts_with(&i.path))
                    .map(|i| scene_item_name(project, i))
                    .collect::<Vec<_>>()
                    .join(" / ");
                ui.label(if items.len() > 1 {
                    format!("{breadcrumb} · {} selected", items.len())
                } else {
                    breadcrumb
                });
                if !self.context.is_empty() && ui.small_button("Parent").clicked() {
                    self.context.pop();
                }
                if items.len() == 1 {
                    let item = &items[0];
                    let (w, h) = dimensions(project, item);
                    let current_position = item.target.as_ref().and_then(|t| position(project, t));
                    let identity = (item.path.clone(), project.revision());
                    if self
                        .field_draft
                        .as_ref()
                        .is_none_or(|d| d.identity != identity)
                    {
                        self.field_draft = Some(FieldDraft {
                            identity,
                            position: current_position,
                            width: w,
                            height: h,
                            changed: false,
                            finish_pending: false,
                        });
                    }
                    let draft = self.field_draft.as_mut().unwrap();
                    // A different dock pane can change mask focus later in this
                    // frame. Defer focus-loss completion so it can cancel first.
                    let mut finish = draft.finish_pending;
                    draft.finish_pending = false;
                    if let Some((x, y)) = &mut draft.position {
                        for (name, value) in [("X", x), ("Y", y)] {
                            ui.label(name);
                            let r = ui.add(egui::DragValue::new(value).speed(1.0));
                            draft.changed |= r.changed();
                            finish |= r.drag_stopped();
                            draft.finish_pending |= r.lost_focus();
                        }
                    }
                    for (name, value) in [("W", &mut draft.width), ("H", &mut draft.height)] {
                        if let Some(value) = value {
                            ui.label(name);
                            let r = ui.add(egui::DragValue::new(value).range(0..=u16::MAX));
                            draft.changed |= r.changed();
                            finish |= r.drag_stopped();
                            draft.finish_pending |= r.lost_focus();
                        }
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        draft.position = current_position;
                        draft.width = w;
                        draft.height = h;
                        draft.changed = false;
                        draft.finish_pending = false;
                    }
                    if draft.changed && draft.finish_pending {
                        ui.ctx().request_repaint();
                    }
                    if draft.changed && finish {
                        if draft.width != w || draft.height != h {
                            commands.push(CanvasEdit::Resize {
                                object_id: item.object_id,
                                width: draft.width,
                                height: draft.height,
                            });
                        }
                        if let (Some(target), Some((x, y))) = (&item.target, draft.position) {
                            commands.push(CanvasEdit::Move {
                                target: target.clone(),
                                x,
                                y,
                            });
                        }
                        draft.changed = false;
                    }
                    if w.is_some() || h.is_some() {
                        let references = project
                            .get_pool()
                            .objects()
                            .iter()
                            .filter_map(crate::operations::operation::object_refs)
                            .flatten()
                            .filter(|r| {
                                resolves_to(project.get_pool(), r.id.value(), item.object_id)
                            })
                            .count();
                        if references > 1 {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                format!("Shared dimensions · {references} references"),
                            );
                        } else {
                            ui.label("Object dimensions").on_hover_text("Resizing changes this object's dimensions everywhere it is referenced. Duplicate it for an independent size.");
                        }
                    } else {
                        ui.label("Dimensions read-only");
                    }
                }
                let enabled = same_parent(items);
                ui.menu_button("Align / distribute", |ui| {
                    for (name, action) in [
                        ("Left", 0),
                        ("Horizontal center", 1),
                        ("Right", 2),
                        ("Top", 3),
                        ("Vertical center", 4),
                        ("Bottom", 5),
                        ("Distribute horizontally", 6),
                        ("Distribute vertically", 7),
                    ] {
                        if ui
                            .add_enabled(
                                enabled && (action < 6 || items.len() >= 3),
                                egui::Button::new(name),
                            )
                            .clicked()
                        {
                            commands.extend(align_edits(
                                project,
                                items,
                                action,
                                ui.input(|i| i.modifiers.shift),
                            ));
                            ui.close();
                        }
                    }
                    if !enabled {
                        ui.label("Select placements with the same parent.");
                    }
                    ui.label("Shift-align: align selection as a group to parent");
                });
            });
        }
    }
    fn keys(
        &mut self,
        ui: &mut egui::Ui,
        project: &EditorProject,
        scene: &Scene,
        modifiers: Modifiers,
        commands: &mut Vec<CanvasEdit>,
    ) {
        let take = |key, mods| ui.input_mut(|i| i.consume_key(mods, key));
        if take(Key::Escape, Modifiers::NONE) {
            let canceled_gesture = self.gesture.take().is_some();
            if canceled_gesture {
                // The next Escape leaves shape editing mode.
            } else if let Some(item) = self
                .selection(scene)
                .first()
                .filter(|item| crate::shape_editing::editing(ui.ctx(), project, item.object_id))
            {
                crate::shape_editing::stop_editing(ui.ctx(), project, item.object_id);
            } else {
                self.selected.clear();
                self.context.clear();
                self.sync_selection(project, scene);
            }
            self.error = None;
            return;
        }
        if self.gesture.is_some() {
            return;
        }
        if take(Key::Num1, Modifiers::SHIFT) {
            self.fit = Some(false);
        }
        if take(Key::Num2, Modifiers::SHIFT) {
            self.fit = Some(true);
        }
        if take(Key::G, Modifiers::COMMAND) {
            let targets = group_targets(&self.selection(scene));
            let container = edits::grouping_candidates(project, &targets)
                .into_iter()
                .find(|candidate| {
                    candidate.object_type == ag_iso_stack::object_pool::ObjectType::Container
                });
            match container {
                Some(candidate) if candidate.enabled => {
                    self.request_group(ui.ctx(), project, targets, candidate.object_type)
                }
                Some(candidate) => self.error = candidate.reason,
                None if !targets.is_empty() => {
                    self.error = Some(
                        "These objects cannot be grouped into a Container. Select siblings that a Container can hold.".to_owned(),
                    )
                }
                None => {}
            }
        }
        if take(Key::A, Modifiers::COMMAND) {
            self.selected = scene
                .items
                .iter()
                .filter(|i| {
                    i.target.is_some()
                        && i.path.starts_with(&self.context)
                        && i.path.len() == self.context.len() + 1
                })
                .map(|i| i.path.clone())
                .collect();
            self.sync_selection(project, scene);
        }
        let items = root_items(self.selection(scene));
        if take(Key::Enter, Modifiers::SHIFT) {
            if let Some(item) = items.last() {
                let mut parent = item.path.clone();
                parent.pop();
                self.selected = if parent.is_empty() {
                    vec![]
                } else {
                    vec![parent.clone()]
                };
                self.context = parent[..parent.len().saturating_sub(1)].to_vec();
                self.sync_selection(project, scene);
            }
        } else if take(Key::Enter, Modifiers::NONE) {
            if let Some(item) = items.last() {
                self.context = item.path.clone();
                if let Some(child) = scene.items.iter().find(|i| {
                    i.target.is_some()
                        && i.path.starts_with(&item.path)
                        && i.path.len() == item.path.len() + 1
                }) {
                    self.selected = vec![child.path.clone()];
                    self.sync_selection(project, scene);
                }
            }
        }
        if take(Key::Tab, Modifiers::SHIFT) || take(Key::Tab, Modifiers::NONE) {
            if let Some(item) = items.last() {
                let parent = &item.path[..item.path.len().saturating_sub(1)];
                let siblings: Vec<_> = scene
                    .items
                    .iter()
                    .filter(|i| {
                        i.target.is_some()
                            && i.path.len() == item.path.len()
                            && i.path.starts_with(parent)
                    })
                    .collect();
                if let Some(index) = siblings.iter().position(|i| i.path == item.path) {
                    let next = if modifiers.shift {
                        (index + siblings.len() - 1) % siblings.len()
                    } else {
                        (index + 1) % siblings.len()
                    };
                    self.selected = vec![siblings[next].path.clone()];
                    self.sync_selection(project, scene);
                }
            }
        }
        if take(Key::D, Modifiers::COMMAND) {
            commands.extend(move_edits(project, &items, Vec2::ZERO, true));
        }
        if take(Key::Delete, Modifiers::NONE) || take(Key::Backspace, Modifiers::NONE) {
            commands.extend(
                items
                    .iter()
                    .filter_map(|i| i.target.clone().map(|target| CanvasEdit::Remove { target })),
            );
        }
        self.nudge(ui, project, scene, modifiers, commands);
    }
    /// The Add object here submenu: object types the shown mask accepts, placed
    /// where the menu was opened.
    /// The innermost object under `p` that can take a new child: a key in its
    /// soft key slot, the Soft Key Mask for an empty slot, otherwise the
    /// innermost editable object with positioned children, or the mask.
    fn add_target(
        project: &EditorProject,
        scene: &Scene,
        softkeys: &super::softkeys::SoftKeys,
        root: u16,
        p: Pos2,
    ) -> Option<AddTarget> {
        let pool = project.get_pool();
        let at = |parent: u16, origin: Pos2| AddTarget {
            parent,
            x: (p.x - origin.x).round().max(0.0) as i16,
            y: (p.y - origin.y).round().max(0.0) as i16,
        };
        if let Some(slot) = softkeys.slots.iter().find(|slot| slot.rect.contains(p)) {
            return match slot.content {
                super::softkeys::SlotContent::Key { id, .. } => Some(at(id, slot.rect.min)),
                super::softkeys::SlotContent::Empty | super::softkeys::SlotContent::Invalid => {
                    softkeys.mask_id.map(|mask| AddTarget {
                        parent: mask,
                        x: 0,
                        y: 0,
                    })
                }
                super::softkeys::SlotContent::Previous | super::softkeys::SlotContent::Next => None,
            };
        }
        let positions_children = |id: u16| {
            ObjectId::new(id)
                .ok()
                .and_then(|id| pool.object_by_id(id))
                .and_then(crate::operations::object_refs)
                .is_some()
        };
        scene
            .items
            .iter()
            .filter(|item| {
                item.path.first() != Some(&super::softkeys::PATH_PREFIX)
                    && item.hit_rect.contains(p)
                    && (item.path.is_empty()
                        || matches!(item.target, Some(PlacementTarget::Primitive { .. })))
                    && positions_children(item.object_id)
            })
            .max_by_key(|item| item.path.len())
            .map(|item| at(item.object_id, item.rect.min))
            .or_else(|| positions_children(root).then(|| at(root, Pos2::ZERO)))
    }

    fn add_object_menu(&mut self, ui: &mut egui::Ui, project: &EditorProject) {
        let version = project.vt_version();
        let parent = self.add_target.as_ref().and_then(|target| {
            project
                .get_pool()
                .object_by_id(ObjectId::new(target.parent).ok()?)
        });
        let allowed = parent.map_or_else(Vec::new, |parent| {
            crate::allowed_object_relationships::get_allowed_child_refs(
                parent.object_type(),
                version,
            )
        });
        let response = ui.add_enabled_ui(!allowed.is_empty() && self.mask.is_some(), |ui| {
            ui.menu_button("Add object here", |ui| {
                if let Some(parent) = parent {
                    ui.weak(format!("In {}", project.object_display_name(parent)));
                }
                let focus = self.add_object_query.is_empty();
                if let Some(object_type) = crate::object_picker::object_type_list(
                    ui,
                    &mut self.add_object_query,
                    focus,
                    version,
                    |object_type| allowed.contains(&object_type),
                ) {
                    if let (Some(root), Some(target)) = (self.mask, &self.add_target) {
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                AddObjectRequest::id(),
                                AddObjectRequest {
                                    object_type,
                                    root,
                                    parent: target.parent,
                                    x: target.x,
                                    y: target.y,
                                },
                            )
                        });
                    }
                    ui.close();
                }
            });
        });
        if allowed.is_empty() {
            response
                .response
                .on_disabled_hover_text("Nothing can be added where you right-clicked.");
        }
    }
    /// Group the selected placements under a new parent, offering the types
    /// that can hold them all. A Container is the usual choice, also on Ctrl+G.
    fn group_menu(&mut self, ui: &mut egui::Ui, project: &EditorProject, items: &[SceneItem]) {
        let targets = group_targets(items);
        let response = ui.add_enabled_ui(!targets.is_empty(), |ui| {
            ui.menu_button("Group into", |ui| {
                let candidates = edits::grouping_candidates(project, &targets);
                if candidates.is_empty() {
                    ui.add_enabled(
                        false,
                        egui::Label::new("No parent type can hold these objects"),
                    );
                }
                for candidate in candidates {
                    let shortcut = if candidate.object_type
                        == ag_iso_stack::object_pool::ObjectType::Container
                    {
                        " · Ctrl+G"
                    } else {
                        ""
                    };
                    let response = ui.add_enabled(
                        candidate.enabled,
                        egui::Button::new(format!("{:?}{shortcut}", candidate.object_type)),
                    );
                    if let Some(reason) = candidate.reason {
                        response.clone().on_disabled_hover_text(reason);
                    }
                    if response.clicked() {
                        self.request_group(
                            ui.ctx(),
                            project,
                            targets.clone(),
                            candidate.object_type,
                        );
                        ui.close();
                    }
                }
            });
        });
        if targets.is_empty() {
            response.response.on_disabled_hover_text(
                "Select objects, with Shift-click for more than one, to group them.",
            );
        }
    }

    fn request_group(
        &self,
        ctx: &egui::Context,
        project: &EditorProject,
        targets: Vec<PlacementTarget>,
        object_type: ag_iso_stack::object_pool::ObjectType,
    ) {
        let Some(root) = self.mask else {
            return;
        };
        ctx.data_mut(|data| {
            data.insert_temp(
                GroupObjectsRequest::id(),
                GroupObjectsRequest {
                    session: project.ui_session_id(),
                    revision: project.revision(),
                    root,
                    targets,
                    object_type,
                },
            )
        });
    }

    /// Moves the selection by 1 pixel per arrow key, or 10 with Shift.
    fn nudge(
        &self,
        ui: &mut egui::Ui,
        project: &EditorProject,
        scene: &Scene,
        modifiers: Modifiers,
        commands: &mut Vec<CanvasEdit>,
    ) {
        let items = root_items(self.selection(scene));
        let mut nudge = Vec2::ZERO;
        for (key, delta) in [
            (Key::ArrowLeft, egui::vec2(-1.0, 0.0)),
            (Key::ArrowRight, egui::vec2(1.0, 0.0)),
            (Key::ArrowUp, egui::vec2(0.0, -1.0)),
            (Key::ArrowDown, egui::vec2(0.0, 1.0)),
        ] {
            let mods = Modifiers {
                shift: modifiers.shift,
                alt: modifiers.alt,
                ..Default::default()
            };
            if ui.input_mut(|i| i.consume_key(mods, key)) {
                nudge += delta * if modifiers.shift { 10.0 } else { 1.0 };
            }
        }
        if nudge != Vec2::ZERO {
            commands.extend(move_edits(project, &items, nudge, false));
        }
    }
}

fn shape_handle(
    project: &EditorProject,
    ctx: &egui::Context,
    item: &SceneItem,
    pointer: Pos2,
    origin: Pos2,
    zoom: f32,
) -> Option<GestureKind> {
    if !crate::shape_editing::editing(ctx, project, item.object_id) {
        return None;
    }
    let pool = project.get_pool();
    let object = ObjectId::new(item.object_id)
        .ok()
        .and_then(|id| pool.object_by_id(id))?;
    let handles = shape_handle_positions(pool, item, object)?;
    handles
        .into_iter()
        .filter_map(|(point, kind)| {
            let distance = (origin + point.to_vec2() * zoom - pointer).length();
            (distance <= 9.0).then_some((distance, kind))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, kind)| kind)
}

fn paint_shape_handles(
    painter: &egui::Painter,
    pool: &ag_iso_stack::object_pool::ObjectPool,
    item: &SceneItem,
    transform: egui::emath::TSTransform,
    color: egui::Color32,
    selected_point: Option<usize>,
) {
    let Some(object) = ObjectId::new(item.object_id)
        .ok()
        .and_then(|id| pool.object_by_id(id))
    else {
        return;
    };
    let Some(handles) = shape_handle_positions(pool, item, object) else {
        return;
    };
    for (point, kind) in handles {
        let (index, name) = match kind {
            GestureKind::PolygonVertex(index) => (Some(index), (index + 1).to_string()),
            GestureKind::LineEndpoint(index) => (None, if index == 0 { "A" } else { "B" }.into()),
            _ => continue,
        };
        let center = transform * point;
        let selected = index.is_some() && index == selected_point;
        painter.circle_filled(center, if selected { 5.5 } else { 4.5 }, color);
        painter.circle_stroke(center, 5.5, Stroke::new(1.0, egui::Color32::WHITE));
        painter.text(
            center + egui::vec2(7.0, -7.0),
            egui::Align2::LEFT_BOTTOM,
            name,
            egui::FontId::proportional(10.0),
            color,
        );
    }
}

fn shape_handle_positions(
    pool: &ag_iso_stack::object_pool::ObjectPool,
    item: &SceneItem,
    object: &Object,
) -> Option<Vec<(Pos2, GestureKind)>> {
    use ag_iso_stack::object_pool::object::Object;
    match object {
        Object::OutputPolygon(polygon) => {
            let brush = line_brush(pool, polygon.line_attributes);
            let offset = f32::from(brush) * 0.5;
            Some(
                polygon
                    .points
                    .iter()
                    .enumerate()
                    .map(|(index, point)| {
                        (
                            item.rect.min
                                + egui::vec2(
                                    f32::from(point.x) + offset,
                                    f32::from(point.y) + offset,
                                ),
                            GestureKind::PolygonVertex(index),
                        )
                    })
                    .collect(),
            )
        }
        Object::OutputLine(line) => {
            let brush = f32::from(line_brush(pool, line.line_attributes));
            let right = (item.rect.right() - brush).max(item.rect.left());
            let bottom = (item.rect.bottom() - brush).max(item.rect.top());
            let (start, end) = match line.line_direction {
                ag_iso_stack::object_pool::object_attributes::LineDirection::TopLeftToBottomRight => {
                    (item.rect.min, egui::pos2(right, bottom))
                }
                ag_iso_stack::object_pool::object_attributes::LineDirection::BottomLeftToTopRight => {
                    (egui::pos2(item.rect.left(), bottom), egui::pos2(right, item.rect.top()))
                }
            };
            let offset = egui::vec2(brush * 0.5, brush * 0.5);
            Some(vec![
                (start + offset, GestureKind::LineEndpoint(0)),
                (end + offset, GestureKind::LineEndpoint(1)),
            ])
        }
        _ => None,
    }
}

fn line_brush(pool: &ag_iso_stack::object_pool::ObjectPool, attributes: NullableObjectId) -> u8 {
    use ag_iso_stack::object_pool::object::Object;
    pool.object_by_nullable_id(attributes)
        .and_then(|object| match object {
            Object::LineAttributes(attributes) => Some(attributes.line_width),
            _ => None,
        })
        .unwrap_or(1)
}

fn line_endpoint_edit(
    project: &EditorProject,
    item: &SceneItem,
    endpoint: usize,
    pointer: Pos2,
) -> Option<CanvasEdit> {
    use ag_iso_stack::object_pool::object::Object;
    use ag_iso_stack::object_pool::object_attributes::LineDirection;
    let object_id = ObjectId::new(item.object_id).ok()?;
    let Object::OutputLine(line) = project.get_pool().object_by_id(object_id)? else {
        return None;
    };
    let target = item.target.clone()?;
    let (local_x, local_y) = position(project, &target)?;
    let brush = f32::from(line_brush(project.get_pool(), line.line_attributes));
    let handles = shape_handle_positions(
        project.get_pool(),
        item,
        project.get_pool().object_by_id(object_id)?,
    )?;
    let start = handles.first()?.0;
    let end = handles.get(1)?.0;
    let fixed = if endpoint == 0 { end } else { start };
    let half = brush * 0.5;
    let parent_origin = item.rect.min - egui::vec2(f32::from(local_x), f32::from(local_y));
    let fixed_local = fixed - parent_origin - egui::vec2(half, half);
    let moving_local = pointer - parent_origin - egui::vec2(half, half);
    let (x, width, moving_x) = line_axis_geometry(moving_local.x, fixed_local.x, brush)?;
    let (y, height, moving_y) = line_axis_geometry(moving_local.y, fixed_local.y, brush)?;
    let direction = if (moving_x - fixed_local.x).signum() == (moving_y - fixed_local.y).signum() {
        LineDirection::TopLeftToBottomRight
    } else {
        LineDirection::BottomLeftToTopRight
    };
    Some(CanvasEdit::EditLineEndpoint {
        object_id: item.object_id,
        target,
        x,
        y,
        width,
        height,
        direction,
    })
}

/// Clamp a dragged line endpoint to representable placement and dimension
/// limits while leaving the opposite endpoint at its original coordinate.
fn line_axis_geometry(moving: f32, fixed: f32, brush: f32) -> Option<(i16, u16, f32)> {
    let fixed = fixed.round();
    let max_distance = f32::from(u16::MAX) - brush;
    let min_placement = f32::from(i16::MIN);
    let max_placement = f32::from(i16::MAX);
    let (min_moving, max_moving) = if fixed < min_placement {
        return None;
    } else if fixed > max_placement {
        (fixed - max_distance, max_placement)
    } else {
        (
            (fixed - max_distance).max(min_placement),
            fixed + max_distance,
        )
    };
    if min_moving > max_moving {
        return None;
    }
    let moving = moving.round().clamp(min_moving, max_moving);
    let top_left = fixed.min(moving);
    if !(min_placement..=max_placement).contains(&top_left) {
        return None;
    }
    let distance = (fixed - moving).abs();
    let dimension = (distance + brush).round().clamp(1.0, f32::from(u16::MAX)) as u16;
    Some((top_left as i16, dimension, moving))
}

fn scene_item_name(project: &EditorProject, item: &SceneItem) -> String {
    let selection = project.format_object_id(item.selection_id);
    if item.selection_id == item.object_id {
        selection
    } else {
        format!("{selection} → {}", project.format_object_id(item.object_id))
    }
}

fn hierarchy_scene_path(
    project: &EditorProject,
    materialized: &MaterializedProject,
    root: u16,
    path: &[usize],
) -> Option<Vec<usize>> {
    let (&branch, children) = path.split_first()?;
    let (mut object, mut scene_path) = if branch == 0 {
        (root, Vec::new())
    } else {
        (
            softkey_object_id(project, root, branch - 1)?,
            vec![super::softkeys::PATH_PREFIX, branch - 1],
        )
    };
    for &index in children {
        if index == usize::MAX {
            let ag_iso_stack::object_pool::object::Object::ObjectPointer(pointer) = materialized
                .pool
                .object_by_id(ObjectId::new(object).ok()?)?
            else {
                return None;
            };
            object = pointer.value.0?.value();
        } else {
            let index = canonical_to_materialized_index(materialized, object, index)?;
            let parent = materialized
                .pool
                .object_by_id(ObjectId::new(object).ok()?)?;
            object = crate::operations::object_refs(parent)?
                .get(index)?
                .id
                .value();
            scene_path.push(index);
        }
    }
    Some(scene_path)
}

fn row_scene_item<'a>(
    project: &EditorProject,
    materialized: &MaterializedProject,
    scene: &'a Scene,
    row: &HierarchyOccurrence,
) -> Option<&'a SceneItem> {
    if let Some(scene_path) = hierarchy_scene_path(project, materialized, row.root, &row.path) {
        if let Some(item) = scene.items.iter().find(|item| {
            item.path == scene_path
                && row
                    .target
                    .as_ref()
                    .is_none_or(|target| item.target.as_ref() == Some(target))
        }) {
            return Some(item);
        }
    }

    let target = row.target.as_ref()?;
    let parent_path = row.parent_path.as_ref()?;
    let parent_scene_path = hierarchy_scene_path(project, materialized, row.root, parent_path)?;
    scene.items.iter().find(|item| {
        item.target.as_ref() == Some(target)
            && item.path.len() == parent_scene_path.len() + 1
            && item.path.starts_with(&parent_scene_path)
    })
}

fn canonical_to_materialized_index(
    materialized: &MaterializedProject,
    parent_id: u16,
    canonical_index: usize,
) -> Option<usize> {
    let parent_id = ObjectId::new(parent_id).ok()?;
    let parent = materialized.pool.object_by_id(parent_id)?;
    let mut canonical_index_at = 0;
    for (materialized_index, reference) in
        crate::operations::object_refs(parent)?.iter().enumerate()
    {
        if matches!(
            materialized
                .source_map
                .by_object_id
                .get(&reference.id.value()),
            Some(crate::components::MaterializedSource::ComponentNode { .. })
        ) {
            continue;
        }
        if canonical_index_at == canonical_index {
            return Some(materialized_index);
        }
        canonical_index_at += 1;
    }
    None
}

fn softkey_object_id(project: &EditorProject, root: u16, slot: usize) -> Option<u16> {
    use ag_iso_stack::object_pool::object::Object;
    let root = project.get_pool().object_by_id(ObjectId::new(root).ok()?)?;
    let softkey_mask_id = match root {
        Object::DataMask(mask) => mask.soft_key_mask.0?.value(),
        Object::AlarmMask(mask) => mask.soft_key_mask.0?.value(),
        _ => return None,
    };
    let softkey_mask = project
        .get_pool()
        .object_by_id(ObjectId::new(softkey_mask_id).ok()?)?;
    let Object::SoftKeyMask(mask) = softkey_mask else {
        return None;
    };
    mask.objects.get(slot).map(|id| id.value())
}

/// The placements of the outermost selected items, which grouping moves.
fn group_targets(items: &[SceneItem]) -> Vec<PlacementTarget> {
    root_items(items.to_vec())
        .into_iter()
        .filter_map(|item| item.target)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{
        object::Object, object_attributes::Point, ObjectPool, ObjectRef, ObjectType,
    };

    fn repeated_parent_project() -> EditorProject {
        let mut project = EditorProject::from(repeated_parent_pool());
        project.mask_size = 500;
        project
    }

    fn repeated_parent_pool() -> ObjectPool {
        let mut pool = ObjectPool::default();
        for (id, kind) in [
            (2, ObjectType::DataMask),
            (100, ObjectType::Container),
            (101, ObjectType::OutputString),
            (104, ObjectType::ObjectPointer),
        ] {
            let mut object = crate::default_object(kind);
            object.mut_id().set_value(id).unwrap();
            pool.add(object);
        }
        if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
            for (id, x) in [(100, 0), (100, 100), (104, 200)] {
                mask.object_refs.push(ObjectRef {
                    id: ObjectId::new(id).unwrap(),
                    offset: Point { x, y: 0 },
                });
            }
        }
        if let Some(Object::Container(container)) =
            pool.object_mut_by_id(ObjectId::new(100).unwrap())
        {
            container.width = 80;
            container.height = 40;
            container.object_refs.push(ObjectRef {
                id: ObjectId::new(101).unwrap(),
                offset: Point { x: 4, y: 4 },
            });
        }
        if let Some(Object::ObjectPointer(pointer)) =
            pool.object_mut_by_id(ObjectId::new(104).unwrap())
        {
            pointer.value = ObjectId::new(100).unwrap().into();
        }
        pool
    }

    #[test]
    fn add_object_here_offers_the_innermost_object_that_takes_children() {
        let mut pool = repeated_parent_pool();
        for (id, kind) in [(3, ObjectType::SoftKeyMask), (4, ObjectType::Key)] {
            let mut object = crate::default_object(kind);
            object.mut_id().set_value(id).unwrap();
            pool.add(object);
        }
        if let Some(Object::SoftKeyMask(mask)) = pool.object_mut_by_id(ObjectId::new(3).unwrap()) {
            mask.objects.push(ObjectId::new(4).unwrap());
        }
        if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
            mask.soft_key_mask = ObjectId::new(3).unwrap().into();
        }
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        let materialized = project.materialize_shared().unwrap();
        let scene = geometry::build_page(&project, &materialized, 2, 0);
        let softkeys = super::super::softkeys::build(&project, project.get_pool(), 2, 0);
        let target = |x: f32, y: f32| {
            CanvasState::add_target(&project, &scene, &softkeys, 2, Pos2::new(x, y))
                .map(|target| (target.parent, target.x, target.y))
        };

        // Inside each occurrence of the container, also over its Output String
        assert_eq!(target(10.0, 10.0), Some((100, 10, 10)));
        assert_eq!(target(110.0, 5.0), Some((100, 10, 5)));
        assert_eq!(target(6.0, 6.0), Some((100, 6, 6)));
        assert_eq!(target(300.0, 300.0), Some((2, 300, 300)));

        let slot = |key: bool| {
            softkeys
                .slots
                .iter()
                .find(|slot| {
                    matches!(
                        slot.content,
                        super::super::softkeys::SlotContent::Key { .. }
                    ) == key
                        && !matches!(
                            slot.content,
                            super::super::softkeys::SlotContent::Previous
                                | super::super::softkeys::SlotContent::Next
                        )
                })
                .unwrap()
                .rect
        };
        let key = slot(true);
        let in_key = key.min + egui::vec2(3.0, 2.0);
        assert_eq!(target(in_key.x, in_key.y), Some((4, 3, 2)));
        let empty = slot(false).center();
        assert_eq!(target(empty.x, empty.y), Some((3, 0, 0)));
    }

    #[test]
    fn pointer_target_change_keeps_placement_selection_and_uses_new_bounds() {
        let mut project = repeated_parent_project();
        let materialized = project.materialize_shared().unwrap();
        let before = geometry::build_page(&project, &materialized, 2, 0);
        let mut canvas = CanvasState::default();
        canvas.select_hierarchy_object(&project, 2, &[0, 2], 104);
        project.update_selected();

        let mut after = before.clone();
        let pointer = after
            .items
            .iter_mut()
            .find(|item| item.path == [2])
            .unwrap();
        pointer.object_id = 101;
        pointer.rect = Rect::from_min_size(pointer.rect.min, egui::vec2(23.0, 12.0));
        pointer.hit_rect = pointer.rect.intersect(pointer.parent_rect);

        let selected = canvas.reconciled_selection(&after);
        assert_eq!(selected, vec![vec![2]]);
        let selected_item = after
            .items
            .iter()
            .find(|item| item.path == selected[0])
            .unwrap();
        assert_eq!(selected_item.selection_id, 104);
        assert_eq!(selected_item.object_id, 101);
        assert_eq!(selected_item.rect.size(), egui::vec2(23.0, 12.0));
        assert_eq!(
            canvas.selected_placement(&project),
            selected_item.target.clone()
        );
    }

    #[test]
    fn hierarchy_selection_supports_toggle_range_context_and_exact_siblings() {
        let project = repeated_parent_project();
        let mut canvas = CanvasState::default();
        canvas.begin_hierarchy_frame(project.revision());
        for (index, object) in [(0, 100), (1, 100), (2, 104)] {
            let path = vec![0, index];
            canvas.register_hierarchy_occurrence(HierarchyOccurrence {
                root: 2,
                parent_path: Some(vec![0]),
                target: Some(PlacementTarget::Primitive {
                    parent_id: 2,
                    child_index: index,
                }),
                path,
                object,
            });
        }

        assert!(canvas.hierarchy_click(&project, 2, &[0, 0], Modifiers::default(), false));
        assert_eq!(canvas.selected_hierarchy_occurrences().len(), 1);
        assert!(canvas.hierarchy_occurrence_selected(2, &[0, 0]));
        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 0], 100));
        assert!(canvas.hierarchy_click(
            &project,
            2,
            &[0, 1],
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            false,
        ));
        let siblings = canvas.selected_hierarchy_siblings().unwrap();
        assert_eq!(
            siblings
                .iter()
                .map(|row| row.path.clone())
                .collect::<Vec<_>>(),
            vec![vec![0, 0], vec![0, 1]]
        );
        assert!(canvas.hierarchy_occurrence_selected(2, &[0, 0]));

        // A context click on either selected row keeps the whole selection.
        assert!(canvas.hierarchy_click(&project, 2, &[0, 0], Modifiers::default(), true));
        assert_eq!(canvas.selected_hierarchy_occurrences().len(), 2);

        // A plain selection establishes the Shift range anchor.
        assert!(canvas.hierarchy_click(&project, 2, &[0, 0], Modifiers::default(), false,));
        assert!(canvas.hierarchy_click(
            &project,
            2,
            &[0, 2],
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            false,
        ));
        assert_eq!(canvas.selected_hierarchy_occurrences().len(), 3);
        assert!(canvas.selected_hierarchy_siblings().is_some());

        // Right-clicking outside the selection selects only that occurrence.
        assert!(canvas.hierarchy_click(&project, 2, &[0, 0], Modifiers::default(), false));
        assert!(canvas.hierarchy_click(&project, 2, &[0, 1], Modifiers::default(), true));
        assert_eq!(canvas.selected_hierarchy_occurrences().len(), 1);
        assert_eq!(canvas.selected_hierarchy_occurrences()[0].path, vec![0, 1]);
    }

    #[test]
    fn hierarchy_toggle_restores_visible_order_and_synthetic_rows_resolve_by_target() {
        let project = repeated_parent_project();
        let synthetic_path = vec![0, usize::MAX - 1, 0];
        let mut canvas = CanvasState::default();
        canvas.begin_hierarchy_frame(project.revision());
        for (path, index, object) in [
            (vec![0, 0], 0, 100),
            (vec![0, 1], 1, 100),
            (synthetic_path.clone(), 2, 104),
        ] {
            canvas.register_hierarchy_occurrence(HierarchyOccurrence {
                root: 2,
                path,
                object,
                target: Some(PlacementTarget::Primitive {
                    parent_id: 2,
                    child_index: index,
                }),
                parent_path: Some(vec![0]),
            });
        }

        assert!(canvas.hierarchy_click(&project, 2, &[0, 0], Modifiers::default(), false));
        assert!(canvas.hierarchy_click(
            &project,
            2,
            &synthetic_path,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            false,
        ));
        assert!(canvas.hierarchy_click(
            &project,
            2,
            &[0, 1],
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            false,
        ));
        assert_eq!(
            canvas
                .selected_hierarchy_occurrences()
                .iter()
                .map(|row| row.path.clone())
                .collect::<Vec<_>>(),
            vec![vec![0, 0], vec![0, 1], synthetic_path],
        );
        assert_eq!(canvas.selected, vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn toggling_off_the_last_hierarchy_row_clears_inspector_and_highlight() {
        let mut project = repeated_parent_project();
        let mut canvas = CanvasState::default();
        canvas.begin_hierarchy_frame(project.revision());
        canvas.register_hierarchy_occurrence(HierarchyOccurrence {
            root: 2,
            path: vec![0, 0],
            object: 100,
            target: Some(PlacementTarget::Primitive {
                parent_id: 2,
                child_index: 0,
            }),
            parent_path: Some(vec![0]),
        });

        assert!(canvas.hierarchy_click(&project, 2, &[0, 0], Modifiers::default(), false));
        project.update_selected();
        assert_eq!(project.get_selected(), NullableObjectId::new(100));
        assert!(canvas.hierarchy_click(
            &project,
            2,
            &[0, 0],
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            false,
        ));
        project.update_selected();

        assert!(canvas.selected_hierarchy_occurrences().is_empty());
        assert!(canvas.selected.is_empty());
        assert_eq!(canvas.inspector, None);
        assert_eq!(canvas.placement_owner, None);
        assert_eq!(project.get_selected(), NullableObjectId::default());
        assert!(!canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 0], 100));
    }

    #[test]
    fn hierarchy_selection_keeps_repeated_parent_and_pointer_alias_occurrences() {
        let mut project = repeated_parent_project();
        let target = PlacementTarget::Primitive {
            parent_id: 100,
            child_index: 0,
        };
        let mut canvas = CanvasState::default();

        assert!(canvas.select_hierarchy_placement(&project, 2, &[0, 1], target.clone()));
        assert_eq!(canvas.selected, vec![vec![1, 0]]);
        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 1, 0], 101));
        assert!(!canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 0, 0], 101));

        assert!(canvas.select_hierarchy_placement(&project, 2, &[0, 2, usize::MAX], target,));
        assert_eq!(canvas.selected, vec![vec![2, 0]]);
        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 2, usize::MAX, 0], 101));
        assert!(!canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 1, 0], 101));

        canvas.select_hierarchy_object(&project, 2, &[0, 1, 0], 101);
        canvas.select_all_hierarchy_references(Some(101));
        assert!(canvas.selected_hierarchy_occurrences().is_empty());

        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 0, 0], 101));
        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 1, 0], 101));
        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 2, usize::MAX, 0], 101));

        canvas.select_hierarchy_object(&project, 2, &[0, 1, 0], 101);
        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 1, 0], 101));
        assert!(!canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 0, 0], 101));

        canvas.select_hierarchy_object(&project, 2, &[0, 2], 104);
        project.update_selected();
        assert_eq!(project.get_selected(), NullableObjectId::new(104));
        assert!(canvas.has_placement_selection(&project));
        assert_eq!(canvas.selected, vec![vec![2]]);
        assert_eq!(
            canvas.selected_placement(&project),
            Some(PlacementTarget::Primitive {
                parent_id: 2,
                child_index: 2,
            })
        );

        canvas.select_hierarchy_object(&project, 2, &[0, 2, usize::MAX], 100);
        project.update_selected();
        assert_eq!(project.get_selected(), NullableObjectId::new(100));
        assert!(canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 2, usize::MAX], 100));
        assert!(!canvas.hierarchy_occurrence_is_primary(&project, 2, &[0, 0], 100));
    }

    #[test]
    fn line_endpoint_geometry_keeps_fixed_endpoint_through_crossing_and_bounds() {
        let (origin, size, moving) = line_axis_geometry(10.0, 100.0, 3.0).unwrap();
        assert_eq!(origin, 10);
        assert_eq!(size, 93);
        assert_eq!(origin as i32 + size as i32 - 3, 100);
        assert_eq!(moving, 10.0);

        let (origin, size, moving) = line_axis_geometry(120.0, 100.0, 3.0).unwrap();
        assert_eq!(origin, 100);
        assert_eq!(size, 23);
        assert_eq!(moving, 120.0);

        let (origin, size, _) = line_axis_geometry(-40_000.0, 40.0, 1.0).unwrap();
        assert_eq!(origin, i16::MIN);
        assert_eq!(origin as i32 + size as i32 - 1, 40);

        let (origin, size, _) = line_axis_geometry(50_000.0, 40_000.0, 1.0).unwrap();
        assert_eq!(origin, i16::MAX);
        assert_eq!(origin as i32 + size as i32 - 1, 40_000);
    }
}
