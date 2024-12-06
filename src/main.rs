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
use ag_iso_terminal_designer::RenderableObject;
use eframe::egui;
use std::future::Future;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;

const OBJECT_HIERARCHY_ID: &str = "object_hierarchy_ui";

enum FileDialogReason {
    LoadPool,
    OpenImagePictureGraphics(ObjectId),
}

pub struct DesignerApp {
    project: Option<EditorProject>,
    file_dialog_reason: Option<FileDialogReason>,
    file_channel: (Sender<Vec<u8>>, Receiver<Vec<u8>>),
}

impl Default for DesignerApp {
    fn default() -> Self {
        DesignerApp {
            project: None,
            file_dialog_reason: None,
            file_channel: std::sync::mpsc::channel(),
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
                    self.project = Some(EditorProject::from(ObjectPool::from_iop(content)));
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
}

struct ObjectWrapper<'a> {
    object: &'a Object,
    pool: &'a ObjectPool,
}

impl<'a> ObjectWrapper<'a> {
    fn new(object: &'a Object, pool: &'a ObjectPool) -> Self {
        ObjectWrapper { object, pool }
    }
}

impl<'a> egui::Widget for ObjectWrapper<'a> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        match self.object {
            Object::DataMask(o) => o.render(ui, self.pool, Point::default()),
            _ => (),
        }
        ui.label("")
    }
}

fn render_selectable_object(ui: &mut egui::Ui, object: &Object, project: &EditorProject) {
    let name: String = format!("{:?}: {:?}", u16::from(object.id()), object.object_type());
    let is_selected = project.get_selected() == object.id().into();
    let response = ui.selectable_label(is_selected, name.clone());

    if response.clicked() {
        project
            .get_mut_selected()
            .replace(NullableObjectId(Some(object.id())));
    }
    response.context_menu(|ui| {
        if ui.button("Delete").on_hover_text("Delete object").clicked() {
            project.get_mut_pool().borrow_mut().remove(object.id());
        }
    });
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
        let id = parent_id.with(object.id().value());
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
                    if ui.button("Load pool").clicked() {
                        self.open_file_dialog(FileDialogReason::LoadPool, ctx);
                        ui.close_menu();
                    }
                    if self.project.is_some() && ui.button("Save pool").clicked() {
                        self.save_pool();
                        ui.close_menu();
                    }
                });

                if self.project.is_some() {
                    // Add a new object
                    ui.menu_button("Add object", |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for object_type in ObjectType::values() {
                                if ui.button(format!("{:?}", object_type)).clicked() {
                                    let mut new_obj =
                                        ag_iso_terminal_designer::default_object(object_type);
                                    let pool = self.project.as_mut().unwrap();

                                    // Find first available id
                                    let mut id = 0;
                                    while pool
                                        .get_pool()
                                        .object_by_id(ObjectId::new(id).unwrap_or_default())
                                        .is_some()
                                    {
                                        id += 1;
                                    }
                                    new_obj.mut_id().set_value(id).ok();

                                    // Add object to pool and select it
                                    pool.get_mut_pool().borrow_mut().add(new_obj);
                                    pool.get_mut_selected().replace(NullableObjectId::new(id));
                                    ui.close_menu();
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
                    for object in pool.get_pool().objects() {
                        render_selectable_object(ui, object, pool);
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
                                egui::ScrollArea::both().show(ui, |ui| {
                                    ui.add_sized(
                                        [pool.mask_size as f32, pool.mask_size as f32],
                                        ObjectWrapper::new(obj, pool.get_pool()),
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
                        obj.render_parameters(ui, pool, &mut pool.get_mut_selected().borrow_mut());
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
        Box::new(|_| Ok(Box::<DesignerApp>::default())),
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
                Box::new(|_| Ok(Box::new(DesignerApp::default()))),
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
