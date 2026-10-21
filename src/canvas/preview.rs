//! Local playback UI. All events and macros operate on an isolated pool clone.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use egui::{Pos2, Rect};

use super::softkeys::{self, SlotContent};
use crate::{preview_session::PreviewSession, EditorProject};

pub(super) struct PreviewMode {
    session: Arc<Mutex<PreviewSession>>,
    revision: u64,
    initial_mask: Option<u16>,
    page: usize,
    macro_id: Option<u16>,
    initialized: bool,
    mask_size: u16,
    soft_key_size: (u16, u16),
    soft_key_layout: crate::SoftKeyLayout,
    version: ag_iso_stack::object_pool::vt_version::VtVersion,
}

impl PreviewMode {
    pub(super) fn new(project: &EditorProject, pool: &ObjectPool, focused: Option<u16>) -> Self {
        Self {
            session: Arc::new(Mutex::new(PreviewSession::with_version(
                pool,
                project.vt_version(),
            ))),
            revision: project.revision(),
            initial_mask: focused,
            page: 0,
            macro_id: None,
            initialized: false,
            mask_size: project.mask_size,
            soft_key_size: project.soft_key_size(),
            soft_key_layout: project.soft_key_layout(),
            version: project.vt_version(),
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        project: &EditorProject,
        authored: &ObjectPool,
    ) {
        let now = Duration::from_secs_f64(ui.input(|i| i.time).max(0.0));
        let mut reset = false;
        ui.horizontal(|ui| {
            reset = ui
                .button("Reset")
                .on_hover_text("Restart playback from the current design")
                .clicked();
            ui.weak("Local playback");
            if self.revision != project.revision() {
                ui.label("Design changed. Reset to load it.");
            }
        });
        if reset {
            *self = Self::new(project, authored, self.initial_mask);
        }
        {
            let Ok(mut session) = self.session.lock() else {
                return;
            };
            session.begin_frame(now);
            if !self.initialized {
                let root = self
                    .initial_mask
                    .filter(|id| {
                        ObjectId::new(*id)
                            .ok()
                            .and_then(|id| session.pool().object_by_id(id))
                            .is_some()
                    })
                    .or_else(|| {
                        session
                            .pool()
                            .working_set_object()
                            .map(|ws| ws.active_mask.value())
                    })
                    .or_else(|| {
                        session.pool().objects().iter().find_map(|object| {
                            matches!(object, Object::DataMask(_) | Object::AlarmMask(_))
                                .then_some(object.id().value())
                        })
                    });
                if let Some(root) = root {
                    let _ = session.activate_mask(root);
                }
                self.initialized = true;
            }
        }
        let pool = match self.session.lock() {
            Ok(session) => session.pool().clone(),
            Err(_) => return,
        };
        let mut root = self
            .session
            .lock()
            .ok()
            .and_then(|session| session.active_mask());
        let previous = root;
        let mut run_macro = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("playback_mask")
                .selected_text(
                    root.and_then(|id| ObjectId::new(id).ok())
                        .and_then(|id| pool.object_by_id(id))
                        .map(|object| project.object_display_name(object))
                        .unwrap_or_else(|| "Select mask".into()),
                )
                .show_ui(ui, |ui| {
                    for object in pool.objects().iter().filter(|object| {
                        matches!(object, Object::DataMask(_) | Object::AlarmMask(_))
                    }) {
                        ui.selectable_value(
                            &mut root,
                            Some(object.id().value()),
                            project.object_display_name(object),
                        );
                    }
                });
            let macros: Vec<_> = pool
                .objects()
                .iter()
                .filter(|object| matches!(object, Object::Macro(_)))
                .collect();
            if !macros.is_empty() {
                if !macros
                    .iter()
                    .any(|object| Some(object.id().value()) == self.macro_id)
                {
                    self.macro_id = Some(macros[0].id().value());
                }
                egui::ComboBox::from_id_salt("playback_macro")
                    .selected_text(
                        self.macro_id
                            .and_then(|id| ObjectId::new(id).ok())
                            .and_then(|id| pool.object_by_id(id))
                            .map(|object| project.object_display_name(object))
                            .unwrap_or_default(),
                    )
                    .show_ui(ui, |ui| {
                        for object in macros {
                            ui.selectable_value(
                                &mut self.macro_id,
                                Some(object.id().value()),
                                project.object_display_name(object),
                            );
                        }
                    });
                run_macro = ui.button("Run macro").clicked();
            }
        });
        if previous != root {
            if let (Some(root), Ok(mut session)) = (root, self.session.lock()) {
                let _ = session.activate_mask(root);
            }
            self.page = 0;
        }
        if run_macro {
            if let (Some(id), Ok(mut session)) = (self.macro_id, self.session.lock()) {
                let _ = session.execute_macro(id);
            }
        }
        let pool = match self.session.lock() {
            Ok(session) => session.pool().clone(),
            Err(_) => return,
        };
        root = self
            .session
            .lock()
            .ok()
            .and_then(|session| session.active_mask());
        let diagnostic_count = if let Ok(session) = self.session.lock() {
            if !session.diagnostics().is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt("playback_diagnostics")
                    .max_height(100.0)
                    .show(ui, |ui| {
                        for diagnostic in session.diagnostics() {
                            crate::ui_style::error_text(
                                ui,
                                &format!(
                                    "{}: {}",
                                    project.format_object_id(diagnostic.object_id),
                                    diagnostic.message
                                ),
                            );
                        }
                    });
            }
            session.diagnostics().len()
        } else {
            0
        };
        if let Some(root) = root {
            self.paint(ui, project, &pool, root);
        } else {
            ui.label("No active mask");
        }
        if let Ok(mut session) = self.session.lock() {
            session.finish_frame();
            if session.diagnostics().len() != diagnostic_count {
                ui.ctx().request_repaint();
            }
        }
    }

    fn paint(&mut self, ui: &mut egui::Ui, project: &EditorProject, pool: &ObjectPool, root: u16) {
        let keys = softkeys::build_with_settings(
            pool,
            root,
            self.page,
            self.mask_size,
            self.soft_key_size,
            self.soft_key_layout,
            project.object_id_format(),
        );
        self.page = keys.page;
        let mask_rect = Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(self.mask_size as f32, self.mask_size as f32),
        );
        let bounds = if keys.rect.is_positive() {
            mask_rect.union(keys.rect)
        } else {
            mask_rect
        };
        let available = ui.available_size().max(egui::vec2(1.0, 1.0));
        let scale = (available.x / bounds.width())
            .min(available.y / bounds.height())
            .clamp(0.01, 2.0);
        let (viewport, _) = ui.allocate_exact_size(available, egui::Sense::hover());
        let transform = egui::emath::TSTransform::new(
            viewport.min.to_vec2() - bounds.min.to_vec2() * scale,
            scale,
        );
        let mut mask_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("playback_mask_content")
                .max_rect(mask_rect),
        );
        mask_ui.set_clip_rect(
            (transform.inverse() * viewport.intersect(ui.clip_rect())).intersect(mask_rect),
        );
        mask_ui.with_visual_transform(transform, |ui| {
            if let Ok(id) = ObjectId::new(root) {
                crate::render_object_with_preview_session(
                    ui,
                    &self.session,
                    id,
                    Default::default(),
                    self.version,
                    self.soft_key_size,
                );
            }
        });
        for slot in keys.slots {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("playback_key", root, slot.index))
                    .max_rect(slot.rect),
            );
            child.set_clip_rect(
                (transform.inverse() * viewport.intersect(ui.clip_rect())).intersect(slot.rect),
            );
            child.with_visual_transform(transform, |ui| match slot.content {
                SlotContent::Key(id) => {
                    if let Ok(id) = ObjectId::new(id) {
                        crate::render_object_with_preview_session(
                            ui,
                            &self.session,
                            id,
                            Default::default(),
                            self.version,
                            self.soft_key_size,
                        );
                    }
                }
                SlotContent::Next | SlotContent::Previous => {
                    let next = matches!(slot.content, SlotContent::Next);
                    if ui
                        .put(
                            slot.rect,
                            egui::Button::new(if next { "Next" } else { "Previous" }),
                        )
                        .clicked()
                    {
                        self.page = if next {
                            (self.page + 1) % keys.pages
                        } else {
                            (self.page + keys.pages - 1) % keys.pages
                        };
                    }
                }
                _ => {}
            });
        }
    }
}
