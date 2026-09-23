use super::{integration::GraphicService, preview, GraphicAssetId};
use crate::EditorProject;
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

fn checkerboard(painter: &egui::Painter, rect: egui::Rect) {
    const TILE: f32 = 6.0;
    for row in 0..(rect.height() / TILE).ceil() as usize {
        for column in 0..(rect.width() / TILE).ceil() as usize {
            let tile = egui::Rect::from_min_size(
                rect.min + egui::vec2(column as f32 * TILE, row as f32 * TILE),
                egui::vec2(TILE, TILE),
            )
            .intersect(rect);
            let shade = if (row + column) % 2 == 0 { 190 } else { 235 };
            painter.rect_filled(tile, 0.0, egui::Color32::from_gray(shade));
        }
    }
}

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
        if let Some(texture) = texture {
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            ui.painter().image(
                texture.id(),
                egui::Rect::from_center_size(rect.center(), size * scale),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
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

/// Choose a waiting upload without mutating the project during inspector rendering.
pub fn available_picker(ui: &mut egui::Ui, project: &EditorProject) -> Option<GraphicAssetId> {
    let mut selected = None;
    if project.graphic_library().assets.is_empty() {
        ui.label("No available graphics. Upload one in the Graphics panel.");
    }
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
            for asset in project.graphic_library().assets.values() {
                ui.push_id(asset.id, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(texture) = preview::asset_texture(ui.ctx(), asset) {
                            picker_preview(ui, &texture);
                        }
                        if ui.selectable_label(false, &asset.name).clicked() {
                            selected = Some(asset.id);
                        }
                    });
                });
            }
        });
    selected
}

fn picker_preview(ui: &mut egui::Ui, texture: &egui::TextureHandle) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
    checkerboard(ui.painter(), rect);
    egui::Image::new(texture).paint_at(ui, rect);
}

fn picture_picker(ui: &mut egui::Ui, project: &EditorProject) -> Option<ObjectId> {
    let mut selected = None;
    let pictures = picture_graphics(project);
    if pictures.is_empty() {
        ui.label("No PictureGraphics in this project.");
    }
    for object in pictures {
        let Object::PictureGraphic(picture) = object else {
            continue;
        };
        ui.push_id(object.id().value(), |ui| {
            ui.horizontal(|ui| {
                if let Some(texture) =
                    preview::picture_texture(ui.ctx(), picture, project.get_pool())
                {
                    picker_preview(ui, &texture);
                }
                if ui
                    .selectable_label(
                        false,
                        format!(
                            "{}  #{}",
                            project.get_object_info(object).get_name(object),
                            object.id().value()
                        ),
                    )
                    .clicked()
                {
                    selected = Some(object.id());
                }
            });
        });
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
    let pictures = pictures.into_iter().filter(|object| {
        format!(
            "{} #{}",
            project.get_object_info(object).get_name(object),
            object.id().value()
        )
        .to_lowercase()
        .contains(&search)
    });
    thumbnail_grid(ui, "picture_grid", pictures, |ui, object| {
        let Object::PictureGraphic(picture) = object else {
            return;
        };
        let name = format!(
            "{} #{}",
            project.get_object_info(object).get_name(object),
            object.id().value()
        );
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
            let mut open = true;
            let mut selected = None;
            egui::Window::new("Assign to PictureGraphic")
                .id(assign_key)
                .open(&mut open)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            selected = picture_picker(ui, project);
                        });
                });
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
            if !open {
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
pub fn select(ui: &mut egui::Ui, project: &mut EditorProject, value: &mut serde_json::Value) {
    egui::ComboBox::from_id_salt(ui.next_auto_id())
        .selected_text(value.as_u64().map_or_else(
            || "Select PictureGraphic".to_owned(),
            |id| format!("PictureGraphic {id}"),
        ))
        .show_ui(ui, |ui| {
            if let Some(id) = picture_picker(ui, project) {
                *value = serde_json::json!(id.value());
            }
            ui.separator();
            ui.strong("AVAILABLE");
            let mut selected = None;
            for asset in project.graphic_library().assets.values() {
                if ui.selectable_label(false, &asset.name).clicked() {
                    selected = Some(asset.id);
                }
            }
            if let Some(id) = selected {
                if let Ok(id) = GraphicService::create_picture_graphic_from_asset(project, id) {
                    *value = serde_json::json!(id);
                }
            }
        });
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
