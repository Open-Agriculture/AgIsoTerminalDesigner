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
    draft_key: Option<(ObjectId, u64)>,
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
        if !self.open {
            return false;
        }
        let selected = project
            .get_selected()
            .0
            .and_then(|id| project.get_pool().object_by_id(id))
            .filter(|object| TYPES.iter().any(|(kind, _)| *kind == object.object_type()));
        if let Some(object) = selected {
            let key = (object.id(), project.revision());
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
                ui.label("Edits apply immediately and are undoable. Every consumer shares the same attribute object.");
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
                                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(220.0));
                                if ui.button("Rename").clicked() { action = Some(Action::Rename(id, self.name.clone())); }
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
                                ui.colored_label(ui.visuals().warn_fg_color, format!("Component usage is unavailable; deletion is disabled: {error}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{object::Object, ObjectPool};
    use ag_iso_terminal_designer::{
        components::*, default_object, ObjectTarget, Operation, OperationTransaction,
    };
    use serde_json::json;

    fn project() -> EditorProject {
        let mut pool = ObjectPool::default();
        for (kind, id) in [
            (ObjectType::FontAttributes, 23000),
            (ObjectType::OutputString, 11000),
            (ObjectType::OutputString, 11001),
            (ObjectType::ObjectPointer, 27000),
            (ObjectType::DataMask, 1000),
        ] {
            let mut object = default_object(kind);
            object.mut_id().set_value(id).unwrap();
            if let Object::OutputString(field) = &mut object {
                field.font_attributes = ObjectId::new(23000).unwrap().into();
            }
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        project
    }

    #[test]
    fn usage_reports_each_consumer_and_allowed_target_and_blocks_deletion() {
        let mut project = project();
        let id = ObjectId::new(23000).unwrap();
        let mut transaction = OperationTransaction::new(None);
        transaction.add_operation(Operation::SetObjectPointerAllowedReferences {
            object: ObjectTarget::id(27000),
            references: vec![ObjectTarget::id(23000)],
        });
        project.execute_transaction(transaction).unwrap();
        let uses = usage(&project, id);
        assert!(uses.error.is_none(), "{:?}", uses.error);
        assert_eq!(
            uses.objects.keys().copied().collect::<Vec<_>>(),
            vec![11000, 11001, 27000]
        );
        assert!(uses.objects[&27000].contains("allowed target"));
        assert!(!project.delete_object(id));
        assert!(project.get_pool().object_by_id(id).is_some());
    }

    #[test]
    fn usage_includes_component_consumer_and_navigation_source() {
        let mut project = project();
        let component_id = ComponentId::default();
        let node_id = ComponentNodeId::default();
        ComponentService::edit_component_definition(
            &mut project,
            ComponentDefinition {
                schema_version: 1,
                id: component_id,
                name: "Caption".into(),
                description: String::new(),
                category_path: vec![],
                tags: vec![],
                revision: 0,
                root_node: node_id,
                nodes: vec![ComponentNode {
                    id: node_id,
                    name: "Label".into(),
                    object_type: ObjectType::OutputString,
                    properties: BTreeMap::from([
                        ("width".into(), json!(80)),
                        ("height".into(), json!(20)),
                        ("value".into(), json!("Text")),
                    ]),
                    children: vec![],
                    references: BTreeMap::from([(
                        "font_attributes".into(),
                        ComponentReference::ExternalObject(ObjectTarget::id(23000)),
                    )]),
                    macro_references: vec![],
                    object_list: None,
                }],
                parameters: vec![],
            },
        )
        .unwrap();
        let instance = ComponentService::instantiate_component(
            &mut project,
            component_id,
            1000,
            0,
            0,
            BTreeMap::new(),
        )
        .unwrap();
        let uses = usage(&project, ObjectId::new(23000).unwrap());
        assert!(uses.error.is_none(), "{:?}", uses.error);
        let materialized = project.materialize_shared().unwrap();
        let compiled_id = materialized.source_map.by_component_node[&(instance, node_id)];
        assert!(uses.objects[&compiled_id].contains("Caption / Label"));
        project
            .get_mut_selected()
            .set(ObjectId::new(compiled_id).unwrap().into());
        project.update_selected();
        assert_eq!(
            project.editor_selection(),
            Some(EditorSelection::ComponentNode {
                instance_id: instance,
                node_id
            })
        );
        assert!(!project.delete_object(ObjectId::new(23000).unwrap()));
    }

    #[test]
    fn unused_attribute_can_be_deleted_and_restored() {
        let mut project = EditorProject::default();
        let id = project
            .create_object(ObjectType::LineAttributes, "Border".into())
            .unwrap();
        let uses = usage(&project, id);
        assert!(uses.objects.is_empty());
        assert!(uses.error.is_none());
        assert!(project.delete_object(id));
        assert!(project.undo_operation());
        assert!(project.get_pool().object_by_id(id).is_some());
    }

    #[test]
    fn switching_tabs_commits_numeric_edit_before_duplicate() {
        fn frame(
            ctx: &egui::Context,
            manager: &mut SharedAttributes,
            project: &mut EditorProject,
            time: f64,
            events: Vec<egui::Event>,
        ) -> Vec<(String, egui::Rect)> {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    let mut error = None;
                    manager.show(ctx, project, &mut error);
                    assert!(error.is_none(), "shared attribute action failed: {error:?}");
                },
            );
            project.commit_queued("Edit shared attribute");
            assert!(
                project.take_operation_error().is_none(),
                "queued shared attribute edit failed"
            );
            project.update_selected();
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        egui::Rect::from_min_size(text.pos, text.galley.size()),
                    )),
                    _ => None,
                })
                .collect()
        }

        fn click(
            ctx: &egui::Context,
            manager: &mut SharedAttributes,
            project: &mut EditorProject,
            time: &mut f64,
            position: egui::Pos2,
        ) -> Vec<(String, egui::Rect)> {
            let mut output = Vec::new();
            for pressed in [true, false] {
                *time += 0.05;
                output = frame(
                    ctx,
                    manager,
                    project,
                    *time,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            output
        }

        fn text_position(shapes: &[(String, egui::Rect)], text: &str) -> egui::Pos2 {
            shapes
                .iter()
                .find(|(candidate, _)| candidate == text)
                .unwrap_or_else(|| panic!("Missing {text}: {shapes:?}"))
                .1
                .center()
        }

        let mut pool = ObjectPool::default();
        let mut line = default_object(ObjectType::LineAttributes);
        line.mut_id().set_value(23000).unwrap();
        if let Object::LineAttributes(line) = &mut line {
            line.line_width = 3;
        }
        pool.add(line);
        let mut font = default_object(ObjectType::FontAttributes);
        font.mut_id().set_value(23001).unwrap();
        pool.add(font);
        let mut project = EditorProject::from(pool);
        project
            .get_mut_selected()
            .set(ObjectId::new(23000).unwrap().into());
        project.update_selected();
        let ctx = egui::Context::default();
        let mut manager = SharedAttributes::default();
        manager.open(&ctx, &project);
        let mut time = 0.0;
        let mut shapes = Vec::new();
        for _ in 0..3 {
            time += 0.05;
            shapes = frame(&ctx, &mut manager, &mut project, time, vec![]);
        }

        // Focus the Line Width DragValue and type a replacement without pressing
        // Enter, so losing focus to the Font tab is what commits the edit.
        let width_label = shapes
            .iter()
            .find(|(text, _)| text == "Line Width")
            .unwrap_or_else(|| panic!("Missing Line Width label: {shapes:?}"))
            .1;
        let width_value = shapes
            .iter()
            .filter(|(text, rect)| {
                text == "3"
                    && (rect.center().y - width_label.center().y).abs() < 2.0
                    && rect.left() >= width_label.right()
            })
            .min_by(|(_, a), (_, b)| a.left().total_cmp(&b.left()))
            .unwrap_or_else(|| panic!("Missing Line Width value: {shapes:?}"))
            .1
            .center();
        time += 0.05;
        frame(
            &ctx,
            &mut manager,
            &mut project,
            time,
            vec![
                egui::Event::PointerMoved(width_value),
                egui::Event::PointerButton {
                    pos: width_value,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: width_value,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        time += 0.05;
        frame(
            &ctx,
            &mut manager,
            &mut project,
            time,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            }],
        );
        time += 0.05;
        frame(
            &ctx,
            &mut manager,
            &mut project,
            time,
            vec![egui::Event::Text("7".into())],
        );

        time += 0.05;
        let shapes = frame(&ctx, &mut manager, &mut project, time, vec![]);
        click(
            &ctx,
            &mut manager,
            &mut project,
            &mut time,
            text_position(&shapes, "Font"),
        );
        let original = project
            .get_pool()
            .object_by_id(ObjectId::new(23000).unwrap())
            .unwrap();
        let Object::LineAttributes(original) = original else {
            panic!("original should be Line Attributes");
        };
        assert_eq!(original.line_width, 7, "tab switch should commit the edit");
        time += 0.05;
        let shapes = frame(&ctx, &mut manager, &mut project, time, vec![]);
        click(
            &ctx,
            &mut manager,
            &mut project,
            &mut time,
            text_position(&shapes, "Line"),
        );
        time += 0.05;
        let shapes = frame(&ctx, &mut manager, &mut project, time, vec![]);
        click(
            &ctx,
            &mut manager,
            &mut project,
            &mut time,
            text_position(&shapes, "Duplicate"),
        );

        let duplicated = project
            .get_selected()
            .0
            .and_then(|id| project.get_pool().object_by_id(id))
            .expect("duplicate should become selected");
        let Object::LineAttributes(duplicated) = duplicated else {
            panic!("duplicate should be Line Attributes");
        };
        assert_ne!(
            duplicated.id.value(),
            23000,
            "duplicate should have a fresh ID"
        );
        assert_eq!(duplicated.line_width, 7);
    }
}
