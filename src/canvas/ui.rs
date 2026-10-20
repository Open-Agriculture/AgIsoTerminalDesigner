use super::layout::*;
use super::MaskViewState;
use super::{
    edits::{self, position, CanvasEdit, PlacementTarget},
    geometry::{self, Scene, SceneItem},
};
use crate::object_rendering::Colorable;
use crate::{components::MaterializedProject, EditorProject};
use ag_iso_stack::object_pool::{object_attributes::Point, NullableObjectId, ObjectId};
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
    scene_identity: Vec<(Vec<usize>, u16, Option<PlacementTarget>)>,
    scene_origins: HashMap<Vec<usize>, Pos2>,
    placement_owner: Option<u16>,
    context: Vec<usize>,
    inspector: Option<u16>,
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
    fit: Option<bool>,
    fit_all: bool,
    initially_fitted: bool,
    pages: HashMap<u16, (Option<u16>, usize)>,
    preferred_slot: Option<(u16, usize, Option<u16>)>,
    pair_layout: Vec<(u16, Vec2, Rect)>,
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
            fit: None,
            fit_all: false,
            initially_fitted: false,
            pages: HashMap::new(),
            preferred_slot: None,
            pair_layout: Vec::new(),
        }
    }
}

#[derive(Clone)]
enum GestureKind {
    Move,
    Resize(Vec2),
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
        if project.get_selected().0.map(|id| id.value()) != Some(item.object_id) {
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

    fn scene_identity(scene: &Scene) -> Vec<(Vec<usize>, u16, Option<PlacementTarget>)> {
        scene
            .items
            .iter()
            .map(|item| (item.path.clone(), item.object_id, item.target.clone()))
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
                let (_, object, target) =
                    self.scene_identity.iter().find(|(old, _, _)| old == path)?;
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
                    item.object_id == *object
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
            slot.index == slot_index && matches!(slot.content, super::softkeys::SlotContent::Key(_))
        }) {
            keys = super::softkeys::build(project, project.get_pool(), root, slot_index / capacity);
        }
        let slot = keys
            .slots
            .iter()
            .find(|slot| {
                slot.index == slot_index
                    && matches!(slot.content, super::softkeys::SlotContent::Key(_))
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

        let (mut object_id, mut scene_path, remaining) = match parent_path.first() {
            Some(0) => (root, Vec::new(), &parent_path[1..]),
            Some(tree_slot) => {
                let slot = *tree_slot - 1;
                let path = vec![super::softkeys::PATH_PREFIX, slot];
                let Some(object_id) = softkey_object_id(project, root, slot) else {
                    return false;
                };
                if !scene.items.iter().any(|item| item.path == path) {
                    return false;
                }
                (object_id, path, &parent_path[1..])
            }
            None => return false,
        };

        for &index in remaining {
            if index == usize::MAX {
                let Ok(pointer_id) = ag_iso_stack::object_pool::ObjectId::new(object_id) else {
                    return false;
                };
                let Some(ag_iso_stack::object_pool::object::Object::ObjectPointer(pointer)) =
                    project.get_pool().object_by_id(pointer_id)
                else {
                    return false;
                };
                let Some(pointer_target) = pointer.value.0 else {
                    return false;
                };
                object_id = pointer_target.value();
                continue;
            }

            let Some(materialized_index) =
                canonical_to_materialized_index(&materialized, object_id, index)
            else {
                return false;
            };
            scene_path.push(materialized_index);
            if !scene.items.iter().any(|item| item.path == scene_path) {
                return false;
            }
            let Ok(parent_object_id) = ObjectId::new(object_id) else {
                return false;
            };
            let Some(parent) = materialized.pool.object_by_id(parent_object_id) else {
                return false;
            };
            let Some(reference) = crate::operations::object_refs(parent)
                .and_then(|refs| refs.get(materialized_index))
            else {
                return false;
            };
            object_id = reference.id.value();
        }

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
                    super::softkeys::SlotContent::Key(key) => {
                        super::mask_view::subtree_contains(&materialized.pool, key, object)
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
            self.selected = scene
                .items
                .iter()
                .filter(|i| self.preferred_slot.is_none_or(|(owner, slot, selected)| {
                    owner != root || selected != inspector ||
                        i.path.starts_with(&[super::softkeys::PATH_PREFIX, slot])
                }))
                .find(|i| (i.target.is_some() || i.path.first() == Some(&super::softkeys::PATH_PREFIX)) && (Some(i.object_id) == inspector || inspector.and_then(|id| materialized.source_map.by_object_id.get(&id)).is_some_and(|source| matches!((source, &i.target), (crate::components::MaterializedSource::ComponentNode { instance_id, .. }, Some(PlacementTarget::Component { instance_id: target })) if instance_id == target))))
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
                .map(|item| item.object_id);
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
                let kind = if space {
                    GestureKind::Pan
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
                ui.menu_button("Select layer", |ui| {
                    if let Some(pos) = hover {
                        for item in scene.items.iter().rev().filter(|i| {
                            i.target.is_some()
                                && i.hit_rect.width() >= 0.0
                                && i.hit_rect.height() >= 0.0
                                && i.hit_rect.expand(hit_tolerance).contains(pos)
                        }) {
                            if ui
                                .button(format!(
                                    "{} · depth {}",
                                    project.format_object_id(item.object_id),
                                    item.path.len()
                                ))
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
                crate::object_rendering::render_object_with_version(
                    child,
                    &display.pool,
                    object,
                    Point::default(),
                    project.vt_version,
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
                super::softkeys::SlotContent::Key(id) => {
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
                            crate::object_rendering::render_object_with_version(
                                key_ui,
                                &display.pool,
                                object,
                                Point::default(),
                                project.vt_version,
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
        let id = self
            .selected
            .last()
            .and_then(|p| scene.items.iter().find(|i| &i.path == p))
            .map(|i| i.object_id);
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
            .map(|item| item.object_id);
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
                    .map(|i| project.format_object_id(i.object_id))
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
            if self.gesture.take().is_none() {
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
            if take(key, mods) {
                nudge += delta * if modifiers.shift { 10.0 } else { 1.0 };
            }
        }
        if nudge != Vec2::ZERO {
            commands.extend(move_edits(project, &items, nudge, false));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{
        object::Object, object_attributes::Point, ObjectPool, ObjectRef, ObjectType,
    };

    fn repeated_parent_project() -> EditorProject {
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
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        project
    }

    #[test]
    fn hierarchy_selection_keeps_repeated_parent_and_pointer_alias_occurrences() {
        let project = repeated_parent_project();
        let target = PlacementTarget::Primitive {
            parent_id: 100,
            child_index: 0,
        };
        let mut canvas = CanvasState::default();

        assert!(canvas.select_hierarchy_placement(&project, 2, &[0, 1], target.clone()));
        assert_eq!(canvas.selected, vec![vec![1, 0]]);

        assert!(canvas.select_hierarchy_placement(&project, 2, &[0, 2, usize::MAX], target,));
        assert_eq!(canvas.selected, vec![vec![2, 0]]);
    }
}
