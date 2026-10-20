//! Project-local component browser and source editor.
use super::*;
use crate::EditorProject;
use ag_iso_stack::object_pool::{
    object::{Object, VtString},
    object_attributes::FontType,
    ObjectId, ObjectType,
};
use eframe::egui;

#[derive(Clone)]
struct PendingInspectorEdit {
    session: uuid::Uuid,
    revision: u64,
    frame: u64,
    instance: ComponentInstanceId,
    position: Option<(i32, i32)>,
    overrides: std::collections::BTreeMap<ComponentParameterId, serde_json::Value>,
}

/// Finish valid drafts whose inspector disappeared during this frame's navigation.
/// Call after rendering the editor panes, before committing queued primitive edits.
pub fn finish_inspector_edits(ctx: &egui::Context, project: &mut EditorProject) {
    let key = egui::Id::new("pending_component_inspectors");
    let pending = ctx
        .data_mut(|data| data.remove_temp::<Vec<PendingInspectorEdit>>(key))
        .unwrap_or_default();
    let mut retained = Vec::new();
    let revision = project.revision();
    for edit in pending {
        if edit.session != project.ui_session_id() || edit.revision != revision {
            continue;
        }
        if edit.frame == ctx.cumulative_frame_nr() {
            retained.push(edit);
            continue;
        }
        let result = project.edit_source("Edit component instance", |_, source, _, _| {
            let instance = source
                .instances
                .iter_mut()
                .find(|instance| instance.id == edit.instance)
                .ok_or_else(|| {
                    ComponentError::new(
                        "COMPONENT_INSTANCE_NOT_FOUND",
                        "Component instance no longer exists",
                    )
                })?;
            if let Some((x, y)) = edit.position {
                instance.x = x;
                instance.y = y;
            }
            instance.overrides.extend(edit.overrides);
            Ok(())
        });
        if let Err(error) = result {
            ctx.data_mut(|data| {
                data.insert_temp(egui::Id::new("component_error"), error.to_string())
            });
        }
    }
    ctx.data_mut(|data| data.insert_temp(key, retained));
}

/// UI drafts live in egui temporary state; saving invokes the same atomic service as AI.
pub fn show(ui: &mut egui::Ui, project: &mut EditorProject) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
    ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
    ui.spacing_mut().interact_size.y = 26.0;
    ui.heading("Components");
    if let Some(error) = ui.data_mut(|d| d.get_temp::<String>(egui::Id::new("component_error"))) {
        crate::ui_style::error_text(ui, &error);
        if ui.button("Dismiss error").clicked() {
            ui.data_mut(|d| d.remove::<String>(egui::Id::new("component_error")));
        }
    }
    let key = ui.id().with("component_search");
    let mut search = ui.data_mut(|d| d.get_temp::<String>(key).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search by name or ID…"));
    ui.data_mut(|d| d.insert_temp(key, search.clone()));
    let find_id = ui.make_persistent_id("find_component");
    if ui.button("Find component…").clicked() {
        crate::object_picker::open(ui.ctx(), find_id);
    }
    if let Some(id) = crate::object_picker::window(
        ui.ctx(),
        find_id,
        "Find component",
        |ui, query| {
            let mut selected = None;
            let mut count = 0;
            for definition in project.component_source().definitions.values() {
                if !crate::object_picker::matches(&definition.name, definition.id, query) {
                    continue;
                }
                count += 1;
                let preview = definition_preview(ui.ctx(), project, definition);
                ui.push_id(definition.id, |ui| {
                    if crate::object_picker::row(
                        ui,
                        &definition.name,
                        &definition.id.to_string(),
                        false,
                        |ui, rect| {
                            paint_definition_preview(
                                ui,
                                rect,
                                &preview,
                                definition.root_node,
                                project.vt_version,
                            )
                        },
                    ) {
                        selected = Some(definition.id);
                    }
                });
            }
            if count == 0 {
                ui.label("No matching components.");
            }
            selected
        },
        |ui, id| {
            let Some(definition) = project.component_source().definitions.get(id) else {
                ui.label("This component is no longer available.");
                return false;
            };
            ui.label("Open this component's master editor. Its contents will not change until you edit and save it.");
            ui.strong(&definition.name);
            ui.label(definition.id.to_string());
            let preview = definition_preview(ui.ctx(), project, definition);
            crate::object_picker::review_preview(ui, |ui, rect| {
                paint_definition_preview(
                    ui,
                    rect,
                    &preview,
                    definition.root_node,
                    project.vt_version,
                )
            });
            true
        },
    ) {
        open_editor(ui, &project.component_source().definitions[&id]);
    }
    let mut error = None;
    if ui.button("Create from selection…").clicked() {
        if let Some(id) = project.get_selected().0 {
            match plan_component_capture(project, id.value(), "New component".into()) {
                Ok(plan) => {
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("component_capture"), plan))
                }
                Err(e) => error = Some(e.to_string()),
            }
        }
    }
    let definitions: Vec<_> = project
        .component_source()
        .definitions
        .values()
        .cloned()
        .collect();
    for definition in definitions {
        let category = definition.category_path.join(" / ");
        let text = format!(
            "{} / {} {}",
            category,
            definition.name,
            definition.tags.join(" ")
        );
        if !crate::object_picker::matches(&text, definition.id, &search) {
            continue;
        }
        let preview = definition_preview(ui.ctx(), project, &definition);
        let vt_version = project.vt_version;
        crate::ui_style::collapsing_section(
            ui,
            ui.id().with(definition.id),
            false,
            |ui| {
                crate::object_picker::preview(ui, |ui, rect| {
                    paint_definition_preview(ui, rect, &preview, definition.root_node, vt_version)
                });
                ui.vertical(|ui| {
                    ui.label(format!("{} / {}", category, definition.name));
                    ui.weak(definition.id.to_string());
                });
            },
            |ui| {
                ui.label(&definition.description);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Instantiate").clicked() {
                        let parent = project.get_selected().0.map(|id| id.value());
                        if let Some(parent) = parent {
                            if let Err(e) = ComponentService::instantiate_component(
                                project,
                                definition.id,
                                parent,
                                0,
                                0,
                                Default::default(),
                            ) {
                                error = Some(e.to_string());
                            }
                        } else {
                            error = Some("Select an ordinary positioned parent first".into());
                        }
                    }
                    if ui.button("Edit master").clicked() {
                        open_editor(ui, &definition);
                    }
                    if ui.button("Duplicate").clicked() {
                        if let Err(e) =
                            ComponentService::duplicate_definition(project, definition.id)
                        {
                            error = Some(e.to_string());
                        }
                    }
                    if ui.button("Delete").clicked() {
                        if let Err(e) = ComponentService::delete_definition(project, definition.id)
                        {
                            error = Some(e.to_string());
                        }
                    }
                });
            },
        );
    }
    ui.separator();
    let instances = project.component_source().instances.clone();
    for instance in instances {
        let definition = &project.component_source().definitions[&instance.component_id];
        let root = instance.compiled_ids[&definition.root_node];
        let label = format!(
            "{} · {} ({}, {})",
            instance.name, definition.name, instance.x, instance.y
        );
        egui::CollapsingHeader::new(label)
            .id_salt(instance.id)
            .icon(crate::ui_style::collapsing_icon)
            .show(ui, |ui| {
                if ui
                    .selectable_label(
                        project
                            .get_selected()
                            .0
                            .is_some_and(|id| id.value() == root),
                        "Instance",
                    )
                    .clicked()
                {
                    project
                        .get_mut_selected()
                        .set(ag_iso_stack::object_pool::NullableObjectId::new(root));
                }
                for node in &definition.nodes {
                    if ui.selectable_label(false, &node.name).clicked() {
                        project.get_mut_selected().set(
                            ag_iso_stack::object_pool::NullableObjectId::new(
                                instance.compiled_ids[&node.id],
                            ),
                        );
                    }
                }
            });
    }
    if let Some(e) = error {
        report_error(ui, e);
    }
    capture_dialog(ui.ctx(), project);
    edit_dialog(ui.ctx(), project);
}
type DefinitionPreview = Result<
    std::sync::Arc<(
        ag_iso_stack::object_pool::ObjectPool,
        std::collections::BTreeMap<ComponentNodeId, u16>,
    )>,
    String,
>;

fn definition_preview(
    ctx: &egui::Context,
    project: &EditorProject,
    definition: &ComponentDefinition,
) -> DefinitionPreview {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    project.revision().hash(&mut hash);
    serde_json::to_vec(definition)
        .unwrap_or_default()
        .hash(&mut hash);
    let hash = hash.finish();
    let key = egui::Id::new(("component_preview", definition.id));
    if let Some((old_hash, preview)) =
        ctx.data_mut(|data| data.get_temp::<(u64, DefinitionPreview)>(key))
    {
        if old_hash == hash {
            return preview;
        }
    }
    let preview = super::materialize::preview_definition(project, definition)
        .map(std::sync::Arc::new)
        .map_err(|error| error.to_string());
    ctx.data_mut(|data| data.insert_temp(key, (hash, preview.clone())));
    preview
}

fn paint_definition_preview(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    preview: &DefinitionPreview,
    node: ComponentNodeId,
    version: ag_iso_stack::object_pool::vt_version::VtVersion,
) {
    match preview {
        Ok(preview) => {
            let (pool, ids) = preview.as_ref();
            if let Some(object) = ids
                .get(&node)
                .and_then(|id| ag_iso_stack::object_pool::ObjectId::new(*id).ok())
                .and_then(|id| pool.object_by_id(id))
            {
                crate::object_picker::object_preview_with_version(ui, rect, pool, object, version);
            }
        }
        Err(_) => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No preview",
                egui::FontId::proportional(10.0),
                crate::ui_style::palette(ui).muted_text,
            );
        }
    }
}

fn open_editor(ui: &mut egui::Ui, definition: &ComponentDefinition) {
    ui.data_mut(|d| {
        d.insert_temp(
            egui::Id::new("component_edit"),
            serde_json::to_string_pretty(definition).unwrap(),
        )
    });
}
fn capture_dialog(ctx: &egui::Context, project: &mut EditorProject) {
    let key = egui::Id::new("component_capture");
    let Some(mut plan) = ctx.data_mut(|d| d.get_temp::<CapturePlan>(key)) else {
        return;
    };
    let mut close = false;
    egui::Window::new("Create Component").show(ctx, |ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut plan.definition.name);
        ui.label("Description");
        ui.text_edit_multiline(&mut plan.definition.description);
        let mut category = plan.definition.category_path.join(" / ");
        ui.label("Category");
        if ui.text_edit_singleline(&mut category).changed() {
            plan.definition.category_path = category
                .split('/')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
        }
        let mut tags = plan.definition.tags.join(", ");
        ui.label("Tags");
        if ui.text_edit_singleline(&mut tags).changed() {
            plan.definition.tags = tags
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
        }
        ui.label(format!(
            "Parameters: {} (explicitly authored)",
            plan.definition.parameters.len()
        ));
        manual_parameter_form(ui, &mut plan.definition, project);
        for (_, property, target) in &plan.external_references {
            ui.label(format!(
                "External reference: {property} → ordinary object {target}"
            ));
        }
        let title = if plan.external_references.is_empty() {
            "Create"
        } else {
            "Keep listed external references and create"
        };
        if ui.button(title).clicked() {
            match ComponentService::create_component_from_selection(project, plan.clone(), true) {
                Ok(_) => close = true,
                Err(e) => {
                    report_error(ui, e.to_string());
                }
            }
        }
        if ui.button("Cancel").clicked() {
            close = true;
        }
    });
    ctx.data_mut(|d| {
        if close {
            d.remove::<CapturePlan>(key);
        } else {
            d.insert_temp(key, plan);
        }
    });
}
fn edit_dialog(ctx: &egui::Context, project: &mut EditorProject) {
    let key = egui::Id::new("component_edit");
    let Some(mut draft) = ctx.data_mut(|d| d.get_temp::<String>(key)) else {
        return;
    };
    let mut close = false;
    let editor_identity = serde_json::from_str::<ComponentDefinition>(&draft)
        .ok()
        .map(|d| (d.id, d.revision));
    egui::Window::new("Component Edit Mode — all instances")
        .id(egui::Id::new("component_master_window").with(editor_identity))
        .default_width(650.0)
        .show(ctx, |ui| {
            if let Ok(mut definition) = serde_json::from_str::<ComponentDefinition>(&draft) {
                egui::ScrollArea::vertical()
                    .max_height(550.0)
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut definition.name);
                        ui.label("Description");
                        ui.text_edit_multiline(&mut definition.description);
                        let mut category = definition.category_path.join(" / ");
                        ui.label("Category");
                        if ui.text_edit_singleline(&mut category).changed() {
                            definition.category_path = category
                                .split('/')
                                .map(|s| s.trim().to_owned())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                        let mut tags = definition.tags.join(", ");
                        ui.label("Tags");
                        if ui.text_edit_singleline(&mut tags).changed() {
                            definition.tags = tags
                                .split(',')
                                .map(|s| s.trim().to_owned())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                        let node_names: Vec<_> = definition
                            .nodes
                            .iter()
                            .map(|n| (n.id, n.name.clone()))
                            .collect();
                        let mut remove = None;
                        let preview_definition_source = definition.clone();
                        let editor_font_context = super::materialize::preview_definition(
                            project,
                            &preview_definition_source,
                        );
                        for node in &mut definition.nodes {
                            egui::CollapsingHeader::new(format!(
                                "{} ({:?})",
                                node.name, node.object_type
                            ))
                            .id_salt(node.id)
                            .icon(crate::ui_style::collapsing_icon)
                            .show(ui, |ui| {
                                ui.text_edit_singleline(&mut node.name);
                                for (property, value) in &mut node.properties {
                                    ui.horizontal(|ui| {
                                        ui.label(property.as_str());
                                        let font_type = if property.as_str() == "value"
                                            && matches!(
                                                node.object_type,
                                                ObjectType::OutputString
                                                    | ObjectType::InputString
                                                    | ObjectType::StringVariable
                                            )
                                        {
                                            Some(font_type_for_node(
                                                &editor_font_context,
                                                node.id,
                                            ))
                                        } else {
                                            None
                                        };
                                        value_editor_with_font(
                                            ui,
                                            node.id,
                                            property.as_str(),
                                            value,
                                            font_type,
                                        );
                                    });
                                }
                                for (property, reference) in &mut node.references {
                                    ui.label(format!("Reference: {}", property.as_str()));
                                    if let Some(descriptor) =
                                        crate::object_properties::get_property_descriptors(
                                            node.object_type,
                                        )
                                        .into_iter()
                                        .find(|d| d.name == *property)
                                    {
                                        if let crate::PropertySemantic::ObjectReference {
                                            allowed_types,
                                        } = descriptor.semantic
                                        {
                                            if let Some(selected) = crate::object_picker::button(
                                                ui,
                                                (node.id, *property, "external_reference"),
                                                "Choose external object…",
                                                &format!("Change {} reference on component node {}", property.as_str(), node.name),
                                                project,
                                                None,
                                                true,
                                                |object| {
                                                    allowed_types.is_empty()
                                                        || allowed_types
                                                            .contains(&object.object_type())
                                                },
                                                |ui, selected| {
                                                    ui.label("Preview of saving this component draft:");
                                                    let mut updated = preview_definition_source.clone();
                                                    let target = updated.nodes.iter_mut().find(|target| target.id == node.id).unwrap();
                                                    target.references.insert(property.clone(), selected.0.map_or(ComponentReference::Null, |id| ComponentReference::ExternalObject(crate::ObjectTarget::id(id.value()))));
                                                    crate::object_picker::impact::review_change(ui, project, |candidate| ComponentService::edit_component_definition(candidate, updated).map_err(|error| error.to_string()))
                                                },
                                            ) {
                                                *reference = selected.0.map_or(
                                                    ComponentReference::Null,
                                                    |id| {
                                                        ComponentReference::ExternalObject(
                                                            crate::ObjectTarget::id(id.value()),
                                                        )
                                                    },
                                                );
                                            }
                                        }
                                    }

                                    let mut value = serde_json::to_value(&*reference).unwrap();
                                    value_editor(ui, node.id, property.as_str(), &mut value);
                                    if let Ok(updated) = serde_json::from_value(value) {
                                        *reference = updated;
                                    }
                                }
                                for child in &mut node.children {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            node_names
                                                .iter()
                                                .find(|(id, _)| *id == child.child)
                                                .map(|(_, name)| name.as_str())
                                                .unwrap_or("Missing child"),
                                        );
                                        ui.add(egui::DragValue::new(&mut child.x));
                                        ui.add(egui::DragValue::new(&mut child.y));
                                    });
                                }
                                if node.id != definition.root_node
                                    && ui.button("Remove node").clicked()
                                {
                                    remove = Some(node.id);
                                }
                            });
                        }
                        if let Some(id) = remove {
                            definition.nodes.retain(|n| n.id != id);
                            for node in &mut definition.nodes {
                                node.children.retain(|c| c.child != id);
                            }
                        }
                        let type_key = ui.id().with("new_component_node_type");
                        let mut ty = ui.data_mut(|d| {
                            d.get_temp::<ag_iso_stack::object_pool::ObjectType>(type_key)
                                .unwrap_or(ag_iso_stack::object_pool::ObjectType::OutputString)
                        });
                        if !crate::vt_compatibility::supported_object_type(ty, project.vt_version) {
                            ty = ag_iso_stack::object_pool::ObjectType::OutputString;
                        }
                        egui::ComboBox::from_id_salt("node_type")
                            .icon(crate::ui_style::combo_box_icon)
                            .selected_text(format!("{ty:?}"))
                            .show_ui(ui, |ui| {
                                for choice in ag_iso_stack::object_pool::ObjectType::values() {
                                    if !crate::vt_compatibility::supported_object_type(choice, project.vt_version) {
                                        continue;
                                    }
                                    ui.selectable_value(&mut ty, choice, format!("{choice:?}"));
                                }
                            });
                        ui.data_mut(|d| d.insert_temp(type_key, ty));
                        if ui.button("Add node").clicked() {
                            let object = crate::default_object(ty);
                            let mut properties = std::collections::BTreeMap::new();
                            let mut references = std::collections::BTreeMap::new();
                            for descriptor in crate::object_properties::get_property_descriptors(ty)
                                .into_iter()
                                .filter(|d| d.writable)
                            {
                                if matches!(
                                    descriptor.semantic,
                                    crate::PropertySemantic::ObjectReference { .. }
                                ) {
                                    references
                                        .insert(descriptor.name, ComponentReference::Null);
                                } else if let Ok(value) =
                                    crate::object_properties::get_object_property(
                                        &object,
                                        descriptor.name,
                                    )
                                {
                                    properties.insert(descriptor.name, value);
                                }
                            }
                            let id = ComponentNodeId::default();
                            if crate::allowed_object_relationships::get_allowed_child_refs(
                                definition
                                    .nodes
                                    .iter()
                                    .find(|n| n.id == definition.root_node)
                                    .unwrap()
                                    .object_type,
                                project.vt_version,
                            )
                            .contains(&ty)
                            {
                                definition
                                    .nodes
                                    .iter_mut()
                                    .find(|n| n.id == definition.root_node)
                                    .unwrap()
                                    .children
                                    .push(ComponentChild {
                                        child: id,
                                        x: 0,
                                        y: 0,
                                    });
                            }
                            definition.nodes.push(ComponentNode {
                                macro_references: vec![],
                                object_list: None,
                                id,
                                name: format!("{ty:?}"),
                                object_type: ty,
                                properties,
                                references,
                                children: vec![],
                            });
                        }
                        ui.separator();
                        ui.heading("Explicit parameters");
                        let mut remove_parameter = None;
                        let parameter_preview_source = definition.clone();
                        for parameter in &mut definition.parameters {
                            ui.push_id(parameter.id, |ui| {
                                ui.horizontal(|ui| {
                                    ui.text_edit_singleline(&mut parameter.name);
                                    ui.label(parameter.property.as_str());
                                    if ui.button("Remove parameter").clicked() {
                                        remove_parameter = Some(parameter.id);
                                    }
                                });
                                ui.label("Default");
                                if parameter.kind == ComponentParameterKind::Graphic {
                                    crate::graphics::ui::select(
                                        ui,
                                        project,
                                        &mut parameter.default_value,
                                        |candidate, id| {
                                            let mut updated = parameter_preview_source.clone();
                                            updated.parameters.iter_mut().find(|p| p.id == parameter.id).unwrap().default_value = serde_json::json!(id);
                                            ComponentService::edit_component_definition(candidate, updated).map_err(|error| error.to_string())
                                        },
                                    );
                                } else {
                                    let font_type = (parameter.kind
                                        == ComponentParameterKind::Text
                                        && parameter.property.as_str() == "value")
                                        .then(|| {
                                            font_type_for_node(
                                                &editor_font_context,
                                                parameter.target_node,
                                            )
                                        });
                                    value_editor_with_font(
                                        ui,
                                        parameter.target_node,
                                        &format!("default_{}", parameter.id),
                                        &mut parameter.default_value,
                                        font_type,
                                    );
                                }
                                let mut description =
                                    parameter.description.clone().unwrap_or_default();
                                if ui.text_edit_singleline(&mut description).changed() {
                                    parameter.description = Some(description);
                                }
                            });
                        }
                        if let Some(id) = remove_parameter {
                            definition.parameters.retain(|p| p.id != id);
                        }
                        manual_parameter_form(ui, &mut definition, project);
                    });
                draft = serde_json::to_string_pretty(&definition).unwrap();
            }
            ui.collapsing("Advanced source", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(250.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut draft)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
            if ui.button("Save master").clicked() {
                match serde_json::from_str::<ComponentDefinition>(&draft)
                    .map_err(|e| e.to_string())
                    .and_then(|d| {
                        ComponentService::edit_component_definition(project, d)
                            .map_err(|e| e.to_string())
                    }) {
                    Ok(()) => close = true,
                    Err(e) => {
                        report_error(ui, e);
                    }
                }
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    ctx.data_mut(|d| {
        if close {
            d.remove::<String>(key);
        } else {
            d.insert_temp(key, draft);
        }
    });
}
fn value_editor(
    ui: &mut egui::Ui,
    node: ComponentNodeId,
    property: &str,
    value: &mut serde_json::Value,
) {
    value_editor_with_font(ui, node, property, value, None);
}

fn value_editor_with_font(
    ui: &mut egui::Ui,
    node: ComponentNodeId,
    property: &str,
    value: &mut serde_json::Value,
    font_type: Option<Result<FontType, String>>,
) {
    if let Ok(vt_string) = serde_json::from_value::<VtString>(value.clone()) {
        match decode_editor_text(&vt_string, font_type.as_ref()) {
            Ok(mut text) => {
                if ui.text_edit_singleline(&mut text).changed() {
                    *value = serde_json::Value::String(text);
                }
            }
            Err(error) => {
                crate::ui_style::error_text(ui, &format!("Cannot edit encoded string: {error}"));
            }
        }
        return;
    }
    if let Some(mut boolean) = value.as_bool() {
        if ui.checkbox(&mut boolean, "").changed() {
            *value = boolean.into();
        }
    } else {
        let key = ui.id().with((node, property));
        let mut text = ui.data_mut(|d| {
            d.get_temp::<String>(key)
                .unwrap_or_else(|| value.to_string())
        });
        if ui.text_edit_singleline(&mut text).changed() {
            if let Ok(parsed) = serde_json::from_str(&text) {
                *value = parsed;
            }
        }
        ui.data_mut(|d| d.insert_temp(key, text));
    }
}

fn decode_editor_text(
    value: &VtString,
    font_type: Option<&Result<FontType, String>>,
) -> Result<String, String> {
    match value {
        VtString::Text(text) => Ok(text.clone()),
        VtString::Encoded { encoded_bytes } if encoded_bytes.starts_with(&[0xFF, 0xFE]) => {
            value.decode(FontType::Latin1).map_err(str::to_owned)
        }
        VtString::Encoded { .. } => value
            .decode(
                font_type
                    .ok_or_else(|| "Font Attributes are unavailable".to_owned())?
                    .as_ref()
                    .map_err(Clone::clone)
                    .copied()?,
            )
            .map_err(str::to_owned),
    }
}

fn font_type_for_node(
    context: &ComponentResult<(
        ag_iso_stack::object_pool::ObjectPool,
        std::collections::BTreeMap<ComponentNodeId, u16>,
    )>,
    node_id: ComponentNodeId,
) -> Result<FontType, String> {
    let (pool, ids) = context.as_ref().map_err(|error| error.to_string())?;
    let id = ids
        .get(&node_id)
        .copied()
        .and_then(|id| ObjectId::new(id).ok())
        .ok_or_else(|| "string node is missing from the compiled component".to_owned())?;
    let object = pool
        .object_by_id(id)
        .ok_or_else(|| "compiled string node is missing".to_owned())?;
    font_type_for_object(pool, object)
}

fn font_type_for_instance_node(
    context: &ComponentResult<super::MaterializedProject>,
    instance_id: ComponentInstanceId,
    node_id: ComponentNodeId,
) -> Result<FontType, String> {
    let materialized = context.as_ref().map_err(|error| error.to_string())?;
    let id = materialized
        .source_map
        .by_component_node
        .get(&(instance_id, node_id))
        .copied()
        .and_then(|id| ObjectId::new(id).ok())
        .ok_or_else(|| "string node is missing from the materialized instance".to_owned())?;
    let object = materialized
        .pool
        .object_by_id(id)
        .ok_or_else(|| "materialized string node is missing".to_owned())?;
    font_type_for_object(&materialized.pool, object)
}

fn font_type_for_object(
    pool: &ag_iso_stack::object_pool::ObjectPool,
    object: &Object,
) -> Result<FontType, String> {
    let font_id = match object {
        Object::OutputString(field) => field.font_attributes,
        Object::InputString(field) => field.font_attributes,
        Object::StringVariable(variable) => {
            let variable_id = variable.id;
            pool.objects()
                .iter()
                .find_map(|object| match object {
                    Object::OutputString(field)
                        if field.variable_reference.0 == Some(variable_id) =>
                    {
                        Some(field.font_attributes)
                    }
                    Object::InputString(field)
                        if field.variable_reference.0 == Some(variable_id) =>
                    {
                        Some(field.font_attributes)
                    }
                    _ => None,
                })
                .ok_or_else(|| "String Variable has no referencing string field".to_owned())?
        }
        _ => return Err("component node is not a string object".to_owned()),
    };
    match pool.object_by_nullable_id(font_id) {
        Some(Object::FontAttributes(font)) => Ok(font.font_type),
        _ => Err("Font Attributes reference is missing or invalid".to_owned()),
    }
}
fn declared_parameter_default(
    node: &ComponentNode,
    property: crate::object_properties::PropertyKey,
    kind: ComponentParameterKind,
) -> serde_json::Value {
    let value = node
        .properties
        .get(&property)
        .cloned()
        .or_else(|| {
            crate::object_properties::get_object_property(
                &crate::default_object(node.object_type),
                property,
            )
            .ok()
        })
        .unwrap_or(serde_json::Value::Null);

    if kind == ComponentParameterKind::Boolean
        && node.object_type == ObjectType::InputBoolean
        && property.as_str() == "value"
    {
        if let Some(number) = value.as_i64() {
            return serde_json::Value::Bool(number != 0);
        }
        if let Some(number) = value.as_u64() {
            return serde_json::Value::Bool(number != 0);
        }
        if let Some(number) = value.as_f64() {
            return serde_json::Value::Bool(number != 0.0);
        }
    }

    value
}

fn manual_parameter_form(
    ui: &mut egui::Ui,
    definition: &mut ComponentDefinition,
    project: &mut EditorProject,
) {
    ui.collapsing("Add Parameter", |ui| {
        let key = ui.id().with("parameter_draft");
        let mut draft = ui.data_mut(|d| {
            d.get_temp::<(String, ComponentNodeId, String, ComponentParameterKind)>(key)
                .unwrap_or((
                    String::new(),
                    definition.root_node,
                    String::new(),
                    ComponentParameterKind::Text,
                ))
        });
        ui.label("Name");
        ui.text_edit_singleline(&mut draft.0);
        let picker_id = ui.make_persistent_id("parameter_node");
        if ui
            .button(
                definition
                    .nodes
                    .iter()
                    .find(|node| node.id == draft.1)
                    .map_or("Choose node…", |node| node.name.as_str()),
            )
            .clicked()
        {
            crate::object_picker::open(ui.ctx(), picker_id);
        }
        if let Some(node) = crate::object_picker::window(
            ui.ctx(),
            picker_id,
            "Choose component node",
            |ui, query| {
                let preview = definition_preview(ui.ctx(), project, definition);
                let mut selected = None;
                for node in &definition.nodes {
                    if !crate::object_picker::matches(&node.name, node.id, query) {
                        continue;
                    }
                    ui.push_id(node.id, |ui| {
                        if crate::object_picker::row(
                            ui,
                            &node.name,
                            &node.id.to_string(),
                            node.id == draft.1,
                            |ui, rect| {
                                paint_definition_preview(
                                    ui,
                                    rect,
                                    &preview,
                                    node.id,
                                    project.vt_version,
                                );
                            },
                        ) {
                            selected = Some(node.id);
                        }
                    });
                }
                selected
            },
            |ui, id| {
                let Some(node) = definition.nodes.iter().find(|node| node.id == *id) else {
                    ui.label("This node is no longer available.");
                    return false;
                };
                ui.label("Use this node as the target for the parameter you are declaring.");
                ui.strong(&node.name);
                ui.label(node.id.to_string());
                let preview = definition_preview(ui.ctx(), project, definition);
                crate::object_picker::review_preview(ui, |ui, rect| {
                    paint_definition_preview(ui, rect, &preview, *id, project.vt_version)
                });
                true
            },
        ) {
            draft.1 = node;
        }
        if let Some(node) = definition.nodes.iter().find(|n| n.id == draft.1) {
            egui::ComboBox::from_id_salt("parameter_property")
                .icon(crate::ui_style::combo_box_icon)
                .selected_text(if draft.2.is_empty() {
                    "Choose property"
                } else {
                    &draft.2
                })
                .show_ui(ui, |ui| {
                    for descriptor in
                        crate::object_properties::get_property_descriptors(node.object_type)
                            .into_iter()
                            .filter(|d| d.writable)
                    {
                        ui.selectable_value(
                            &mut draft.2,
                            descriptor.name.to_string(),
                            descriptor.name.as_str(),
                        );
                    }
                });
            egui::ComboBox::from_id_salt("parameter_kind")
                .icon(crate::ui_style::combo_box_icon)
                .selected_text(format!("{:?}", draft.3))
                .show_ui(ui, |ui| {
                    for kind in [
                        ComponentParameterKind::Text,
                        ComponentParameterKind::Number,
                        ComponentParameterKind::Boolean,
                        ComponentParameterKind::ObjectReference,
                        ComponentParameterKind::Colour,
                        ComponentParameterKind::Enum,
                        ComponentParameterKind::Graphic,
                    ] {
                        ui.selectable_value(&mut draft.3, kind, format!("{kind:?}"));
                    }
                });
            let graphic_key = key.with("graphic_default");
            let mut graphic = ui.data_mut(|d| {
                d.get_temp::<serde_json::Value>(graphic_key)
                    .unwrap_or_default()
            });
            if draft.3 == ComponentParameterKind::Graphic {
                draft.2 = "graphic".into();
                if draft.0.is_empty() {
                    draft.0 = "Icon".into();
                }
                ui.label("Target must be a PictureGraphic node");
                crate::graphics::ui::select(ui, project, &mut graphic, |candidate, id| {
                    let mut updated = definition.clone();
                    updated.parameters.push(ComponentParameter {
                        id: ComponentParameterId::default(),
                        name: draft.0.clone(),
                        description: None,
                        target_node: draft.1,
                        property: ComponentParameterTarget::Graphic,
                        kind: draft.3,
                        default_value: serde_json::json!(id),
                    });
                    ComponentService::edit_component_definition(candidate, updated)
                        .map_err(|error| error.to_string())
                });
                ui.data_mut(|d| d.insert_temp(graphic_key, graphic.clone()));
            }
            if ui.button("Add declared parameter").clicked() {
                let target = if draft.3 == ComponentParameterKind::Graphic {
                    Some(ComponentParameterTarget::Graphic)
                } else {
                    crate::object_properties::PropertyKey::parse(&draft.2)
                        .map(ComponentParameterTarget::Property)
                };
                if let Some(target) = target {
                    let value = if draft.3 == ComponentParameterKind::Graphic {
                        graphic
                    } else if draft.3 == ComponentParameterKind::ObjectReference {
                        match target {
                            ComponentParameterTarget::Property(property) => {
                                match node.references.get(&property) {
                                    Some(ComponentReference::ExternalObject(target)) => {
                                        serde_json::to_value(target).unwrap()
                                    }
                                    _ => serde_json::Value::Null,
                                }
                            }
                            ComponentParameterTarget::Graphic => unreachable!(),
                        }
                    } else if let ComponentParameterTarget::Property(property) = target {
                        declared_parameter_default(node, property, draft.3)
                    } else {
                        serde_json::Value::Null
                    };
                    let parameter = ComponentParameter {
                        id: ComponentParameterId::default(),
                        name: draft.0.clone(),
                        description: None,
                        target_node: draft.1,
                        property: target,
                        kind: draft.3,
                        default_value: value,
                    };
                    let mut candidate = definition.clone();
                    candidate.parameters.push(parameter);
                    match validate_definition(&candidate) {
                        Ok(()) => *definition = candidate,
                        Err(e) => report_error(ui, e.to_string()),
                    }
                } else {
                    report_error(
                        ui,
                        "Choose a supported property before adding the parameter",
                    );
                }
            }
        }
        ui.data_mut(|d| d.insert_temp(key, draft));
    });
}
pub fn inspector(
    ui: &mut egui::Ui,
    project: &mut EditorProject,
    id: ComponentInstanceId,
    node: ComponentNodeId,
) {
    let Some(instance) = project
        .component_source()
        .instances
        .iter()
        .find(|i| i.id == id)
        .cloned()
    else {
        return;
    };
    let definition = project.component_source().definitions[&instance.component_id].clone();
    let materialized = super::materialize::materialize_project(project);
    let mut pending_edit = PendingInspectorEdit {
        session: project.ui_session_id(),
        revision: project.revision(),
        frame: ui.ctx().cumulative_frame_nr(),
        instance: id,
        position: None,
        overrides: Default::default(),
    };
    ui.heading(&definition.name);
    ui.label(format!("Instance: {id}"));
    ui.label(format!(
        "Node: {}",
        definition
            .nodes
            .iter()
            .find(|n| n.id == node)
            .map(|n| n.name.as_str())
            .unwrap_or("?")
    ));
    let position_key = ui.id().with((
        "instance_position",
        project.ui_session_id(),
        id,
        project.revision(),
    ));
    let mut position = ui
        .data(|data| data.get_temp::<(i32, i32)>(position_key))
        .unwrap_or((instance.x, instance.y));
    let mut finished = false;
    let mut cancelled = false;
    ui.horizontal(|ui| {
        ui.label("Position");
        let x = ui.add(egui::DragValue::new(&mut position.0));
        let y = ui.add(egui::DragValue::new(&mut position.1));
        cancelled = (x.has_focus() || y.has_focus() || x.lost_focus() || y.lost_focus())
            && ui.input(|input| input.key_pressed(egui::Key::Escape));
        finished = x.drag_stopped() || y.drag_stopped() || x.lost_focus() || y.lost_focus();
    });
    ui.data_mut(|data| {
        if finished || cancelled {
            data.remove::<(i32, i32)>(position_key);
        } else {
            data.insert_temp(position_key, position);
        }
    });
    let mut result = Ok(());
    if finished && !cancelled && position != (instance.x, instance.y) {
        result = ComponentService::move_instance(project, id, position.0, position.1);
    } else if !finished && !cancelled && position != (instance.x, instance.y) {
        pending_edit.position = Some(position);
    }
    for parameter in &definition.parameters {
        ui.label(&parameter.name);
        let value = instance
            .overrides
            .get(&parameter.id)
            .unwrap_or(&parameter.default_value);
        let font_type = (parameter.kind == ComponentParameterKind::Text
            && parameter.property.as_str() == "value")
            .then(|| font_type_for_instance_node(&materialized, id, parameter.target_node));
        let initial_text = if parameter.kind == ComponentParameterKind::Text {
            serde_json::from_value::<VtString>(value.clone())
                .map_err(|error| error.to_string())
                .and_then(|value| decode_editor_text(&value, font_type.as_ref()))
        } else {
            Ok(serde_json::to_string(value).unwrap())
        };
        let key = ui.id().with((
            project.ui_session_id(),
            id,
            parameter.id,
            project.revision(),
        ));
        let mut draft = ui.data_mut(|d| {
            d.get_temp::<String>(key)
                .unwrap_or_else(|| initial_text.clone().unwrap_or_default())
        });
        let mut apply = false;
        let mut response = None;
        if parameter.kind == ComponentParameterKind::Graphic {
            let previous = draft.clone();
            let mut value = serde_json::from_str(&draft).unwrap_or_default();
            crate::graphics::ui::select(ui, project, &mut value, |candidate, graphic| {
                ComponentService::set_component_override(
                    candidate,
                    id,
                    parameter.id,
                    serde_json::json!(graphic),
                )
                .map_err(|error| error.to_string())
            });
            draft = value.to_string();
            apply = draft != previous;
        } else if parameter.kind == ComponentParameterKind::ObjectReference {
            if let Some(node) = definition
                .nodes
                .iter()
                .find(|node| node.id == parameter.target_node)
            {
                if let Some(descriptor) =
                    crate::object_properties::get_property_descriptors(node.object_type)
                        .into_iter()
                        .find(|d| parameter.property == ComponentParameterTarget::Property(d.name))
                {
                    if let crate::PropertySemantic::ObjectReference { allowed_types } =
                        descriptor.semantic
                    {
                        if let Some(selected) = crate::object_picker::button(
                            ui,
                            (id, parameter.id, "override_reference"),
                            "Choose referenced object…",
                            &format!("Choose the value for parameter '{}'.", parameter.name),
                            project,
                            None,
                            true,
                            |object| {
                                allowed_types.is_empty()
                                    || allowed_types.contains(&object.object_type())
                            },
                            |ui, selected| {
                                ui.label("Preview after changing the reference:");
                                crate::object_picker::impact::review_change(
                                    ui,
                                    project,
                                    |candidate| {
                                        let value = serde_json::to_value(
                                            selected
                                                .0
                                                .map(|id| crate::ObjectTarget::id(id.value())),
                                        )
                                        .unwrap();
                                        ComponentService::set_component_override(
                                            candidate,
                                            id,
                                            parameter.id,
                                            value,
                                        )
                                        .map_err(|error| error.to_string())
                                    },
                                )
                            },
                        ) {
                            draft = serde_json::to_string(
                                &selected.0.map(|id| crate::ObjectTarget::id(id.value())),
                            )
                            .unwrap();
                            apply = true;
                        }
                    }
                }
            }
            ui.label(&draft);
        } else if parameter.kind == ComponentParameterKind::Text {
            match &initial_text {
                Ok(_) => {
                    response = Some(ui.text_edit_singleline(&mut draft));
                }
                Err(error) => {
                    crate::ui_style::error_text(
                        ui,
                        &format!("Cannot edit encoded string: {error}"),
                    );
                }
            }
        } else {
            response = Some(ui.text_edit_singleline(&mut draft));
        }
        if let Some(response) = response {
            let cancelled = (response.has_focus() || response.lost_focus())
                && ui.input(|input| input.key_pressed(egui::Key::Escape));
            if cancelled {
                draft = initial_text.clone().unwrap_or_default();
                response.surrender_focus();
            } else if response.lost_focus()
                || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
            {
                apply = initial_text.as_ref().is_ok_and(|initial| *initial != draft);
                response.surrender_focus();
            }
        }
        ui.data_mut(|d| d.insert_temp(key, draft.clone()));
        if apply {
            result = (if parameter.kind == ComponentParameterKind::Text {
                Ok(serde_json::Value::String(draft.clone()))
            } else {
                serde_json::from_str(&draft)
            })
            .map_err(|e| ComponentError::new("COMPONENT_OVERRIDE_TYPE_MISMATCH", e.to_string()))
            .and_then(|edited| {
                if &edited == value {
                    Ok(())
                } else {
                    ComponentService::set_component_override(project, id, parameter.id, edited)
                }
            });
        } else if initial_text.as_ref().is_ok_and(|initial| *initial != draft) {
            let value = if parameter.kind == ComponentParameterKind::Text {
                Some(serde_json::Value::String(draft))
            } else {
                serde_json::from_str(&draft).ok()
            };
            if let Some(value) = value {
                pending_edit.overrides.insert(parameter.id, value);
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                result = ComponentService::reset_component_override(project, id, parameter.id);
                ui.data_mut(|d| d.remove::<String>(key));
            }
        });
    }
    ui.label("Size, styling and internal positions are inherited from the master.");
    if ui.button("Reset All Overrides").clicked() {
        result = ComponentService::reset_all_overrides(project, id);
    }
    if ui.button("Edit Component").clicked() {
        open_editor(ui, &definition);
    }
    if ui.button("Detach Instance").clicked() {
        result = ComponentService::detach_component_instance(project, id);
    }
    if ui.button("Duplicate Instance").clicked() {
        result = ComponentService::duplicate_component_instance(
            project,
            id,
            instance.x + 10,
            instance.y + 10,
        )
        .map(|_| ());
    }
    if ui.button("Delete Instance").clicked() {
        result = ComponentService::delete_component_instance(project, id);
    }
    if let Err(e) = result {
        report_error(ui, e.to_string());
    }
    let key = egui::Id::new("pending_component_inspectors");
    ui.data_mut(|data| {
        let mut pending = data
            .get_temp::<Vec<PendingInspectorEdit>>(key)
            .unwrap_or_default();
        pending.retain(|edit| edit.instance != id || edit.session != project.ui_session_id());
        if pending_edit.position.is_some() || !pending_edit.overrides.is_empty() {
            pending.push(pending_edit);
        }
        data.insert_temp(key, pending);
    });
}

fn report_error(ui: &mut egui::Ui, message: impl Into<String>) {
    let message = message.into();
    ui.data_mut(|d| d.insert_temp(egui::Id::new("component_error"), message.clone()));
    crate::ui_style::error_text(ui, &message);
}
