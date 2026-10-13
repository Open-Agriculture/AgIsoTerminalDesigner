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
    ui.vertical(|ui| {
        ui.set_width(CARD_WIDTH);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(CARD_WIDTH, 86.0), egui::Sense::click());
        checkerboard(ui.painter(), rect);
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
                egui::Color32::from_gray(60),
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
                && object_picker::impact::review_change(ui, project, |candidate| {
                    GraphicService::convert_asset_into_picture_graphic(
                        candidate,
                        *asset,
                        target.value(),
                    )
                    .map_err(|error| error.to_string())
                })
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

pub fn show(ui: &mut egui::Ui, project: &mut EditorProject) {
    ui.heading("Graphics");
    let mut error = None;
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button("Upload Graphic").clicked() {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            let result = std::fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    GraphicService::upload(project, &path.to_string_lossy(), bytes)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                });
            error = result.err();
        }
    }
    let search_key = ui.id().with("graphic_search");
    let mut search = ui.data_mut(|data| data.get_temp::<String>(search_key).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search graphics..."));
    ui.data_mut(|data| data.insert_temp(search_key, search.clone()));
    let search = search.to_lowercase();
    let assign_key = ui.id().with("graphic_assign");
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
            Action::Create => GraphicService::create_picture_graphic_from_asset(project, id)
                .map(|id| go_to_picture_graphic(project, ObjectId::new(id).unwrap()))
                .map_err(|error| error.to_string()),
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
                |ui, choice| object_picker::impact::review_change(ui, project, |candidate| {
                    let target = choice.0.ok_or("Choose a PictureGraphic")?;
                    GraphicService::convert_asset_into_picture_graphic(candidate, asset_id, target.value()).map_err(|error| error.to_string())
                }),
            )
            .and_then(|id| id.0);
            if let Some(id) = selected {
                match GraphicService::convert_asset_into_picture_graphic(
                    project,
                    asset_id,
                    id.value(),
                ) {
                    Ok(()) => {
                        go_to_picture_graphic(project, id);
                        assigning = None;
                    }
                    Err(value) => error = Some(value.to_string()),
                }
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
    let key = ui.next_auto_id().with("select_graphic");
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
                    review_asset(ui, project, *id)
                }
            };
            valid
                && object_picker::impact::review_change(ui, project, |candidate| {
                    let id = match choice {
                        Choice::Picture(id) => id.value(),
                        Choice::Asset(asset) => {
                            GraphicService::create_picture_graphic_from_asset(candidate, *asset)
                                .map_err(|error| error.to_string())?
                        }
                    };
                    preview_selection(candidate, id)
                })
        },
    );
    match choice {
        Some(Choice::Picture(id)) => *value = serde_json::json!(id.value()),
        Some(Choice::Asset(asset)) => {
            if let Ok(id) = GraphicService::create_picture_graphic_from_asset(project, asset) {
                *value = serde_json::json!(id);
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_grid_reflows_when_panel_narrows() {
        let context = egui::Context::default();
        let mut positions = Vec::new();
        for width in [360.0, 160.0] {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    positions.clear();
                    egui::CentralPanel::default().show(context, |ui| {
                        thumbnail_grid(ui, "resize_test", 0..3, |ui, index| {
                            let response = ui.push_id(index, |ui| {
                                graphics_card(ui, "Graphic", None, index % 2 == 0);
                            });
                            positions.push(response.response.rect);
                        });
                    });
                },
            );
            assert_eq!(positions.len(), 3);
            if width > 300.0 {
                assert_eq!(positions[0].top(), positions[2].top());
                assert!(positions[1].left() >= positions[0].right());
            } else {
                assert!(positions[1].top() >= positions[0].bottom());
                assert!(positions[2].top() >= positions[1].bottom());
                assert!(positions.iter().all(|rect| rect.right() <= width));
            }
        }
    }
}
