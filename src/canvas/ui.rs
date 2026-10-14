use super::layout::*;
use super::{
    edits::{self, position, CanvasEdit, PlacementTarget},
    geometry::{self, Scene, SceneItem},
};
use crate::{components::MaterializedProject, EditorProject};
use ag_iso_stack::object_pool::{object_attributes::Point, NullableObjectId, ObjectId};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Stroke, Vec2};

/// Transient viewport, occurrence selection, and gesture drafts for one project.
/// Reset this state when replacing the project; no canvas state is serialized.
pub struct CanvasState {
    mask: Option<u16>,
    revision: u64,
    selected: Vec<Vec<usize>>,
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
    fit: Option<bool>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            mask: None,
            revision: 0,
            selected: vec![],
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
            fit: None,
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
        if !ui.input(|i| i.focused) {
            self.gesture = None;
            self.field_draft = None;
        }
        if self.mask != Some(root) {
            *self = Self::default();
            self.mask = Some(root);
        }
        if self.revision != project.revision() {
            self.gesture = None;
            self.selected.clear();
            self.context.clear();
            self.inspector = None;
            self.field_draft = None;
            self.revision = project.revision();
        }
        let scene = geometry::build(project, materialized, root);
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
                .find(|i| i.target.is_some() && (Some(i.object_id) == inspector || inspector.and_then(|id| materialized.source_map.by_object_id.get(&id)).is_some_and(|source| matches!((source, &i.target), (crate::components::MaterializedSource::ComponentNode { instance_id, .. }, Some(PlacementTarget::Component { instance_id: target })) if instance_id == target))))
                .map(|i| vec![i.path.clone()])
                .unwrap_or_default();
        }
        let items = self.selection(&scene);
        let mut commands = vec![];
        self.toolbar(ui, project, &scene, &items, &mut commands);
        if !commands.is_empty() {
            self.commit(project, "Edit layout", std::mem::take(&mut commands));
            ui.ctx().request_repaint();
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
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
        let canvas_id = response.id;
        let visuals = ui.visuals();
        let selection_color = visuals.selection.stroke.color;
        let guide_color = visuals.warn_fg_color;
        let mask_fill = visuals.extreme_bg_color;
        let workspace_fill = visuals.faint_bg_color;
        let mask_rect = scene
            .items
            .first()
            .map(|i| i.rect)
            .unwrap_or(Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(project.mask_size as f32, project.mask_size as f32),
            ));
        if let Some(selection) = self.fit.take() {
            let fit_rect = if selection {
                bounds(&items).unwrap_or(mask_rect)
            } else {
                mask_rect
            };
            self.zoom = ((viewport.width() - 48.0) / fit_rect.width().max(1.0))
                .min((viewport.height() - 48.0) / fit_rect.height().max(1.0))
                .clamp(0.1, 8.0);
            self.pan = viewport.size() * 0.5 - fit_rect.center().to_vec2() * self.zoom;
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
        let origin = viewport.min + self.pan;
        let zoom = self.zoom;
        let to_pool = |p: Pos2| Pos2::ZERO + (p - origin) / zoom;
        let hover = pointer
            .hover_pos()
            .filter(|p| viewport.contains(*p))
            .map(to_pool);
        let hit = hover.and_then(|p| self.hit(&scene, p, modifiers.ctrl || modifiers.command));
        let can_key = response.has_focus() || self.gesture.is_some();
        if can_key {
            self.keys(ui, project, &scene, modifiers, &mut commands);
        }
        if pointer.button_pressed(egui::PointerButton::Primary) && response.hovered() {
            response.request_focus();
            if let Some(p) = pointer.interact_pos() {
                let pool_pos = to_pool(p);
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
                    GestureKind::Move
                } else {
                    if !modifiers.shift {
                        self.selected.clear();
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
        if pointer.button_pressed(egui::PointerButton::Middle) && response.hovered() {
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
            || response.dragged_by(egui::PointerButton::Middle)
            || response.drag_stopped_by(egui::PointerButton::Primary)
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
                    || response.drag_stopped_by(egui::PointerButton::Middle)
                    || response.hovered()
                        && (pointer.button_released(egui::PointerButton::Primary)
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
        let transform = egui::emath::TSTransform::new(viewport.min.to_vec2() + self.pan, self.zoom);
        let workspace_painter = ui.painter().with_clip_rect(viewport);
        workspace_painter.rect_filled(viewport, 0.0, workspace_fill);
        let mask_screen_rect =
            egui::emath::TSTransform::new(viewport.min.to_vec2() + self.pan, self.zoom) * mask_rect;
        workspace_painter.rect_filled(mask_screen_rect.intersect(viewport), 1.0, mask_fill);
        workspace_painter.rect_stroke(
            mask_screen_rect,
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(canvas_id.with("objects"))
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
        let painter = ui.painter().with_clip_rect(viewport);
        let screen = |r: Rect| transform * r;
        let draft_scene = self
            .gesture
            .as_ref()
            .and_then(|g| g.draft.as_ref())
            .map(|(m, _)| geometry::build(project, m, root));
        let display_scene = draft_scene.as_ref().unwrap_or(&scene);
        let display_items =
            if let Some((_, targets)) = self.gesture.as_ref().and_then(|g| g.draft.as_ref()) {
                display_scene
                    .items
                    .iter()
                    .filter(|i| i.target.as_ref().is_some_and(|t| targets.contains(t)))
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
                            self.context = item.path[..item.path.len().saturating_sub(1)].to_vec();
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
            if ui.button("Fit mask · Shift+1").clicked() {
                self.fit = Some(false);
                ui.close();
            }
            if ui.button("Fit selection · Shift+2").clicked() {
                self.fit = Some(true);
                ui.close();
            }
        });
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
                    viewport.min + self.pan,
                    self.zoom,
                    dimensions(project, &items[0]),
                ) {
                    ui.ctx().set_cursor_icon(handle_cursor(h));
                }
            }
        }
    }

    fn selection(&self, scene: &Scene) -> Vec<SceneItem> {
        scene
            .items
            .iter()
            .filter(|i| self.selected.contains(&i.path))
            .cloned()
            .collect()
    }
    fn hit<'a>(&self, scene: &'a Scene, pos: Pos2, deep: bool) -> Option<&'a SceneItem> {
        let leaf = scene.items.iter().rev().find(|i| {
            i.target.is_some()
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
        let depth = if leaf.path.starts_with(&self.context) {
            self.context.len() + 1
        } else {
            1
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
                    let scene = geometry::build(project, &m, root);
                    self.selected = scene
                        .items
                        .iter()
                        .filter(|i| i.target.as_ref().is_some_and(|t| targets.contains(t)))
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
            if ui.small_button("Fit mask").clicked() {
                self.fit = Some(false);
            }
            if ui
                .add_enabled(!items.is_empty(), egui::Button::new("Fit selection"))
                .clicked()
            {
                self.fit = Some(true);
            }
            ui.separator();
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
                ui.label("Space-drag / middle mouse: pan · Ctrl-wheel: zoom");
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
                        });
                    }
                    let draft = self.field_draft.as_mut().unwrap();
                    let mut finish = false;
                    if let Some((x, y)) = &mut draft.position {
                        for (name, value) in [("X", x), ("Y", y)] {
                            ui.label(name);
                            let r = ui.add(egui::DragValue::new(value).speed(1.0));
                            draft.changed |= r.changed();
                            finish |= r.drag_stopped() || r.lost_focus();
                        }
                    }
                    for (name, value) in [("W", &mut draft.width), ("H", &mut draft.height)] {
                        if let Some(value) = value {
                            ui.label(name);
                            let r = ui.add(egui::DragValue::new(value).range(0..=u16::MAX));
                            draft.changed |= r.changed();
                            finish |= r.drag_stopped() || r.lost_focus();
                        }
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        draft.position = current_position;
                        draft.width = w;
                        draft.height = h;
                        draft.changed = false;
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
                                Color32::YELLOW,
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

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
