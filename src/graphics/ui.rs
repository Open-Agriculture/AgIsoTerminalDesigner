use super::{integration::*, *};
use crate::EditorProject;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, project: &mut EditorProject) {
    ui.heading("Graphics");
    let mut error = None;
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button("Upload Graphic").clicked() {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if let Err(e) = GraphicService::upload(project, &path.to_string_lossy(), bytes)
                    {
                        error = Some(e.to_string());
                    }
                }
                Err(e) => error = Some(e.to_string()),
            }
        }
    }
    let search_key = ui.id().with("graphic_search");
    let mut search = ui.data_mut(|d| d.get_temp::<String>(search_key).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search graphics…"));
    ui.data_mut(|d| d.insert_temp(search_key, search.clone()));
    let assets: Vec<_> = project.graphic_library().assets.values().cloned().collect();
    let owners =
        assignments(project.component_source(), project.graphic_library()).unwrap_or_default();
    for assigned in [true, false] {
        ui.strong(if assigned { "ASSIGNED" } else { "AVAILABLE" });
        for asset in &assets {
            let owner = owners.get(&asset.id);
            if assigned != owner.is_some()
                || !format!(
                    "{} {} {}",
                    asset.name,
                    asset.description.as_deref().unwrap_or(""),
                    asset.tags.join(" ")
                )
                .to_lowercase()
                .contains(&search.to_lowercase())
            {
                continue;
            }
            ui.push_id(asset.id, |ui| {
                let key = ui.id().with("rename");
                let mut name = ui.data_mut(|d| d.get_temp::<String>(key));
                let response = ui
                    .group(|ui| {
                        ui.label(&asset.name);
                        if let Some(owner) = owner {
                            if let Some(id) = object_id(project.component_source(), owner) {
                                ui.label(format!("Assigned to PictureGraphic {id}"));
                                if ui.button("Go to PictureGraphic").clicked() {
                                    project.get_mut_selected().set(
                                        ag_iso_stack::object_pool::NullableObjectId(
                                            ag_iso_stack::object_pool::ObjectId::new(id).ok(),
                                        ),
                                    );
                                }
                                if let Some(parent) = project.get_selected().0 {
                                    if ui
                                        .button("Use existing PictureGraphic")
                                        .on_hover_text("Add a child link to the selected parent")
                                        .clicked()
                                    {
                                        if let Err(e) = GraphicService::reuse_child(
                                            project,
                                            asset.id,
                                            parent.value(),
                                        ) {
                                            error = Some(e.to_string());
                                        }
                                    }
                                }
                            }
                        } else {
                            ui.label("Available");
                            if let Some(id) = project.get_selected().0.filter(|id| {
                                matches!(
                                    project.get_pool().object_by_id(*id),
                                    Some(
                                        ag_iso_stack::object_pool::object::Object::PictureGraphic(
                                            _
                                        )
                                    )
                                )
                            }) {
                                if ui
                                    .button(format!("Assign to PictureGraphic {}", id.value()))
                                    .clicked()
                                {
                                    if let Err(e) =
                                        GraphicService::assign(project, id.value(), Some(asset.id))
                                    {
                                        error = Some(e.to_string());
                                    }
                                }
                            }
                        }
                        if let Some(PictureGraphicSource::Primitive { object_id }) = owner {
                            if ui.button("Remove assignment").clicked() {
                                if let Err(e) = GraphicService::assign(project, *object_id, None) {
                                    error = Some(e.to_string());
                                }
                            }
                        }
                        if let Some(draft) = &mut name {
                            ui.text_edit_singleline(draft);
                            if ui.button("Save name").clicked() {
                                if let Err(e) =
                                    GraphicService::rename(project, asset.id, draft.clone())
                                {
                                    error = Some(e.to_string());
                                }
                                name = None;
                            }
                        }
                    })
                    .response;
                response.on_hover_ui(|ui| {
                    ui.horizontal(|ui| {
                        #[cfg(not(target_arch = "wasm32"))]
                        if ui.button("Download").clicked() {
                            let (filename, bytes) = asset.download();
                            if let Some(path) =
                                rfd::FileDialog::new().set_file_name(filename).save_file()
                            {
                                if let Err(e) = std::fs::write(path, bytes) {
                                    error = Some(e.to_string());
                                }
                            }
                        }
                        if ui.button("Rename").clicked() {
                            name = Some(asset.name.clone());
                        }
                        if ui
                            .add_enabled(!assigned, egui::Button::new("Delete"))
                            .clicked()
                        {
                            if let Err(e) = GraphicService::delete(project, asset.id) {
                                error = Some(e.to_string());
                            }
                        }
                    });
                });
                ui.data_mut(|d| {
                    if let Some(name) = name {
                        d.insert_temp(key, name);
                    } else {
                        d.remove::<String>(key);
                    }
                });
            });
        }
    }
    let key = ui.id().with("graphic_error");
    if let Some(e) = error {
        ui.data_mut(|d| d.insert_temp(key, e));
    }
    if let Some(e) = ui.data_mut(|d| d.get_temp::<String>(key)) {
        ui.colored_label(egui::Color32::RED, e);
        if ui.button("Dismiss graphic error").clicked() {
            ui.data_mut(|d| d.remove::<String>(key));
        }
    }
}

pub fn select(ui: &mut egui::Ui, project: &EditorProject, value: &mut serde_json::Value) {
    egui::ComboBox::from_id_salt(ui.next_auto_id())
        .selected_text(
            serde_json::from_value::<GraphicAssetId>(value.clone())
                .ok()
                .and_then(|id| project.graphic_library().assets.get(&id))
                .map(|a| a.name.as_str())
                .unwrap_or("Select project graphic"),
        )
        .show_ui(ui, |ui| {
            for asset in project.graphic_library().assets.values() {
                let usage = project.picture_for_graphic(asset.id).ok().flatten();
                let label = usage
                    .as_ref()
                    .and_then(|o| object_id(project.component_source(), o))
                    .map(|id| format!("{} — assigned to PictureGraphic {id}", asset.name))
                    .unwrap_or_else(|| format!("{} — available", asset.name));
                ui.selectable_value(value, serde_json::json!(asset.id), label);
            }
        });
}
