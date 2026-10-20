//! Immediate, undoable management of ordinary project attribute objects.
use ag_iso_stack::object_pool::{ObjectId, ObjectType};
use ag_iso_terminal_designer::{components::MaterializedSource, EditorProject};
use eframe::egui;
use std::collections::BTreeMap;

const TYPES: [(ObjectType, &str); 3] = [
    (ObjectType::FontAttributes, "Font"),
    (ObjectType::LineAttributes, "Line"),
    (ObjectType::FillAttributes, "Fill"),
];

pub struct SharedAttributes {
    pub open: bool,
    kind: ObjectType,
    query: String,
    name: String,
    draft_key: Option<(ObjectId, u64, uuid::Uuid)>,
    session: u64,
}

impl Default for SharedAttributes {
    fn default() -> Self {
        Self {
            open: false,
            kind: ObjectType::FontAttributes,
            query: String::new(),
            name: String::new(),
            draft_key: None,
            session: 0,
        }
    }
}

struct Usage {
    objects: BTreeMap<u16, String>,
    error: Option<String>,
}

/// Include materialized component consumers and editor-only allowed targets.
/// Failed materialization leaves canonical uses visible but prevents deletion.
fn usage(project: &EditorProject, id: ObjectId) -> Usage {
    let mut objects = BTreeMap::new();
    for object in project.get_pool().objects() {
        let referenced = object.referenced_objects().contains(&id);
        let allowed = project
            .get_object_info(object)
            .allowed_references
            .contains(&id);
        if referenced || allowed {
            let suffix = if allowed { " (allowed target)" } else { "" };
            objects.insert(
                object.id().value(),
                format!("{}{suffix}", project.object_display_name(object)),
            );
        }
    }
    let error = match project.materialize_shared() {
        Ok(materialized) => {
            for object in materialized.pool.objects() {
                if !object.referenced_objects().contains(&id) {
                    continue;
                }
                if let Some(MaterializedSource::ComponentNode {
                    instance_id,
                    node_id,
                }) = materialized
                    .source_map
                    .by_object_id
                    .get(&object.id().value())
                {
                    let source = project.component_source();
                    if let Some(instance) = source.instances.iter().find(|i| i.id == *instance_id) {
                        if let Some(definition) = source.definitions.get(&instance.component_id) {
                            if let Some(node) = definition.nodes.iter().find(|n| n.id == *node_id) {
                                objects.insert(
                                    object.id().value(),
                                    format!(
                                        "{} / {} (component {})",
                                        definition.name, node.name, instance_id
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            None
        }
        Err(error) => Some(error.to_string()),
    };
    Usage { objects, error }
}

enum Action {
    Create(ObjectType),
    Duplicate(ObjectId),
    Rename(ObjectId, String),
    Delete(ObjectId),
}

impl SharedAttributes {
    pub fn open(&mut self, ctx: &egui::Context, project: &EditorProject) {
        self.open = true;
        self.session = ctx.cumulative_frame_nr();
        self.draft_key = None;
        if let Some(object) = project
            .get_selected()
            .0
            .and_then(|id| project.get_pool().object_by_id(id))
        {
            if TYPES.iter().any(|(kind, _)| *kind == object.object_type()) {
                self.kind = object.object_type();
            }
        }
    }

    /// Returns true when a consumer was selected and Properties should be revealed.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        project: &mut EditorProject,
        error: &mut Option<String>,
    ) -> bool {
        // A different pane can change selection before this window is rendered.
        // Finish the old name against its original object rather than replacing it.
        if let Some((id, revision, session)) = self.draft_key {
            if !self.open || project.get_selected().0 != Some(id) {
                if revision == project.revision() && session == project.ui_session_id() {
                    if let Some(object) = project.get_pool().object_by_id(id) {
                        if self.name != project.get_object_info(object).get_name(object) {
                            project.rename_object(id, self.name.clone());
                        }
                    }
                }
                self.draft_key = None;
            }
        }
        if !self.open {
            return false;
        }
        let selected = project
            .get_selected()
            .0
            .and_then(|id| project.get_pool().object_by_id(id))
            .filter(|object| TYPES.iter().any(|(kind, _)| *kind == object.object_type()));
        if let Some(object) = selected {
            let key = (object.id(), project.revision(), project.ui_session_id());
            if self.draft_key != Some(key) {
                if self.draft_key.map(|key| key.0) != Some(object.id()) {
                    self.kind = object.object_type();
                }
                self.name = project.get_object_info(object).get_name(object);
                self.draft_key = Some(key);
            }
        } else {
            self.draft_key = None;
        }
        let mut action = None;
        let mut navigate = false;
        let mut next_kind = self.kind;
        egui::Window::new("Shared Attributes")
            .open(&mut self.open)
            .default_size([800.0, 580.0])
            .show(ctx, |ui| {
                ui.label("Press Enter or leave a field to save. Edits are undoable, and every consumer shares the same attribute object.");
                ui.horizontal(|ui| {
                    for (kind, label) in TYPES {
                        ui.selectable_value(&mut next_kind, kind, label);
                    }
                    if ui.button("Create").clicked() {
                        action = Some(Action::Create(self.kind));
                    }
                });
                ui.separator();
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(230.0);
                        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Search by name or ID…").desired_width(230.0));
                        egui::ScrollArea::vertical().id_salt("attribute_list").max_height(460.0).show(ui, |ui| {
                            let mut found = false;
                            for object in project.get_pool().objects() {
                                if object.object_type() != self.kind || !ag_iso_terminal_designer::object_picker::matches_object(project, object, &self.query) {
                                    continue;
                                }
                                found = true;
                                if ui.selectable_label(selected.is_some_and(|s| s.id() == object.id()), project.object_display_name(object)).clicked() {
                                    project.get_mut_selected().set(object.id().into());
                                }
                            }
                            if !found { ui.label("No matching attributes."); }
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_min_width(400.0);
                        egui::ScrollArea::vertical().id_salt("attribute_details").max_height(460.0).show(ui, |ui| {
                            let Some(object) = selected.filter(|o| o.object_type() == self.kind) else {
                                ui.label("Select an attribute to edit it and inspect its uses.");
                                return;
                            };
                            let id = object.id();
                            let uses = usage(project, id);
                            ui.horizontal(|ui| {
                                let response = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(220.0));
                                let original = project.get_object_info(object).get_name(object);
                                let cancelled = response.has_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Escape));
                                if cancelled {
                                    self.name = original;
                                    response.surrender_focus();
                                } else if (response.lost_focus()
                                    || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))))
                                    && self.name != original
                                {
                                    action = Some(Action::Rename(id, self.name.clone()));
                                    response.surrender_focus();
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui.button("Duplicate").clicked() { action = Some(Action::Duplicate(id)); }
                                if ui.add_enabled(uses.objects.is_empty() && uses.error.is_none(), egui::Button::new("Delete")).clicked() { action = Some(Action::Delete(id)); }
                            });
                            ui.separator();
                            // Invalidate pending property drafts after undo, ID changes, and project replacement.
                            ui.push_id((self.session, id.value(), project.revision()), |ui| {
                                ag_iso_terminal_designer::render_property_editor(ui, object, project);
                            });
                            ui.separator();
                            ui.strong(format!("Used by {} objects", uses.objects.len()));
                            if !uses.objects.is_empty() {
                                ui.small("Detach these references before deleting this attribute.");
                            }
                            for (consumer, label) in uses.objects {
                                if ui.link(label).clicked() {
                                    project.get_mut_selected().set(ObjectId::new(consumer).unwrap().into());
                                    navigate = true;
                                }
                            }
                            if let Some(error) = uses.error {
                                ag_iso_terminal_designer::ui_style::error_text_with_color(ui, &format!("Component usage is unavailable; deletion is disabled: {error}"), ui.visuals().warn_fg_color);
                            }
                        });
                    });
                });
            });
        // Render the previous editor once on a tab switch so focused drafts can
        // commit on lost_focus. Give the next tab fresh draft IDs.
        if next_kind != self.kind {
            self.kind = next_kind;
            self.session = self.session.wrapping_add(1);
        }
        if let Some(action) = action {
            // Commit same-frame property edits before copying or removing their object.
            project.commit_queued("Edit shared attribute");
            if let Some(message) = project.take_operation_error() {
                *error = Some(message);
                return navigate;
            }
            match action {
                Action::Create(kind) => match project.allocate_object_id(kind) {
                    Ok(_) => {
                        if let Some(id) = project
                            .create_object(kind, project.generate_smart_name_for_new_object(kind))
                        {
                            project.get_mut_selected().set(id.into());
                        }
                    }
                    Err(message) => *error = Some(message.to_string()),
                },
                Action::Duplicate(id) => match project.duplicate_shared_attribute(id) {
                    Ok(id) => project.get_mut_selected().set(id.into()),
                    Err(message) => *error = Some(message.to_string()),
                },
                Action::Rename(id, name) => {
                    project.rename_object(id, name);
                }
                Action::Delete(id) => {
                    project.delete_object(id);
                }
            }
        }
        if navigate {
            self.open = false;
        }
        navigate
    }
}
