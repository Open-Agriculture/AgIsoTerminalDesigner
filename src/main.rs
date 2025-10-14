//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::NullableObjectId;
use ag_iso_stack::object_pool::ObjectId;
use ag_iso_stack::object_pool::ObjectPool;
use ag_iso_stack::object_pool::ObjectType;
use ag_iso_terminal_designer::ConfigurableObject;
use ag_iso_terminal_designer::EditorProject;
use ag_iso_terminal_designer::InteractiveMaskRenderer;
use ag_iso_terminal_designer::RenderableObject;
use eframe::egui;
use std::future::Future;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;

const OBJECT_HIERARCHY_ID: &str = "object_hierarchy_ui";

enum FileDialogReason {
    LoadPool,
    LoadProject,
    OpenImagePictureGraphics(ObjectId),
}

pub struct DesignerApp {
    project: Option<EditorProject>,
    file_dialog_reason: Option<FileDialogReason>,
    file_channel: (Sender<Vec<u8>>, Receiver<Vec<u8>>),
    show_development_popup: bool,
    new_object_dialog: Option<(ObjectType, String)>,
    apply_smart_naming_on_import: bool,
}

impl DesignerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();

        // TODO: Create font files and load them here
        //// Install ISO 8859-1 (ISO Latin 1) font
        // fonts.font_data.insert(
        //     "iso_latin_1".to_owned(),
        //     egui::FontData::from_static(include_bytes!("assets/fonts/iso-latin1.ttf")),
        // );
        // fonts
        //     .families
        //     .get_mut(&egui::FontFamily::Name("ISO Latin 1".into()))
        //     .unwrap()
        //     .insert(0, "iso_latin_1".to_owned());

        // // Install ISO 8859-15 (ISO Latin 9) font
        // fonts.font_data.insert(
        //     "iso_latin_9".to_owned(),
        //     egui::FontData::from_static(include_bytes!("assets/fonts/iso-latin9.ttf")),
        // );
        // fonts
        //     .families
        //     .get_mut(&egui::FontFamily::Name("ISO Latin 9".into()))
        //     .unwrap()
        //     .insert(0, "iso_latin_9".to_owned());

        // // Install ISO 8859-2 (ISO Latin 2) font
        // fonts.font_data.insert(
        //     "iso_latin_2".to_owned(),
        //     egui::FontData::from_static(include_bytes!("assets/fonts/iso-latin2.ttf")),
        // );
        // fonts
        //     .families
        //     .get_mut(&egui::FontFamily::Name("ISO Latin 2".into()))
        //     .unwrap()
        //     .insert(0, "iso_latin_2".to_owned());

        // // Install ISO 8859-4 (ISO Latin 4) font
        // fonts.font_data.insert(
        //     "iso_latin_4".to_owned(),
        //     egui::FontData::from_static(include_bytes!("assets/fonts/iso-latin4.ttf")),
        // );
        // fonts
        //     .families
        //     .get_mut(&egui::FontFamily::Name("ISO Latin 4".into()))
        //     .unwrap()
        //     .insert(0, "iso_latin_4".to_owned());

        // // Install ISO 8859-5 (Cyrillic) font
        // fonts.font_data.insert(
        //     "iso_cyrillic".to_owned(),
        //     egui::FontData::from_static(include_bytes!("assets/fonts/iso-cyrillic.ttf")),
        // );
        // fonts
        //     .families
        //     .get_mut(&egui::FontFamily::Name("ISO Cyrillic".into()))
        //     .unwrap()
        //     .insert(0, "iso_cyrillic".to_owned());

        // // Install ISO 8859-7 (Greek) font
        // fonts.font_data.insert(
        //     "iso_greek".to_owned(),
        //     egui::FontData::from_static(include_bytes!("assets/fonts/iso-greek.ttf")),
        // );
        // fonts
        //     .families
        //     .get_mut(&egui::FontFamily::Name("ISO Greek".into()))
        //     .unwrap()
        //     .insert(0, "iso_greek".to_owned());

        Self {
            project: None,
            file_dialog_reason: None,
            file_channel: std::sync::mpsc::channel(),
            show_development_popup: true,
            new_object_dialog: None,
            apply_smart_naming_on_import: true, // Default to true for better UX
        }
    }
}

impl DesignerApp {
    /// Open a file dialog
    fn open_file_dialog(&mut self, reason: FileDialogReason, ctx: &egui::Context) {
        self.file_dialog_reason = Some(reason);

        let sender = self.file_channel.0.clone();
        let task = rfd::AsyncFileDialog::new().pick_file();
        let ctx = ctx.clone();
        execute(async move {
            let file = task.await;
            if let Some(file) = file {
                let content = file.read().await;
                let _ = sender.send(content);
            }
            ctx.request_repaint();
        });
    }

    /// Handle a file loaded in the file dialog
    fn handle_file_loaded(&mut self) {
        if let Ok(content) = self.file_channel.1.try_recv() {
            match self.file_dialog_reason {
                Some(FileDialogReason::LoadPool) => {
                    let project = EditorProject::from(ObjectPool::from_iop(content));
                    // Apply smart naming to all objects that don't have custom names (if enabled)
                    if self.apply_smart_naming_on_import {
                        project.apply_smart_naming_to_all_objects();
                    }
                    self.project = Some(project);
                }
                Some(FileDialogReason::LoadProject) => {
                    match EditorProject::load_project(content) {
                        Ok(project) => {
                            self.project = Some(project);
                        }
                        Err(e) => {
                            log::error!("Failed to load project: {}", e);
                            // TODO: Show error dialog
                        }
                    }
                }
                Some(FileDialogReason::OpenImagePictureGraphics(id)) => {
                    if let Some(pool) = &mut self.project {
                        if let Some(obj) = pool.get_mut_pool().borrow_mut().object_mut_by_id(id) {
                            match obj {
                                Object::PictureGraphic(o) => {
                                    // o.load_image(content);
                                }
                                _ => (),
                            }
                        }
                    }
                }
                _ => (),
            }
        }
    }

    /// Open a file dialog to save a pool file
    fn save_pool(&mut self) {
        if let Some(pool) = &self.project {
            let task = rfd::AsyncFileDialog::new()
                .set_file_name("object_pool.iop")
                .save_file();
            let contents = pool.get_pool().as_iop();
            execute(async move {
                let file = task.await;
                if let Some(file) = file {
                    _ = file.write(&contents).await;
                }
            });
        }
    }

    /// Open a file dialog to save a project file
    fn save_project(&mut self) {
        if let Some(project) = &self.project {
            match project.save_project() {
                Ok(contents) => {
                    let task = rfd::AsyncFileDialog::new()
                        .set_file_name("project.aitp")
                        .add_filter("AgIsoTerminal Project", &["aitp"])
                        .save_file();
                    execute(async move {
                        let file = task.await;
                        if let Some(file) = file {
                            _ = file.write(&contents).await;
                        }
                    });
                }
                Err(e) => {
                    log::error!("Failed to save project: {}", e);
                    // TODO: Show error dialog
                }
            }
        }
    }
}

fn render_selectable_object(ui: &mut egui::Ui, object: &Object, project: &EditorProject) {
    let this_ui_id = ui.id();
    let object_info = project.get_object_info(object);

    let renaming_object = project.get_renaming_object();
    if renaming_object
        .clone()
        .is_some_and(|(ui_id, id, _)| id == object.id() && ui_id == this_ui_id)
    {
        let mut name = renaming_object.unwrap().2;
        let response = ui.text_edit_singleline(&mut name);
        project.set_renaming_object(this_ui_id, object.id(), name); // Update the name in the project
        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if response.lost_focus() {
            project.finish_renaming_object(!cancelled);
        } else if !response.has_focus() {
            // We need to focus the text edit when we start renaming
            response.request_focus();
        }
    } else {
        let is_selected = project.get_selected() == object.id().into();
        let label_text = format!(
            "{}: {}",
            u16::from(object.id()),
            object_info.get_name(object)
        );
        let response = ui.selectable_label(is_selected, label_text);

        if response.clicked() {
            project
                .get_mut_selected()
                .replace(NullableObjectId(Some(object.id())));
        }
        if response.double_clicked() {
            project.set_renaming_object(this_ui_id, object.id(), object_info.get_name(object));
        }

        response.context_menu(|ui| {
            if ui.button("Rename").on_hover_text("Rename object").clicked() {
                project.set_renaming_object(this_ui_id, object.id(), object_info.get_name(object));
                ui.close();
            }
            if ui.button("Delete").on_hover_text("Delete object").clicked() {
                project.get_mut_pool().borrow_mut().remove(object.id());
                ui.close();
            }
        });
    }
}

fn render_object_hierarchy(
    ui: &mut egui::Ui,
    parent_id: egui::Id,
    object: &Object,
    project: &EditorProject,
) {
    let refs = object.referenced_objects();
    if refs.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().indent);
            render_selectable_object(ui, object, project);
        });
    } else {
        let id = parent_id.with(project.get_object_info(object).get_unique_id());
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                render_selectable_object(ui, object, project);
            })
            .body(|ui| {
                for (idx, obj_id) in refs.iter().enumerate() {
                    match project.get_pool().object_by_id(*obj_id) {
                        Some(obj) => {
                            render_object_hierarchy(ui, id.with(idx), obj, project);
                        }
                        None => {
                            ui.colored_label(
                                egui::Color32::RED,
                                format!("Missing object: {:?}", id),
                            );
                        }
                    }
                }
            });
    }
}

fn update_object_hierarchy_headers(
    ctx: &egui::Context,
    parent_id: egui::Id,
    object: &Object,
    pool: &ObjectPool,
    new_selected: NullableObjectId,
) -> bool {
    let mut is_selected_or_descendant = new_selected == object.id().into();

    let refs = object.referenced_objects();
    if !refs.is_empty() {
        let id = parent_id.with(object.id().value());

        // Update in a depth-first manner
        for obj_id in refs {
            if let Some(obj) = pool.object_by_id(obj_id) {
                is_selected_or_descendant |=
                    update_object_hierarchy_headers(ctx, id, obj, pool, new_selected);
            }
        }

        if is_selected_or_descendant {
            if let Some(mut state) = egui::collapsing_header::CollapsingState::load(ctx, id) {
                if !state.is_open() {
                    state.set_open(true);
                    state.store(ctx);
                }
            }
        }
    }

    is_selected_or_descendant
}

impl eframe::App for DesignerApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.style_mut(|style| {
            style.interaction.selectable_labels = false;
        });

        // Handle file dialog
        self.handle_file_loaded();

        if self.show_development_popup {
            egui::Window::new("🚧 Under Active Development")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(10.0);
                    ui.label("This application is still under active development. Some features may be missing or broken. We appreciate your patience and feedback!");

                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label("If you encounter issues, please report them at:");
                        ui.hyperlink("https://github.com/Open-Agriculture/AgIsoTerminalDesigner/issues");
                    });

                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.add_space(ui.available_width() - 60.0);
                        if ui.button("OK").clicked() {
                            self.show_development_popup = false;
                        }
                    });
                });
            return;
        }

        // Show new object name dialog
        if let Some((object_type, mut name)) = self.new_object_dialog.clone() {
            let mut should_create = false;
            let mut should_cancel = false;

            egui::Window::new(format!("New {:?}", object_type))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Enter a name for the new object:");
                    ui.add_space(10.0);

                    let response = ui.text_edit_singleline(&mut name);

                    // Auto-focus the text field
                    if !response.has_focus() && !response.lost_focus() {
                        response.request_focus();
                    }

                    // Check for Enter key
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        should_create = true;
                    }

                    // Check for Escape key
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        should_cancel = true;
                    }

                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() || should_create {
                            should_create = true;
                        }
                        if ui.button("Cancel").clicked() || should_cancel {
                            should_cancel = true;
                        }
                    });
                });

            if should_create {
                // Create the object with the given name
                if let Some(pool) = &mut self.project {
                    let mut new_obj = ag_iso_terminal_designer::default_object(object_type);

                    // Allocate a new ID efficiently
                    let id = pool.allocate_object_id();
                    new_obj.mut_id().set_value(id.value()).ok();

                    // Add object to pool
                    pool.get_mut_pool().borrow_mut().add(new_obj.clone());

                    // Set the custom name
                    let mut object_info = pool.object_info.borrow_mut();
                    let info = object_info
                        .entry(new_obj.id())
                        .or_insert_with(|| ag_iso_terminal_designer::ObjectInfo::new(&new_obj));
                    info.set_name(name);
                    drop(object_info);

                    // Select the new object
                    pool.get_mut_selected()
                        .replace(NullableObjectId::new(id.value()));
                }
                self.new_object_dialog = None;
            } else if should_cancel {
                self.new_object_dialog = None;
            } else {
                // Update the name in the dialog state
                self.new_object_dialog = Some((object_type, name));
            }
        }

        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                egui::widgets::global_theme_preference_buttons(ui);
                ui.separator();

                // Undo/redo buttons
                if let Some(pool) = &mut self.project {
                    let undo_shortcut =
                        egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Z);
                    let redo_shortcut =
                        egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Y);

                    if ui
                        .add_enabled(
                            pool.undo_available(),
                            egui::widgets::Button::new("\u{2BAA}"),
                        )
                        .on_hover_text(format!("Undo ({})", ctx.format_shortcut(&undo_shortcut)))
                        .clicked()
                        || ctx.input_mut(|i| i.consume_shortcut(&undo_shortcut))
                    {
                        pool.undo();
                    }
                    if ui
                        .add_enabled(
                            pool.redo_available(),
                            egui::widgets::Button::new("\u{2BAB}"),
                        )
                        .on_hover_text(format!("Redo ({})", ctx.format_shortcut(&redo_shortcut)))
                        .clicked()
                        || ctx.input_mut(|i| i.consume_shortcut(&redo_shortcut))
                    {
                        pool.redo();
                    }
                    ui.separator();
                }

                ui.menu_button("File", |ui| {
                    ui.label("Project Files");
                    if ui.button("Open Project (.aitp)").clicked() {
                        self.open_file_dialog(FileDialogReason::LoadProject, ctx);
                        ui.close();
                    }
                    if self.project.is_some() && ui.button("Save Project (.aitp)").clicked() {
                        self.save_project();
                        ui.close();
                    }

                    ui.separator();
                    ui.label("ISOBUS Files");

                    if ui.button("Import IOP (.iop)").clicked() {
                        self.open_file_dialog(FileDialogReason::LoadPool, ctx);
                        ui.close();
                    }

                    ui.checkbox(
                        &mut self.apply_smart_naming_on_import,
                        "Apply smart naming on import",
                    )
                    .on_hover_text(
                        "Automatically apply smart naming to objects when importing IOP files",
                    );
                    if self.project.is_some() && ui.button("Export IOP (.iop)").clicked() {
                        self.save_pool();
                        ui.close();
                    }
                });

                if self.project.is_some() {
                    // Add a new object
                    ui.menu_button("Add object", |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for object_type in ObjectType::values() {
                                if ui.button(format!("{:?}", object_type)).clicked() {
                                    // Generate smart default name
                                    let pool = self.project.as_ref().unwrap();
                                    let default_name =
                                        pool.generate_smart_name_for_new_object(object_type);
                                    self.new_object_dialog = Some((object_type, default_name));
                                    ui.close();
                                }
                            }
                        });
                    });
                }

                if let Some(pool) = &mut self.project {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Slider::new(&mut pool.mask_size, 100..=2000)
                                .text("Virtual Mask size"),
                        );
                    });
                }
            });
        });

        if let Some(pool) = &mut self.project {
            // Set forward and backward navigation shortcuts to mouse buttons
            if ctx.input(|i| i.pointer.button_released(egui::PointerButton::Extra1)) {
                pool.set_previous_selected();
            } else if ctx.input(|i| i.pointer.button_released(egui::PointerButton::Extra2)) {
                pool.set_next_selected();
            }

            // Object selector panel
            egui::SidePanel::left("left_panel").show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    if let Some(working_set) = pool.get_pool().working_set_object() {
                        render_object_hierarchy(
                            ui,
                            egui::Id::new(OBJECT_HIERARCHY_ID),
                            &Object::WorkingSet(working_set.clone()),
                            pool,
                        );
                    } else {
                        ui.colored_label(
                            egui::Color32::RED,
                            "No working set, please add a new working set...",
                        );
                    }
                    let auxiliary_objects = pool.get_pool().objects_by_types(&[
                        ObjectType::AuxiliaryFunctionType1,
                        ObjectType::AuxiliaryInputType1,
                        ObjectType::AuxiliaryFunctionType2,
                        ObjectType::AuxiliaryInputType2,
                    ]);
                    if !auxiliary_objects.is_empty() {
                        ui.separator();
                        for object in auxiliary_objects {
                            render_selectable_object(ui, object, pool);
                        }
                    }
                    ui.separator();

                    // Filter objects in the pool by name
                    let filter_id = ui.id().with("filter_text");
                    let mut filter_text = ui
                        .data(|data| data.get_temp::<String>(filter_id))
                        .unwrap_or_default();

                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(ui.spacing().scroll.bar_width);
                            ui.menu_button("\u{2195}", |ui| {
                                if ui.button("Sort by name").clicked() {
                                    let pool_copy = pool.clone();
                                    pool.sort_objects_by(|a, b| {
                                        pool_copy
                                            .get_object_info(a)
                                            .get_name(a)
                                            .cmp(&pool_copy.get_object_info(b).get_name(b))
                                    });
                                    ui.close();
                                }
                                if ui.button("Sort by id").clicked() {
                                    pool.sort_objects_by(|a, b| {
                                        u16::from(a.id()).cmp(&u16::from(b.id()))
                                    });
                                    ui.close();
                                }
                            })
                            .response
                            .on_hover_text("Sort objects");

                            let filter_shortcut =
                                egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::F);

                            let response = ui
                                .add(
                                    egui::TextEdit::singleline(&mut filter_text)
                                        .hint_text("Filter object by name...")
                                        .desired_width(ui.available_width()),
                                )
                                .on_hover_text(format!(
                                    "Search shortcut ({})",
                                    ctx.format_shortcut(&filter_shortcut)
                                ));
                            if response.changed() {
                                ui.data_mut(|data| {
                                    data.insert_temp(filter_id, filter_text.clone())
                                });
                            } else if ctx.input_mut(|i| i.consume_shortcut(&filter_shortcut)) {
                                response.request_focus();
                            }
                        });
                    });

                    let filter_text = filter_text.to_lowercase();
                    for object in pool.get_pool().objects() {
                        if filter_text.is_empty()
                            || pool
                                .get_object_info(object)
                                .get_name(object)
                                .to_lowercase()
                                .contains(&filter_text)
                        {
                            render_selectable_object(ui, object, pool);
                        }
                    }

                    ui.allocate_space(ui.available_size());
                });
            });

            // Main panel
            egui::CentralPanel::default().show(ctx, |ui| {
                if pool
                    .get_pool()
                    .objects_by_type(ObjectType::DataMask)
                    .is_empty()
                {
                    ui.colored_label(
                        egui::Color32::RED,
                        "Missing data masks, please load a pool file or add a new mask...",
                    );
                } else {
                    match pool.get_pool().working_set_object() {
                        Some(mask) => match pool.get_pool().object_by_id(mask.active_mask) {
                            Some(obj) => {
                                let selected_ref = pool.get_mut_selected();

                                egui::ScrollArea::both().show(ui, |ui| {
                                    ui.add_sized(
                                        [pool.mask_size as f32, pool.mask_size as f32],
                                        InteractiveMaskRenderer {
                                            object: obj,
                                            pool: pool.get_pool(),
                                            selected_callback: Box::new(move |object_id| {
                                                *selected_ref.borrow_mut() =
                                                    NullableObjectId(Some(object_id));
                                            }),
                                        },
                                    );
                                });
                            }
                            None => {
                                ui.colored_label(
                                    egui::Color32::RED,
                                    format!("Missing data mask: {:?}", mask),
                                );
                            }
                        },
                        None => {
                            ui.colored_label(
                                egui::Color32::RED,
                                "No working sets, please add a new working set...",
                            );
                        }
                    }
                }
            });

            // Parameters panel
            egui::SidePanel::right("right_panel").show(ctx, |ui: &mut egui::Ui| {
                if let Some(id) = pool.get_selected().into() {
                    if let Some(obj) = pool.get_mut_pool().borrow_mut().object_mut_by_id(id) {
                        // Display editable object name as header
                        ui.horizontal(|ui| {
                            ui.label("Name:");

                            let object_info = pool.get_object_info(obj);
                            let mut name = object_info.get_name(obj);
                            let response = ui.text_edit_singleline(&mut name);

                            if response.changed() {
                                let mut object_info_map = pool.object_info.borrow_mut();
                                if let Some(info) = object_info_map.get_mut(&obj.id()) {
                                    info.set_name(name);
                                }
                            }
                        });
                        ui.separator();

                        obj.render_parameters(ui, pool);
                        let (width, height) = pool.get_pool().content_size(obj);
                        ui.separator();
                        let desired_size = egui::Vec2::new(width as f32, height as f32);
                        ui.allocate_ui(desired_size, |ui| {
                            obj.render(ui, pool.get_pool(), Point::default());
                        });
                    } else {
                        ui.colored_label(
                            egui::Color32::RED,
                            format!("Selected object not found: {}", u16::from(id)),
                        );
                    }
                }
                ui.allocate_space(ui.available_size());
            });

            if pool.update_pool() {
                ctx.request_repaint();
            }
            if pool.update_selected() {
                // Make sure all collapsing headers for the selected object are open
                if let Some(working_set) = pool.get_pool().working_set_object() {
                    update_object_hierarchy_headers(
                        ctx,
                        egui::Id::new(OBJECT_HIERARCHY_ID),
                        &Object::WorkingSet(working_set.clone()),
                        pool.get_pool(),
                        pool.get_selected(),
                    );
                }
                ctx.request_repaint();
            }
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("No object pool loaded, please load a pool file...");
            });
        }
    }
}

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 440.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(&include_bytes!("../assets/icon-256.png")[..])
                    .expect("Failed to load icon"),
            ),
        ..Default::default()
    };

    eframe::run_native(
        "AgIsoTerminalDesigner",
        native_options,
        Box::new(|cc| Ok(Box::new(DesignerApp::new(cc)))),
    )
    .ok();
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    let web_options = eframe::WebOptions::default();

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("terminal_designer_canvas_id")
            .expect("Failed to find terminal_designer_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("terminal_designer_canvas_id was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(DesignerApp::new(cc)))),
            )
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn execute<F: Future<Output = ()> + Send + 'static>(f: F) {
    // this is stupid... use any executor of your choice instead
    std::thread::spawn(move || futures::executor::block_on(f));
}

#[cfg(target_arch = "wasm32")]
fn execute<F: Future<Output = ()> + 'static>(f: F) {
    wasm_bindgen_futures::spawn_local(f);
}
