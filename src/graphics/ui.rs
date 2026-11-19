use super::{integration::GraphicService, preview, GraphicAssetId};
use crate::object_picker::checkerboard;
use crate::{object_picker, EditorProject};
use ag_iso_stack::object_pool::{object::Object, NullableObjectId, ObjectId, ObjectType};
use eframe::egui;

/// The panel and all PictureGraphic pickers share this unfiltered enumeration.
pub fn picture_graphics(project: &EditorProject) -> Vec<&Object> {
    project
        .get_pool()
        .objects_by_type(ObjectType::PictureGraphic)
}

pub fn go_to_picture_graphic(project: &EditorProject, id: ObjectId) {
    if matches!(
        project.get_pool().object_by_id(id),
        Some(Object::PictureGraphic(_))
    ) {
        // Normal deferred selection also opens hierarchy headers and the inspector.
        project.get_mut_selected().set(NullableObjectId(Some(id)));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Navigate,
    Assign,
    Create,
    Download,
    Delete,
}

const CARD_WIDTH: f32 = 100.0;

fn thumbnail_grid<T>(
    ui: &mut egui::Ui,
    salt: &str,
    items: impl Iterator<Item = T>,
    mut draw: impl FnMut(&mut egui::Ui, T),
) {
    let spacing = ui.spacing().item_spacing.x;
    let columns = ((ui.available_width() + spacing) / (CARD_WIDTH + spacing))
        .floor()
        .max(1.0) as usize;
    egui::Grid::new(salt).num_columns(columns).show(ui, |ui| {
        for (index, item) in items.enumerate() {
            draw(ui, item);
            if (index + 1) % columns == 0 {
                ui.end_row();
            }
        }
    });
}

/// Shared layout, hover controls, tooltip and context-menu anchor for both sources.
fn graphics_card(
    ui: &mut egui::Ui,
    name: &str,
    texture: Option<egui::TextureHandle>,
    available: bool,
) -> Option<Action> {
    let mut action = None;
    let visuals = ui.visuals().clone();
    egui::Frame::new()
        .fill(visuals.faint_bg_color)
        .stroke(egui::Stroke::new(
            1.0,
            visuals.widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH - 8.0);
            ui.vertical(|ui| {
                let (rect, response) = ui
                    .allocate_exact_size(egui::vec2(CARD_WIDTH - 8.0, 76.0), egui::Sense::click());
                checkerboard(ui.painter(), rect);
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
                    egui::StrokeKind::Inside,
                );
                if let Some(texture) = &texture {
                    object_picker::texture(ui, rect, texture);
                    object_picker::hover_preview(response.clone(), |ui, rect| {
                        object_picker::texture(ui, rect, texture);
                    });
                } else {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "No image",
                        egui::FontId::proportional(12.0),
                        visuals.weak_text_color(),
                    );
                }
                if response.clicked() && !available {
                    action = Some(Action::Navigate);
                }
                response.clone().on_hover_text(name).context_menu(|ui| {
                    if ui.button("Download").clicked() {
                        action = Some(Action::Download);
                        ui.close();
                    }
                    if available && ui.button("Delete").clicked() {
                        action = Some(Action::Delete);
                        ui.close();
                    }
                });
                // Rect containment keeps buttons visible when they take pointer hover.
                if ui.rect_contains_pointer(rect) {
                    let buttons: &[(Action, &str, &str)] = if available {
                        &[
                            (Action::Create, "+", "Create PictureGraphic"),
                            (Action::Assign, "🔗", "Assign to PictureGraphic"),
                        ]
                    } else {
                        &[(Action::Navigate, "↗", "Go to PictureGraphic")]
                    };
                    for (index, (value, icon, tooltip)) in buttons.iter().enumerate() {
                        let button_rect = egui::Rect::from_min_size(
                            rect.right_bottom() - egui::vec2(28.0 * (index + 1) as f32, 26.0),
                            egui::vec2(26.0, 24.0),
                        );
                        if ui
                            .put(button_rect, egui::Button::new(*icon))
                            .on_hover_text(*tooltip)
                            .clicked()
                        {
                            action = Some(*value);
                        }
                    }
                }
                ui.add(egui::Label::new(name).truncate())
                    .on_hover_text(name);
            });
        });
    action
}

/// Waiting uploads use the same searchable window and preview rows as objects.
pub fn available_button(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    project: &EditorProject,
    target: ObjectId,
) -> Option<GraphicAssetId> {
    let id = ui.make_persistent_id(salt);
    if ui.button("Choose Available…").clicked() {
        object_picker::open(ui.ctx(), id);
    }
    object_picker::window(
        ui.ctx(),
        id,
        "Choose available graphic",
        |ui, query| available_picker(ui, project, query),
        |ui, asset| {
            ui.label(format!("Replace the image in {}. The chosen graphic will be removed from Available. This is undoable.", object_picker::object_label(project, Some(target))));
            review_asset(ui, project, *asset)
        },
    )
}

fn review_asset(ui: &mut egui::Ui, project: &EditorProject, id: GraphicAssetId) -> bool {
    let Some(asset) = project.graphic_library().assets.get(&id) else {
        ui.label("This graphic is no longer available.");
        return false;
    };
    ui.strong(&asset.name);
    ui.label(format!("ID: {}", asset.id));
    let texture = preview::asset_texture(ui.ctx(), asset);
    object_picker::review_preview(ui, |ui, rect| {
        if let Some(texture) = &texture {
            object_picker::texture(ui, rect, texture);
        }
    });
    true
}

fn available_picker(
    ui: &mut egui::Ui,
    project: &EditorProject,
    query: &str,
) -> Option<GraphicAssetId> {
    let mut selected = None;
    let mut count = 0;
    for asset in project.graphic_library().assets.values() {
        if !object_picker::matches(&asset.name, asset.id, query) {
            continue;
        }
        count += 1;
        ui.push_id(asset.id, |ui| {
            let texture = preview::asset_texture(ui.ctx(), asset);
            if object_picker::row(ui, &asset.name, &asset.id.to_string(), false, |ui, rect| {
                if let Some(texture) = &texture {
                    object_picker::texture(ui, rect, texture);
                }
            }) {
                selected = Some(asset.id);
            }
        });
    }
    if count == 0 {
        ui.label("No matching available graphics.");
    }
    selected
}

fn download(filename: &str, bytes: &[u8]) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = rfd::FileDialog::new().set_file_name(filename).save_file() {
        std::fs::write(path, bytes).map_err(|error| error.to_string())?;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let filename = filename.to_owned();
        let bytes = bytes.to_vec();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .set_file_name(&filename)
                .save_file()
                .await
            {
                if let Err(error) = file.write(&bytes).await {
                    log::error!("Download failed: {error}");
                }
            }
        });
    }
    Ok(())
}

type UploadedFile = Result<(String, Vec<u8>), String>;

#[cfg(not(target_arch = "wasm32"))]
fn execute_file_dialog(future: impl std::future::Future<Output = ()> + Send + 'static) {
    std::thread::spawn(move || futures::executor::block_on(future));
}

#[cfg(target_arch = "wasm32")]
fn execute_file_dialog(future: impl std::future::Future<Output = ()> + 'static) {
    wasm_bindgen_futures::spawn_local(future);
}

fn start_graphic_upload(ui: &egui::Ui, busy_key: egui::Id, files_key: egui::Id) {
    ui.data_mut(|data| data.insert_temp(busy_key, true));
    let ctx = ui.ctx().clone();
    execute_file_dialog(async move {
        let files = rfd::AsyncFileDialog::new()
            .add_filter(
                "Image Files",
                &[
                    "png", "jpg", "jpeg", "bmp", "gif", "ico", "tiff", "tif", "webp",
                ],
            )
            .pick_files()
            .await;
        let mut loaded = Vec::new();
        if let Some(files) = files {
            for file in files {
                let filename = file.file_name();
                #[cfg(not(target_arch = "wasm32"))]
                let result = std::fs::read(file.path())
                    .map(|bytes| (filename.clone(), bytes))
                    .map_err(|error| format!("{filename}: {error}"));
                #[cfg(target_arch = "wasm32")]
                let result: UploadedFile = Ok((filename, file.read().await));
                loaded.push(result);
            }
        }
        ctx.data_mut(|data| {
            data.insert_temp(files_key, loaded);
            data.insert_temp(busy_key, false);
        });
        ctx.request_repaint();
    });
}

pub fn show(ui: &mut egui::Ui, project: &mut EditorProject) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
    ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
    ui.spacing_mut().interact_size.y = 26.0;
    ui.heading("Graphics");
    let mut error = None;
    let project_session = project.ui_session_id();
    let busy_key = egui::Id::new(("graphic_upload_busy", project_session));
    let files_key = egui::Id::new(("graphic_upload_files", project_session));
    if let Some(files) = ui.data_mut(|data| data.get_temp::<Vec<UploadedFile>>(files_key)) {
        ui.data_mut(|data| data.remove::<Vec<UploadedFile>>(files_key));
        let failures = upload_graphic_files(project, files);
        if !failures.is_empty() {
            error = Some(format!(
                "Some graphics could not be imported:\n{}",
                failures.join("\n")
            ));
        }
    }
    let busy = ui.data_mut(|data| data.get_temp::<bool>(busy_key).unwrap_or(false));
    if ui
        .add_enabled(
            !busy,
            egui::Button::new(if busy {
                "Importing…"
            } else {
                "Upload Graphics"
            }),
        )
        .clicked()
    {
        start_graphic_upload(ui, busy_key, files_key);
    }
    let search_key = ui.id().with(("graphic_search", project_session));
    let mut search = ui.data_mut(|data| data.get_temp::<String>(search_key).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search graphics..."));
    ui.data_mut(|data| data.insert_temp(search_key, search.clone()));
    let search = search.to_lowercase();
    let assign_key = ui.id().with(("graphic_assign", project_session));
    let mut assigning = ui.data_mut(|data| data.get_temp::<GraphicAssetId>(assign_key));

    ui.strong("PICTURE GRAPHICS");
    let pictures = picture_graphics(project);
    let pictures = pictures
        .into_iter()
        .filter(|object| object_picker::matches_object(project, object, &search));
    thumbnail_grid(ui, "picture_grid", pictures, |ui, object| {
        let Object::PictureGraphic(picture) = object else {
            return;
        };
        let name = project.object_display_name(object);
        let name = if project.get_object_info(object).name.is_some() {
            format!("{name} {}", project.format_object_id(object.id().value()))
        } else {
            name
        };
        ui.push_id(("picture", object.id().value()), |ui| {
            match graphics_card(
                ui,
                &name,
                preview::picture_texture(ui.ctx(), picture, project.get_pool()),
                false,
            ) {
                Some(Action::Navigate) => go_to_picture_graphic(project, object.id()),
                Some(Action::Download) => {
                    error = preview::picture_png(picture, project.get_pool())
                        .and_then(|bytes| {
                            download(
                                &format!("PictureGraphic-{}.png", object.id().value()),
                                &bytes,
                            )
                        })
                        .err();
                }
                _ => {}
            }
        });
    });
    ui.separator();
    ui.strong("AVAILABLE");
    // Defer mutation so image bytes need not be cloned on every frame.
    let mut pending = None;
    let assets = project.graphic_library().assets.values().filter(|asset| {
        format!(
            "{} {} {} {}",
            asset.name,
            asset.description.as_deref().unwrap_or(""),
            asset.tags.join(" "),
            asset.original_file_name.as_deref().unwrap_or("")
        )
        .to_lowercase()
        .contains(&search)
    });
    thumbnail_grid(ui, "available_grid", assets, |ui, asset| {
        ui.push_id(("available", asset.id), |ui| {
            if let Some(action) = graphics_card(
                ui,
                &asset.name,
                preview::asset_texture(ui.ctx(), asset),
                true,
            ) {
                pending = Some((asset.id, action));
            }
        });
    });
    if let Some((id, action)) = pending {
        let result = match action {
            Action::Assign => {
                assigning = Some(id);
                object_picker::open(ui.ctx(), assign_key);
                Ok(())
            }
            Action::Create => {
                super::conversion_ui::open_for_asset(ui.ctx(), project, id, None);
                Ok(())
            }
            Action::Delete => GraphicService::delete_available_asset(project, id)
                .map_err(|error| error.to_string()),
            Action::Download => {
                let (name, bytes) = project.graphic_library().assets[&id].download();
                download(&name, bytes)
            }
            _ => Ok(()),
        };
        if let Err(value) = result {
            error = Some(value);
        }
    }
    if let Some(asset_id) = assigning {
        if project.graphic_library().assets.contains_key(&asset_id) {
            let selected = object_picker::object_window(
                ui.ctx(),
                assign_key,
                &format!("Assign '{}' to the chosen PictureGraphic, replacing its image and removing the upload from Available. This is undoable.", project.graphic_library().assets[&asset_id].name),
                project,
                None,
                false,
                |object| object.object_type() == ObjectType::PictureGraphic,
                |ui, choice| object_picker::review_object(ui, project, choice),
            )
            .and_then(|id| id.0);
            if let Some(id) = selected {
                super::conversion_ui::open_for_asset(ui.ctx(), project, asset_id, Some(id));
                assigning = None;
            }
            if !object_picker::is_open(ui.ctx(), assign_key) {
                assigning = None;
            }
        } else {
            assigning = None;
        }
    }
    ui.data_mut(|data| {
        data.remove::<GraphicAssetId>(assign_key);
        if let Some(id) = assigning {
            data.insert_temp(assign_key, id);
        }
    });
    let key = ui.id().with(("graphic_error", project_session));
    if let Some(error) = error {
        ui.data_mut(|data| data.insert_temp(key, error));
    }
    if let Some(error) = ui.data_mut(|data| data.get_temp::<String>(key)) {
        crate::ui_style::error_text(ui, &error);
        if ui.button("Dismiss graphic error").clicked() {
            ui.data_mut(|data| data.remove::<String>(key));
        }
    }
}

fn upload_graphic_files(project: &mut EditorProject, files: Vec<UploadedFile>) -> Vec<String> {
    files
        .into_iter()
        .filter_map(|file| {
            let (filename, bytes) = match file {
                Ok(file) => file,
                Err(error) => return Some(error),
            };
            GraphicService::upload(project, &filename, bytes)
                .err()
                .map(|error| format!("{filename}: {error}"))
        })
        .collect()
}

/// Shared picker for component references, retaining create-from-asset behavior.
pub fn select(
    ui: &mut egui::Ui,
    project: &mut EditorProject,
    value: &mut serde_json::Value,
    preview_selection: impl Fn(&mut EditorProject, u16) -> Result<(), String>,
) {
    #[derive(Clone)]
    enum Choice {
        Picture(ObjectId),
        Asset(GraphicAssetId),
    }
    let key = ui
        .next_auto_id()
        .with(("select_graphic", project.ui_session_id()));
    if ui
        .button(value.as_u64().map_or_else(
            || "Select PictureGraphic…".to_owned(),
            |id| format!("PictureGraphic {}", project.format_object_id(id as u16)),
        ))
        .clicked()
    {
        object_picker::open(ui.ctx(), key);
    }
    let choice = object_picker::window(
        ui.ctx(),
        key,
        "Select graphic",
        |ui, query| {
            ui.strong("PICTURE GRAPHICS");
            let picture = object_picker::objects(
                ui,
                project,
                query,
                value.as_u64().and_then(|id| ObjectId::new(id as u16).ok()),
                false,
                |object| object.object_type() == ObjectType::PictureGraphic,
            )
            .and_then(|id| id.0)
            .map(Choice::Picture);
            ui.separator();
            ui.strong("AVAILABLE — creates a PictureGraphic");
            picture.or_else(|| available_picker(ui, project, query).map(Choice::Asset))
        },
        |ui, choice| {
            ui.label(format!(
                "Current graphic: {}",
                object_picker::object_label(
                    project,
                    value.as_u64().and_then(|id| ObjectId::new(id as u16).ok())
                )
            ));
            let valid = match choice {
                Choice::Picture(id) => {
                    ui.label("Use this PictureGraphic for the graphic value.");
                    object_picker::review_object(ui, project, NullableObjectId(Some(*id)))
                }
                Choice::Asset(id) => {
                    ui.label("Create a new PictureGraphic from this upload and use it for the graphic value. The upload will be removed from Available. Creation is undoable.");
                    return review_asset(ui, project, *id);
                }
            };
            valid
                && object_picker::impact::review_change(ui, project, |candidate| {
                    let Choice::Picture(id) = choice else {
                        unreachable!()
                    };
                    preview_selection(candidate, id.value())
                })
        },
    );
    match choice {
        Some(Choice::Picture(id)) => *value = serde_json::json!(id.value()),
        Some(Choice::Asset(asset)) => {
            ui.data_mut(|data| data.insert_temp(key.with("conversion_asset"), asset));
            super::conversion_ui::open_asset_options(
                ui.ctx(),
                project,
                key,
                asset,
                super::PictureGraphicFormat::EightBit,
            );
        }
        None => {}
    }
    if let Some(options) = super::conversion_ui::take_asset_options(ui.ctx(), key) {
        if let Some(asset) =
            ui.data_mut(|data| data.remove_temp::<GraphicAssetId>(key.with("conversion_asset")))
        {
            let result = create_and_select(project, asset, &options, &preview_selection);
            match result {
                Ok(id) => *value = serde_json::json!(id),
                Err(error) => ui.data_mut(|data| data.insert_temp(key.with("error"), error)),
            }
        }
    }
    if let Some(error) = ui.data_mut(|data| data.get_temp::<String>(key.with("error"))) {
        crate::ui_style::error_text(ui, &error);
        if ui.button("Dismiss conversion error").clicked() {
            ui.data_mut(|data| data.remove::<String>(key.with("error")));
        }
    }
}

fn create_and_select(
    project: &mut EditorProject,
    asset: GraphicAssetId,
    options: &super::conversion::ConversionOptions,
    select: impl FnOnce(&mut EditorProject, u16) -> Result<(), String>,
) -> Result<u16, String> {
    let mut created = None;
    project.edit_batch(
        "Create and select PictureGraphic",
        |candidate| {
            let id = GraphicService::create_picture_graphic_from_asset_with_options(
                candidate, asset, options,
            )
            .map_err(|error| error.to_string())?;
            select(candidate, id)?;
            created = Some(id);
            Ok(())
        },
        false,
    )?;
    Ok(created.expect("successful conversion creates a PictureGraphic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creating_and_selecting_is_atomic_on_failure_and_undo() {
        let mut pool = ag_iso_stack::object_pool::ObjectPool::default();
        let mut pointer = crate::default_object(ObjectType::ObjectPointer);
        pointer.mut_id().set_value(100).unwrap();
        pool.add(pointer);
        let mut project = EditorProject::from(pool);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let asset = GraphicService::upload(&mut project, "test.png", bytes.into_inner()).unwrap();
        let options = super::super::conversion::ConversionOptions::suggested(
            super::super::PictureGraphicFormat::Monochrome,
        );
        let before = project.save_project().unwrap();
        assert!(create_and_select(&mut project, asset, &options, |_, _| {
            Err("Reference update failed".into())
        })
        .is_err());
        assert_eq!(project.save_project().unwrap(), before);
        let id = create_and_select(&mut project, asset, &options, |candidate, id| {
            let mut transaction = crate::operations::OperationTransaction::new(None);
            transaction.add_operation(crate::operations::Operation::SetObjectReference {
                object: crate::operations::ObjectTarget::id(100),
                property: crate::object_properties::PropertyKey::Value,
                reference: Some(crate::operations::ObjectTarget::id(id)),
            });
            candidate
                .execute_transaction(transaction)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .unwrap();
        assert!(!project.graphic_library().assets.contains_key(&asset));
        assert!(project
            .get_pool()
            .object_by_id(ObjectId::new(id).unwrap())
            .is_some());
        assert!(
            matches!(project.get_pool().object_by_id(ObjectId::new(100).unwrap()),
            Some(Object::ObjectPointer(pointer)) if pointer.value.0 == ObjectId::new(id).ok())
        );
        assert!(project.undo_operation());
        assert_eq!(project.save_project().unwrap(), before);
        assert!(project.redo_operation());
        assert!(!project.graphic_library().assets.contains_key(&asset));
    }
}
