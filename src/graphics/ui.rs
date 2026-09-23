use super::integration::GraphicService;
use crate::EditorProject;
use ag_iso_stack::object_pool::{object::Object, NullableObjectId, ObjectType};
use eframe::egui;

pub fn show(ui: &mut egui::Ui, project: &mut EditorProject) {
    ui.heading("Graphics");
    let mut error = None;
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button("Upload Graphic").clicked() {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if let Err(upload_error) =
                        GraphicService::upload(project, &path.to_string_lossy(), bytes)
                    {
                        error = Some(upload_error.to_string());
                    }
                }
                Err(read_error) => error = Some(read_error.to_string()),
            }
        }
    }
    let search_key = ui.id().with("graphic_search");
    let mut search = ui.data_mut(|data| data.get_temp::<String>(search_key).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search graphics..."));
    ui.data_mut(|data| data.insert_temp(search_key, search.clone()));
    let search = search.to_lowercase();

    ui.strong("PICTURE GRAPHICS");
    for object in project
        .get_pool()
        .objects_by_type(ObjectType::PictureGraphic)
    {
        let Object::PictureGraphic(_) = object else {
            continue;
        };
        let name = project.get_object_info(object).get_name(object);
        let matches = format!("{} {}", object.id().value(), name)
            .to_lowercase()
            .contains(&search);
        if !matches {
            continue;
        }
        ui.push_id(("picture_graphic", object.id().value()), |ui| {
            ui.group(|ui| {
                ui.label(&name);
                ui.label(format!("PictureGraphic {}", object.id().value()));
                if ui.button("Go to PictureGraphic").clicked() {
                    project
                        .get_mut_selected()
                        .set(NullableObjectId(Some(object.id())));
                }
            });
        });
    }

    ui.strong("AVAILABLE");
    let assets: Vec<_> = project.graphic_library().assets.values().cloned().collect();
    for asset in assets {
        let matches = format!(
            "{} {} {} {}",
            asset.name,
            asset.description.as_deref().unwrap_or(""),
            asset.tags.join(" "),
            asset.original_file_name.as_deref().unwrap_or(""),
        )
        .to_lowercase()
        .contains(&search);
        if !matches {
            continue;
        }
        ui.push_id(("available_graphic", asset.id), |ui| {
            ui.group(|ui| {
                ui.label(&asset.name);
                if let Some(description) = &asset.description {
                    ui.label(description);
                }
                if let Some(selected) = project.get_selected().0.filter(|id| {
                    matches!(
                        project.get_pool().object_by_id(*id),
                        Some(Object::PictureGraphic(_))
                    )
                }) {
                    if ui
                        .button(format!("Convert into PictureGraphic {}", selected.value()))
                        .clicked()
                    {
                        if let Err(value) = GraphicService::convert_asset_into_picture_graphic(
                            project,
                            asset.id,
                            selected.value(),
                        ) {
                            error = Some(value.to_string());
                        }
                    }
                }
                if ui.button("Create PictureGraphic").clicked() {
                    match GraphicService::create_picture_graphic_from_asset(project, asset.id) {
                        Ok(id) => project.get_mut_selected().set(NullableObjectId(
                            ag_iso_stack::object_pool::ObjectId::new(id).ok(),
                        )),
                        Err(value) => error = Some(value.to_string()),
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Download original").clicked() {
                    let (filename, bytes) = asset.download();
                    if let Some(path) = rfd::FileDialog::new().set_file_name(filename).save_file() {
                        if let Err(value) = std::fs::write(path, bytes) {
                            error = Some(value.to_string());
                        }
                    }
                }
                if ui.button("Delete").clicked() {
                    if let Err(value) = GraphicService::delete_available_asset(project, asset.id) {
                        error = Some(value.to_string());
                    }
                }
            });
        });
    }
    let key = ui.id().with("graphic_error");
    if let Some(error) = error {
        ui.data_mut(|data| data.insert_temp(key, error));
    }
    if let Some(error) = ui.data_mut(|data| data.get_temp::<String>(key)) {
        ui.colored_label(egui::Color32::RED, error);
        if ui.button("Dismiss graphic error").clicked() {
            ui.data_mut(|data| data.remove::<String>(key));
        }
    }
}

/// Select an existing reusable PictureGraphic. Available assets must be
/// converted through GraphicService before a component stores the reference.
pub fn select(ui: &mut egui::Ui, project: &mut EditorProject, value: &mut serde_json::Value) {
    let selected = value.as_u64();
    egui::ComboBox::from_id_salt(ui.next_auto_id())
        .selected_text(selected.map_or_else(
            || "Select PictureGraphic".to_owned(),
            |id| format!("PictureGraphic {id}"),
        ))
        .show_ui(ui, |ui| {
            ui.strong("PICTURE GRAPHICS");
            for object in project
                .get_pool()
                .objects_by_type(ObjectType::PictureGraphic)
            {
                ui.selectable_value(
                    value,
                    serde_json::json!(object.id().value()),
                    format!(
                        "{} - #{}",
                        project.get_object_info(object).get_name(object),
                        object.id().value()
                    ),
                );
            }
            ui.separator();
            ui.strong("AVAILABLE");
            let assets: Vec<_> = project.graphic_library().assets.values().cloned().collect();
            for asset in assets {
                if ui.selectable_label(false, &asset.name).clicked() {
                    if let Ok(id) =
                        GraphicService::create_picture_graphic_from_asset(project, asset.id)
                    {
                        *value = serde_json::json!(id);
                    }
                }
            }
        });
}
