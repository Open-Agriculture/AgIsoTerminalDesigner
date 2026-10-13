//! Project-local component browser and source editor.
use super::*;
use crate::EditorProject;
use ag_iso_stack::object_pool::{
    object::{Object, VtString},
    object_attributes::FontType,
    ObjectId, ObjectType,
};
use eframe::egui;

/// UI drafts live in egui temporary state; saving invokes the same atomic service as AI.
pub fn show(ui: &mut egui::Ui, project: &mut EditorProject) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
    ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
    ui.spacing_mut().interact_size.y = 26.0;
    ui.heading("Components");
    if let Some(error) = ui.data_mut(|d| d.get_temp::<String>(egui::Id::new("component_error"))) {
        ui.colored_label(ui.visuals().error_fg_color, error);
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
                            paint_definition_preview(ui, rect, &preview, definition.root_node)
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
                paint_definition_preview(ui, rect, &preview, definition.root_node)
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
        egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.id().with(definition.id),
            false,
        )
        .show_header(ui, |ui| {
            crate::object_picker::preview(ui, |ui, rect| {
                paint_definition_preview(ui, rect, &preview, definition.root_node)
            });
            ui.vertical(|ui| {
                ui.label(format!("{} / {}", category, definition.name));
                ui.weak(definition.id.to_string());
            });
        })
        .body(|ui| {
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
                    if let Err(e) = ComponentService::duplicate_definition(project, definition.id) {
                        error = Some(e.to_string());
                    }
                }
                if ui.button("Delete").clicked() {
                    if let Err(e) = ComponentService::delete_definition(project, definition.id) {
                        error = Some(e.to_string());
                    }
                }
            });
        });
    }
    ui.separator();
    let instances = project.component_source().instances.clone();
    for instance in instances {
        let definition = &project.component_source().definitions[&instance.component_id];
        let root = instance.compiled_ids[&definition.root_node];
        let label = format!("{} ({}, {})", definition.name, instance.x, instance.y);
        egui::CollapsingHeader::new(label)
            .id_salt(instance.id)
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
) {
    match preview {
        Ok(preview) => {
            let (pool, ids) = preview.as_ref();
            if let Some(object) = ids
                .get(&node)
                .and_then(|id| ag_iso_stack::object_pool::ObjectId::new(*id).ok())
                .and_then(|id| pool.object_by_id(id))
            {
                crate::object_picker::object_preview(ui, rect, pool, object);
            }
        }
        Err(_) => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No preview",
                egui::FontId::proportional(10.0),
                egui::Color32::DARK_GRAY,
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
                            .show(ui, |ui| {
                                ui.text_edit_singleline(&mut node.name);
                                for (property, value) in &mut node.properties {
                                    ui.horizontal(|ui| {
                                        ui.label(property);
                                        let font_type = if property == "value"
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
                                            property,
                                            value,
                                            font_type,
                                        );
                                    });
                                }
                                for (property, reference) in &mut node.references {
                                    ui.label(format!("Reference: {property}"));
                                    if let Some(descriptor) =
                                        crate::object_properties::get_property_descriptors(
                                            node.object_type,
                                        )
                                        .into_iter()
                                        .find(|d| d.name == property)
                                    {
                                        if let crate::PropertySemantic::ObjectReference {
                                            allowed_types,
                                        } = descriptor.semantic
                                        {
                                            if let Some(selected) = crate::object_picker::button(
                                                ui,
                                                (node.id, property, "external_reference"),
                                                "Choose external object…",
                                                &format!("Change {property} reference on component node {}", node.name),
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
                                    value_editor(ui, node.id, property, &mut value);
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
                        egui::ComboBox::from_id_salt("node_type")
                            .selected_text(format!("{ty:?}"))
                            .show_ui(ui, |ui| {
                                for choice in ag_iso_stack::object_pool::ObjectType::values() {
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
                                        .insert(descriptor.name.into(), ComponentReference::Null);
                                } else if let Ok(value) =
                                    crate::object_properties::get_object_property(
                                        &object,
                                        descriptor.name,
                                    )
                                {
                                    properties.insert(descriptor.name.into(), value);
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
                                    ui.label(&parameter.property);
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
                                        && parameter.property == "value")
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
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    format!("Cannot edit encoded string: {error}"),
                );
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
    match pool.object_by_id(font_id) {
        Some(Object::FontAttributes(font)) => Ok(font.font_type),
        _ => Err("Font Attributes reference is missing or invalid".to_owned()),
    }
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
                                paint_definition_preview(ui, rect, &preview, node.id);
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
                    paint_definition_preview(ui, rect, &preview, *id)
                });
                true
            },
        ) {
            draft.1 = node;
        }
        if let Some(node) = definition.nodes.iter().find(|n| n.id == draft.1) {
            egui::ComboBox::from_id_salt("parameter_property")
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
                            descriptor.name.to_owned(),
                            descriptor.name,
                        );
                    }
                });
            egui::ComboBox::from_id_salt("parameter_kind")
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
                        property: draft.2.clone(),
                        kind: draft.3,
                        default_value: serde_json::json!(id),
                    });
                    ComponentService::edit_component_definition(candidate, updated)
                        .map_err(|error| error.to_string())
                });
                ui.data_mut(|d| d.insert_temp(graphic_key, graphic.clone()));
            }
            if ui.button("Add declared parameter").clicked() {
                let value = if draft.3 == ComponentParameterKind::Graphic {
                    graphic
                } else if draft.3 == ComponentParameterKind::ObjectReference {
                    match node.references.get(&draft.2) {
                        Some(ComponentReference::ExternalObject(target)) => {
                            serde_json::to_value(target).unwrap()
                        }
                        _ => serde_json::Value::Null,
                    }
                } else {
                    node.properties
                        .get(&draft.2)
                        .cloned()
                        .or_else(|| {
                            crate::object_properties::get_object_property(
                                &crate::default_object(node.object_type),
                                &draft.2,
                            )
                            .ok()
                        })
                        .unwrap_or(serde_json::Value::Null)
                };
                let parameter = ComponentParameter {
                    id: ComponentParameterId::default(),
                    name: draft.0.clone(),
                    description: None,
                    target_node: draft.1,
                    property: draft.2.clone(),
                    kind: draft.3,
                    default_value: value,
                };
                let mut candidate = definition.clone();
                candidate.parameters.push(parameter);
                match validate_definition(&candidate) {
                    Ok(()) => *definition = candidate,
                    Err(e) => {
                        report_error(ui, e.to_string());
                    }
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
    let mut x = instance.x;
    let mut y = instance.y;
    let mut moved = false;
    ui.horizontal(|ui| {
        ui.label("Position");
        moved |= ui.add(egui::DragValue::new(&mut x)).changed();
        moved |= ui.add(egui::DragValue::new(&mut y)).changed();
    });
    let mut result = Ok(());
    if moved {
        result = ComponentService::move_instance(project, id, x, y);
    }
    for parameter in &definition.parameters {
        ui.label(&parameter.name);
        let value = instance
            .overrides
            .get(&parameter.id)
            .unwrap_or(&parameter.default_value);
        let font_type = (parameter.kind == ComponentParameterKind::Text
            && parameter.property == "value")
            .then(|| font_type_for_instance_node(&materialized, id, parameter.target_node));
        let initial_text = if parameter.kind == ComponentParameterKind::Text {
            serde_json::from_value::<VtString>(value.clone())
                .map_err(|error| error.to_string())
                .and_then(|value| decode_editor_text(&value, font_type.as_ref()))
        } else {
            Ok(serde_json::to_string(value).unwrap())
        };
        let key = ui.id().with((id, parameter.id, project.revision()));
        let mut draft = ui.data_mut(|d| {
            d.get_temp::<String>(key)
                .unwrap_or_else(|| initial_text.clone().unwrap_or_default())
        });
        if parameter.kind == ComponentParameterKind::Graphic {
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
        } else if parameter.kind == ComponentParameterKind::ObjectReference {
            if let Some(node) = definition
                .nodes
                .iter()
                .find(|node| node.id == parameter.target_node)
            {
                if let Some(descriptor) =
                    crate::object_properties::get_property_descriptors(node.object_type)
                        .into_iter()
                        .find(|d| d.name == parameter.property)
                {
                    if let crate::PropertySemantic::ObjectReference { allowed_types } =
                        descriptor.semantic
                    {
                        if let Some(selected) = crate::object_picker::button(
                            ui,
                            (id, parameter.id, "override_reference"),
                            "Choose referenced object…",
                            &format!("Choose the draft value for parameter '{}'. Use Set override to apply it.", parameter.name),
                            project,
                            None,
                            true,
                            |object| {
                                allowed_types.is_empty()
                                    || allowed_types.contains(&object.object_type())
                            },
                            |ui, selected| {
                                ui.label("Preview after Set override:");
                                crate::object_picker::impact::review_change(ui, project, |candidate| {
                                    let value = serde_json::to_value(selected.0.map(|id| crate::ObjectTarget::id(id.value()))).unwrap();
                                    ComponentService::set_component_override(candidate, id, parameter.id, value).map_err(|error| error.to_string())
                                })
                            },
                        ) {
                            draft = serde_json::to_string(
                                &selected.0.map(|id| crate::ObjectTarget::id(id.value())),
                            )
                            .unwrap();
                        }
                    }
                }
            }
            ui.label(&draft);
        } else if parameter.kind == ComponentParameterKind::Text {
            match &initial_text {
                Ok(_) => {
                    ui.text_edit_singleline(&mut draft);
                }
                Err(error) => {
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        format!("Cannot edit encoded string: {error}"),
                    );
                }
            }
        } else {
            ui.text_edit_singleline(&mut draft);
        }
        ui.data_mut(|d| d.insert_temp(key, draft.clone()));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    parameter.kind != ComponentParameterKind::Text || initial_text.is_ok(),
                    egui::Button::new("Set override"),
                )
                .clicked()
            {
                result = (if parameter.kind == ComponentParameterKind::Text {
                    Ok(serde_json::Value::String(draft.clone()))
                } else {
                    serde_json::from_str(&draft)
                })
                .map_err(|e| ComponentError::new("COMPONENT_OVERRIDE_TYPE_MISMATCH", e.to_string()))
                .and_then(|value| {
                    ComponentService::set_component_override(project, id, parameter.id, value)
                });
            }
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
}

fn report_error(ui: &mut egui::Ui, message: impl Into<String>) {
    let message = message.into();
    ui.data_mut(|d| d.insert_temp(egui::Id::new("component_error"), message.clone()));
    ui.colored_label(ui.visuals().error_fg_color, message);
}
