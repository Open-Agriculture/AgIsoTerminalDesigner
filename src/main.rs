//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
mod app_persistence;

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::{DataCodeType, PictureGraphicFormat, Point};
use ag_iso_stack::object_pool::NullableObjectId;
use ag_iso_stack::object_pool::ObjectId;
use ag_iso_stack::object_pool::ObjectPool;
use ag_iso_stack::object_pool::ObjectType;
use ag_iso_terminal_designer::render_property_editor;
use ag_iso_terminal_designer::InteractiveMaskRenderer;
use ag_iso_terminal_designer::RenderableObject;
use ag_iso_terminal_designer::{EditorProject, Operation};
#[cfg(not(target_arch = "wasm32"))]
use app_persistence::RecentProject;
use app_persistence::{AppPersistence, RecoveryRecord};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::future::Future;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
#[cfg(target_arch = "wasm32")]
use std::{cell::Cell, rc::Rc};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{closure::Closure, JsCast as _};

const OBJECT_HIERARCHY_ID: &str = "object_hierarchy_ui";
const SELECTED_ANCESTRY_HIERARCHY_ID: &str = "selected_ancestry_hierarchy_ui";

#[derive(Clone, Copy)]
enum FileDialogReason {
    LoadPool,
    LoadProject,
    OpenImagePictureGraphics(ObjectId),
}

#[derive(Clone)]
enum PendingAction {
    OpenProject,
    ImportPool,
    #[cfg(not(target_arch = "wasm32"))]
    OpenRecent(PathBuf),
    Recover,
    Exit,
}

enum FileEvent {
    Opened {
        reason: FileDialogReason,
        content: Vec<u8>,
        file_name: String,
        #[cfg(not(target_arch = "wasm32"))]
        path: PathBuf,
    },
    SaveCompleted {
        #[cfg(not(target_arch = "wasm32"))]
        path: PathBuf,
        file_name: String,
    },
    OpenCancelled(FileDialogReason),
    SaveCancelled,
    Failed(String),
    #[cfg(not(target_arch = "wasm32"))]
    RecentFailed {
        path: PathBuf,
        error: String,
    },
}

pub struct DesignerApp {
    project: Option<EditorProject>,
    file_channel: (Sender<FileEvent>, Receiver<FileEvent>),
    show_development_popup: bool,
    new_object_dialog: Option<(ObjectType, String)>,
    apply_smart_naming_on_import: bool,
    #[cfg(not(target_arch = "wasm32"))]
    project_path: Option<PathBuf>,
    project_display_name: String,
    pending_action: Option<PendingAction>,
    confirm_discard_recovery: Option<PendingAction>,
    save_then_continue: Option<PendingAction>,
    discard_on_replace: bool,
    open_in_progress: bool,
    save_in_progress: bool,
    persistence: AppPersistence,
    recovery: Option<RecoveryRecord>,
    recovery_load_error: bool,
    error_message: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    recent_projects: Vec<RecentProject>,
    #[cfg(target_arch = "wasm32")]
    browser_dirty: Rc<Cell<bool>>,
    #[cfg(target_arch = "wasm32")]
    _before_unload: Option<Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>>,
}

impl DesignerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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

        let persistence = AppPersistence::new();
        let (recovery, mut error_message, recovery_load_error) = match persistence.load_recovery() {
            Ok(recovery) => (recovery, None, false),
            Err(error) => (None, Some(error), true),
        };

        #[cfg(not(target_arch = "wasm32"))]
        let recent_projects = match persistence.load_recents() {
            Ok(projects) => projects,
            Err(error) => {
                error_message = Some(match error_message {
                    Some(existing) => format!("{existing}\n\n{error}"),
                    None => error,
                });
                Vec::new()
            }
        };

        #[cfg(target_arch = "wasm32")]
        let browser_dirty = Rc::new(Cell::new(false));
        #[cfg(target_arch = "wasm32")]
        let before_unload = {
            let dirty = browser_dirty.clone();
            let callback = Closure::wrap(Box::new(move |event: web_sys::BeforeUnloadEvent| {
                if dirty.get() {
                    event.prevent_default();
                    event.set_return_value("Unsaved project changes");
                }
            }) as Box<dyn FnMut(_)>);
            if let Some(window) = web_sys::window() {
                if let Err(error) = window.add_event_listener_with_callback(
                    "beforeunload",
                    callback.as_ref().unchecked_ref(),
                ) {
                    error_message =
                        Some(error.as_string().unwrap_or_else(|| {
                            "Could not install browser close protection".into()
                        }));
                }
            }
            Some(callback)
        };

        Self {
            project: None,
            file_channel: std::sync::mpsc::channel(),
            show_development_popup: true,
            new_object_dialog: None,
            apply_smart_naming_on_import: true, // Default to true for better UX
            #[cfg(not(target_arch = "wasm32"))]
            project_path: None,
            project_display_name: "Untitled project".to_owned(),
            pending_action: None,
            confirm_discard_recovery: None,
            save_then_continue: None,
            discard_on_replace: false,
            open_in_progress: false,
            save_in_progress: false,
            persistence,
            recovery,
            recovery_load_error,
            error_message,
            #[cfg(not(target_arch = "wasm32"))]
            recent_projects,
            #[cfg(target_arch = "wasm32")]
            browser_dirty,
            #[cfg(target_arch = "wasm32")]
            _before_unload: before_unload,
        }
    }
}

impl DesignerApp {
    /// Open a file dialog
    fn open_file_dialog(&mut self, reason: FileDialogReason, ctx: &egui::Context) {
        if matches!(
            reason,
            FileDialogReason::LoadPool | FileDialogReason::LoadProject
        ) {
            if self.open_in_progress {
                return;
            }
            self.open_in_progress = true;
        }
        let is_image_loading = matches!(reason, FileDialogReason::OpenImagePictureGraphics(_));

        let sender = self.file_channel.0.clone();
        let mut dialog = rfd::AsyncFileDialog::new();

        // Add image file filters for image loading
        if is_image_loading {
            dialog = dialog.add_filter(
                "Image Files",
                &[
                    "png", "jpg", "jpeg", "bmp", "gif", "ico", "tiff", "tif", "webp",
                ],
            );
        } else if matches!(reason, FileDialogReason::LoadProject) {
            dialog = dialog.add_filter("AgIsoTerminal Project", &["aitp"]);
        } else {
            dialog = dialog.add_filter("ISOBUS Object Pool", &["iop"]);
        }

        let task = dialog.pick_file();
        let ctx = ctx.clone();
        execute(async move {
            if let Some(file) = task.await {
                let content = file.read().await;
                let file_name = file.file_name();
                #[cfg(not(target_arch = "wasm32"))]
                let event = FileEvent::Opened {
                    reason,
                    content,
                    file_name,
                    path: file.path().to_path_buf(),
                };
                #[cfg(target_arch = "wasm32")]
                let event = FileEvent::Opened {
                    reason,
                    content,
                    file_name,
                };
                let _ = sender.send(event);
            } else {
                let _ = sender.send(FileEvent::OpenCancelled(reason));
            }
            ctx.request_repaint();
        });
    }

    /// Handle a file loaded in the file dialog
    fn handle_file_loaded(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.file_channel.1.try_recv() {
            match event {
                FileEvent::Opened {
                    reason,
                    content,
                    file_name,
                    #[cfg(not(target_arch = "wasm32"))]
                    path,
                } => match reason {
                    FileDialogReason::LoadPool => {
                        self.open_in_progress = false;
                        let project = EditorProject::from(ObjectPool::from_iop(content));
                        // Apply smart naming to all objects that don't have custom names (if enabled)
                        if self.apply_smart_naming_on_import {
                            project.apply_smart_naming_to_all_objects();
                        }
                        project.mark_saved();
                        self.project = Some(project);
                        self.project_display_name = file_name;
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            self.project_path = None;
                        }
                        if self.discard_on_replace {
                            self.clear_recovery();
                            self.discard_on_replace = false;
                        }
                    }
                    FileDialogReason::LoadProject => match EditorProject::load_project(content) {
                        Ok(project) => {
                            self.open_in_progress = false;
                            self.project = Some(project);
                            self.project_display_name = file_name;
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                self.project_path = Some(path.clone());
                                self.touch_recent(path);
                            }
                            if self.discard_on_replace {
                                self.clear_recovery();
                                self.discard_on_replace = false;
                            }
                        }
                        Err(e) => {
                            self.open_in_progress = false;
                            self.discard_on_replace = false;
                            #[cfg(not(target_arch = "wasm32"))]
                            self.remove_recent(&path);
                            self.error_message = Some(format!("Failed to load project: {e}"));
                        }
                    },
                    FileDialogReason::OpenImagePictureGraphics(id) => {
                        if let Some(pool) = &mut self.project {
                            if let Some(Object::PictureGraphic(o)) =
                                pool.get_pool().object_by_id(id)
                            {
                                        if let Ok(img) = image::load_from_memory(&content) {
                                            // Update dimensions based on the new picture
                                            let w = img.width();
                                            let h = img.height();

                                            if w > u16::MAX as u32 || h > u16::MAX as u32 {
                                                log::error!(
                                                    "Image dimensions exceed maximum size of {}x{}",
                                                    u16::MAX,
                                                    u16::MAX
                                                );
                                                self.error_message = Some(format!(
                                                    "Image dimensions exceed the maximum of {}×{}",
                                                    u16::MAX,
                                                    u16::MAX
                                                ));
                                                continue;
                                            }

                                            let width = if o.width == 0 {
                                                w as u16
                                            } else {
                                                o.width
                                            };
                                            let mut options = o.options.clone();
                                            options.transparent = true;

                                            let rgba = if let Some(view) = img.as_rgba8() {
                                                // Borrowed view (no allocation)
                                                std::borrow::Cow::Borrowed(view)
                                            } else {
                                                // Allocates once if the image isn't already RGBA8
                                                std::borrow::Cow::Owned(img.to_rgba8())
                                            };

                                            // Build raw and run-length encoded data
                                            let pixel_count = (w as usize) * (h as usize);

                                            // Worst case: raw = N, rle = 2*N
                                            let mut raw = Vec::with_capacity(pixel_count);
                                            let mut rle = Vec::with_capacity(pixel_count * 2);

                                            let mut have_run = false;
                                            let mut run_value: u8 = 0;
                                            let mut run_count: u8 = 0;

                                            for p in rgba.pixels() {
                                                let idx = if p[3] == 0 {
                                                    1
                                                } else {
                                                    find_closest_color_index(p[0], p[1], p[2])
                                                };

                                                raw.push(idx);

                                                if !have_run {
                                                    have_run = true;
                                                    run_value = idx;
                                                    run_count = 1;
                                                    continue;
                                                }

                                                if idx == run_value && run_count < u8::MAX {
                                                    run_count += 1;
                                                } else {
                                                    rle.push(run_count);
                                                    rle.push(run_value);
                                                    run_value = idx;
                                                    run_count = 1;
                                                }
                                            }

                                            // flush final run
                                            if have_run {
                                                rle.push(run_count);
                                                rle.push(run_value);
                                            }

                                            // Choose the best encoding
                                            if rle.len() < raw.len() {
                                                options.data_code_type = DataCodeType::RunLength;
                                                log::info!(
                                            "Selected run-length encoding ({} bytes) over raw ({} bytes)",
                                            rle.len(),
                                            raw.len()
                                        );
                                            } else {
                                                options.data_code_type = DataCodeType::Raw;
                                                log::info!(
                                            "Selected raw encoding ({} bytes) over run-length ({} bytes)",
                                            raw.len(),
                                            rle.len()
                                        );
                                            }

                                            let data = if rle.len() < raw.len() { rle } else { raw };
                                            for (property, value) in [
                                                ("actual_width", serde_json::json!(w)),
                                                ("actual_height", serde_json::json!(h)),
                                                ("width", serde_json::json!(width)),
                                                ("format", serde_json::to_value(PictureGraphicFormat::EightBit).unwrap()),
                                                ("transparency_colour", serde_json::json!(1)),
                                                ("options", serde_json::to_value(options).unwrap()),
                                                ("data", serde_json::json!(data)),
                                            ] {
                                                pool.queue_operation(Operation::SetProperty {
                                                    object_id: id.value(),
                                                    property: property.to_owned(),
                                                    value,
                                                });
                                            }
                                        } else {
                                            self.error_message = Some(
                                                "Failed to decode the selected image".to_owned(),
                                            );
                                        }
                            }
                        }
                    }
                },
                FileEvent::SaveCompleted {
                    #[cfg(not(target_arch = "wasm32"))]
                    path,
                    file_name,
                } => {
                    self.save_in_progress = false;
                    if let Some(project) = &self.project {
                        project.mark_saved();
                    }
                    self.project_display_name = file_name;
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        self.project_path = Some(path.clone());
                        self.touch_recent(path);
                    }
                    self.clear_recovery();
                    if let Some(action) = self.save_then_continue.take() {
                        self.perform_action(action, ctx);
                    }
                }
                FileEvent::OpenCancelled(reason) => {
                    if matches!(
                        reason,
                        FileDialogReason::LoadPool | FileDialogReason::LoadProject
                    ) {
                        self.open_in_progress = false;
                        self.discard_on_replace = false;
                    }
                }
                FileEvent::SaveCancelled => {
                    self.save_in_progress = false;
                    self.save_then_continue = None;
                }
                FileEvent::Failed(error) => {
                    self.save_in_progress = false;
                    self.error_message = Some(error);
                    self.save_then_continue = None;
                    self.discard_on_replace = false;
                }
                #[cfg(not(target_arch = "wasm32"))]
                FileEvent::RecentFailed { path, error } => {
                    self.open_in_progress = false;
                    self.remove_recent(&path);
                    self.error_message = Some(error);
                    self.discard_on_replace = false;
                }
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

    /// Save to the known desktop path, or ask for a destination when needed.
    fn save_project(&mut self, save_as: bool, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        let _ = save_as;
        if self.save_in_progress {
            return;
        }
        if let Some(project) = &self.project {
            match project.save_project() {
                Ok(contents) => {
                    self.save_in_progress = true;
                    #[cfg(not(target_arch = "wasm32"))]
                    if !save_as {
                        if let Some(path) = self.project_path.clone() {
                            let sender = self.file_channel.0.clone();
                            let ctx = ctx.clone();
                            let file_name = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("project.aitp")
                                .to_owned();
                            execute(async move {
                                let event = match std::fs::write(&path, contents) {
                                    Ok(()) => FileEvent::SaveCompleted { path, file_name },
                                    Err(error) => FileEvent::Failed(format!(
                                        "Failed to save project: {error}"
                                    )),
                                };
                                let _ = sender.send(event);
                                ctx.request_repaint();
                            });
                            return;
                        }
                    }

                    let task = rfd::AsyncFileDialog::new()
                        .set_file_name(self.suggested_project_file_name())
                        .add_filter("AgIsoTerminal Project", &["aitp"])
                        .save_file();
                    let sender = self.file_channel.0.clone();
                    let ctx = ctx.clone();
                    execute(async move {
                        let event = if let Some(file) = task.await {
                            let file_name = file.file_name();
                            match file.write(&contents).await {
                                Ok(()) => {
                                    #[cfg(not(target_arch = "wasm32"))]
                                    let completed = FileEvent::SaveCompleted {
                                        path: file.path().to_path_buf(),
                                        file_name,
                                    };
                                    #[cfg(target_arch = "wasm32")]
                                    let completed = FileEvent::SaveCompleted { file_name };
                                    completed
                                }
                                Err(error) => {
                                    FileEvent::Failed(format!("Failed to save project: {error}"))
                                }
                            }
                        } else {
                            FileEvent::SaveCancelled
                        };
                        let _ = sender.send(event);
                        ctx.request_repaint();
                    });
                }
                Err(e) => {
                    self.save_in_progress = false;
                    self.error_message = Some(format!("Failed to serialize project: {e}"));
                    self.save_then_continue = None;
                }
            }
        }
    }

    fn request_action(&mut self, action: PendingAction, ctx: &egui::Context) {
        if self.save_in_progress {
            self.save_then_continue = Some(action);
            return;
        }
        if self.open_in_progress {
            return;
        }
        if self.project.as_ref().is_some_and(EditorProject::is_dirty) {
            self.pending_action = Some(action);
        } else if self.project.is_none()
            && self.recovery.is_some()
            && !matches!(action, PendingAction::Recover)
        {
            self.confirm_discard_recovery = Some(action);
        } else {
            self.perform_action(action, ctx);
        }
    }

    fn suggested_project_file_name(&self) -> String {
        if self.project_display_name.ends_with(".aitp") {
            return self.project_display_name.clone();
        }
        let stem = self
            .project_display_name
            .strip_suffix(".iop")
            .unwrap_or("project");
        let stem = stem.strip_prefix("Recovered — ").unwrap_or(stem);
        format!("{stem}.aitp")
    }

    fn perform_action(&mut self, action: PendingAction, ctx: &egui::Context) {
        match action {
            PendingAction::OpenProject => self.open_file_dialog(FileDialogReason::LoadProject, ctx),
            PendingAction::ImportPool => self.open_file_dialog(FileDialogReason::LoadPool, ctx),
            #[cfg(not(target_arch = "wasm32"))]
            PendingAction::OpenRecent(path) => {
                self.open_in_progress = true;
                let sender = self.file_channel.0.clone();
                let ctx = ctx.clone();
                execute(async move {
                    let event = match std::fs::read(&path) {
                        Ok(content) => FileEvent::Opened {
                            reason: FileDialogReason::LoadProject,
                            content,
                            file_name: path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("project.aitp")
                                .to_owned(),
                            path,
                        },
                        Err(error) => FileEvent::RecentFailed {
                            path,
                            error: format!("Could not open recent project: {error}"),
                        },
                    };
                    let _ = sender.send(event);
                    ctx.request_repaint();
                });
            }
            PendingAction::Recover => {
                if let Some(record) = self.recovery.clone() {
                    match EditorProject::load_project(record.project_json.into_bytes()) {
                        Ok(project) => {
                            project.mark_recovered();
                            self.project = Some(project);
                            self.project_display_name = record.display_name;
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                self.project_path = None;
                            }
                        }
                        Err(error) => {
                            self.error_message =
                                Some(format!("Could not recover project: {error}"));
                        }
                    }
                }
            }
            PendingAction::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn persist_recovery(&mut self) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        if !project.is_dirty() {
            if self.recovery.is_some() {
                self.clear_recovery();
            }
            return;
        }
        match project.save_project() {
            Ok(data) => match RecoveryRecord::new(self.project_display_name.clone(), data) {
                Ok(record) => match self.persistence.store_recovery(&record) {
                    Ok(()) => self.recovery = Some(record),
                    Err(error) => self.error_message = Some(error),
                },
                Err(error) => self.error_message = Some(error),
            },
            Err(error) => {
                self.error_message = Some(format!("Could not create recovery data: {error}"));
            }
        }
    }

    fn clear_recovery(&mut self) -> bool {
        if let Err(error) = self.persistence.clear_recovery() {
            self.error_message = Some(error);
            false
        } else {
            self.recovery = None;
            true
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn touch_recent(&mut self, path: PathBuf) {
        self.persistence
            .touch_recent(&mut self.recent_projects, path);
        if let Err(error) = self.persistence.store_recents(&self.recent_projects) {
            self.error_message = Some(error);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn remove_recent(&mut self, path: &std::path::Path) {
        self.recent_projects.retain(|entry| entry.path != path);
        if let Err(error) = self.persistence.store_recents(&self.recent_projects) {
            self.error_message = Some(error);
        }
    }

    /// Convert a string to a valid C identifier
    fn to_c_identifier(name: &str) -> String {
        name.chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' => c.to_ascii_uppercase(),
                _ => '_',
            })
            .collect()
    }

    /// Open a file dialog to save a C header file with object IDs
    fn save_header(&mut self) {
        if let Some(project) = &self.project {
            let pool = project.get_pool();

            // Start with the header
            let mut header = String::from("// Object IDs for the objects in the object pool.\n\n");
            header.push_str("#pragma once\n");
            header.push_str("#define UNDEFINED 65535\n");

            // Collect all objects with their names and IDs
            let mut objects: Vec<(String, u16)> = pool
                .objects()
                .iter()
                .map(|obj| {
                    let name = project.get_object_info(obj).get_name(obj);
                    let c_name = Self::to_c_identifier(&name);
                    let id = u16::from(obj.id());
                    (c_name, id)
                })
                .collect();

            // Sort by ID for consistent output
            objects.sort_by_key(|&(_, id)| id);

            // Add defines for each object
            for (name, id) in objects {
                header.push_str(&format!("#define {} {}\n", name, id));
            }

            let contents = header.into_bytes();
            let task = rfd::AsyncFileDialog::new()
                .set_file_name("object_pool.h")
                .add_filter("C Header", &["h"])
                .save_file();
            execute(async move {
                let file = task.await;
                if let Some(file) = file {
                    _ = file.write(&contents).await;
                }
            });
        }
    }

    fn render_startup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("AgIsoTerminalDesigner");
                ui.add_space(20.0);

                ui.label("Open an existing designer project:");
                if ui.button("Open Project (.aitp)").clicked() {
                    self.request_action(PendingAction::OpenProject, ctx);
                }

                ui.add_space(16.0);

                ui.label("Or start from an ISOBUS object pool:");
                if ui.button("Import IOP (.iop)").clicked() {
                    self.request_action(PendingAction::ImportPool, ctx);
                }

                if let Some(recovery) = self.recovery.clone() {
                    ui.add_space(24.0);
                    ui.group(|ui| {
                        ui.heading("Recovered work");
                        ui.label(format!(
                            "{} — {}",
                            recovery.display_name,
                            recovery_age_label(recovery.saved_at_unix_seconds)
                        ));
                        ui.label(
                            "This automatic snapshot may contain changes that were never saved.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Recover").clicked() {
                                self.request_action(PendingAction::Recover, ctx);
                            }
                            if ui.button("Discard").clicked() {
                                self.clear_recovery();
                            }
                        });
                    });
                }

                #[cfg(not(target_arch = "wasm32"))]
                if !self.recent_projects.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.heading("Recent projects");
                        let projects = self.recent_projects.clone();
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for recent in projects {
                                    let response = ui
                                        .button(&recent.display_name)
                                        .on_hover_text(recent.path.display().to_string());
                                    if response.clicked() {
                                        self.request_action(
                                            PendingAction::OpenRecent(recent.path),
                                            ctx,
                                        );
                                    }
                                }
                            });
                    });
                }
            });
        });
    }
}

fn recovery_age_label(saved_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let age = now.saturating_sub(saved_at);
    match age {
        0..=59 => "saved less than a minute ago".to_owned(),
        60..=3599 => format!("saved {} minutes ago", age / 60),
        3600..=86_399 => format!("saved {} hours ago", age / 3600),
        _ => format!("saved {} days ago", age / 86_400),
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
                .set(NullableObjectId(Some(object.id())));
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
                project.request_delete_object(object.id());
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

fn collect_selected_to_root_chains(
    pool: &ObjectPool,
    current: ObjectId,
    current_chain: &mut Vec<ObjectId>,
    current_path: &mut HashSet<ObjectId>,
    chains: &mut Vec<Vec<ObjectId>>,
) {
    let mut parent_ids: Vec<ObjectId> = pool
        .parent_objects(current)
        .iter()
        .map(|parent| parent.id())
        .collect();
    parent_ids.sort_by_key(|id| id.value());
    parent_ids.dedup_by_key(|id| id.value());

    if parent_ids.is_empty() {
        chains.push(current_chain.clone());
        return;
    }

    for parent_id in parent_ids {
        if current_path.contains(&parent_id) {
            continue;
        }

        current_path.insert(parent_id);
        current_chain.push(parent_id);
        collect_selected_to_root_chains(pool, parent_id, current_chain, current_path, chains);
        current_chain.pop();
        current_path.remove(&parent_id);
    }
}

fn build_selected_ancestry_adjacency(
    pool: &ObjectPool,
    selected: ObjectId,
) -> HashMap<ObjectId, Vec<ObjectId>> {
    let mut chains = Vec::new();
    let mut current_chain = vec![selected];
    let mut current_path = HashSet::from([selected]);
    collect_selected_to_root_chains(
        pool,
        selected,
        &mut current_chain,
        &mut current_path,
        &mut chains,
    );

    let mut adjacency_sets: HashMap<ObjectId, HashSet<ObjectId>> = HashMap::new();
    for chain in chains {
        for edge in chain.windows(2) {
            adjacency_sets.entry(edge[0]).or_default().insert(edge[1]);
        }
        if let Some(last) = chain.last() {
            adjacency_sets.entry(*last).or_default();
        }
    }

    adjacency_sets
        .into_iter()
        .map(|(id, parents)| {
            let mut sorted_parents: Vec<ObjectId> = parents.into_iter().collect();
            sorted_parents.sort_by_key(|parent| parent.value());
            (id, sorted_parents)
        })
        .collect()
}

fn render_selected_ancestry_hierarchy(
    ui: &mut egui::Ui,
    parent_id: egui::Id,
    object_id: ObjectId,
    project: &EditorProject,
    ancestry_adjacency: &HashMap<ObjectId, Vec<ObjectId>>,
    path_guard: &mut HashSet<ObjectId>,
) {
    let Some(object) = project.get_pool().object_by_id(object_id) else {
        ui.colored_label(
            egui::Color32::RED,
            format!("Missing object: {}", object_id.value()),
        );
        return;
    };

    let parents = ancestry_adjacency
        .get(&object_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    if parents.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().indent);
            render_selectable_object(ui, object, project);
        });
        return;
    }

    let id = parent_id.with(project.get_object_info(object).get_unique_id());
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            render_selectable_object(ui, object, project);
        })
        .body(|ui| {
            for (idx, parent) in parents.iter().enumerate() {
                if path_guard.contains(parent) {
                    ui.colored_label(
                        egui::Color32::RED,
                        format!("Cycle detected in ancestry at {}", parent.value()),
                    );
                    continue;
                }

                path_guard.insert(*parent);
                render_selected_ancestry_hierarchy(
                    ui,
                    id.with(idx),
                    *parent,
                    project,
                    ancestry_adjacency,
                    path_guard,
                );
                path_guard.remove(parent);
            }
        });
}

fn render_selected_ancestry_panel(ui: &mut egui::Ui, project: &EditorProject) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
    ui.strong("Selected object ancestry");
    ui.label("Direction: selected -> parent -> grandparent -> root");
    ui.separator();

    let Some(selected_id) = project.get_selected().into() else {
        ui.label("Select an object to view its hierarchies.");
        return;
    };

    if project.get_pool().object_by_id(selected_id).is_none() {
        ui.colored_label(
            egui::Color32::RED,
            format!("Selected object not found: {}", selected_id.value()),
        );
        return;
    }

    let ancestry_adjacency = build_selected_ancestry_adjacency(project.get_pool(), selected_id);
    let mut path_guard = HashSet::from([selected_id]);

    render_selected_ancestry_hierarchy(
        ui,
        egui::Id::new(SELECTED_ANCESTRY_HIERARCHY_ID),
        selected_id,
        project,
        &ancestry_adjacency,
        &mut path_guard,
    );

    let parent_count = ancestry_adjacency
        .get(&selected_id)
        .map_or(0, std::vec::Vec::len);
    if parent_count == 0 {
        ui.separator();
        ui.label("The selected object has no parent relationships.");
    }
}

impl eframe::App for DesignerApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.style_mut(|style| {
            style.interaction.selectable_labels = false;
        });

        // Handle file dialogs and deferred writes first.
        self.handle_file_loaded(ctx);

        #[cfg(not(target_arch = "wasm32"))]
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.project.as_ref().is_some_and(EditorProject::is_dirty) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if self.save_in_progress {
                    self.save_then_continue = Some(PendingAction::Exit);
                } else if self.pending_action.is_none() && self.save_then_continue.is_none() {
                    self.pending_action = Some(PendingAction::Exit);
                }
            }
        }

        if let Some(error) = self.error_message.clone() {
            egui::Window::new("Something went wrong")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(error);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            self.error_message = None;
                        }
                        if self.recovery_load_error
                            && ui.button("Discard invalid recovery data").clicked()
                        {
                            if self.clear_recovery() {
                                self.recovery_load_error = false;
                                self.error_message = None;
                            }
                        }
                    });
                });
        }

        if self.pending_action.is_some() {
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("This project has unsaved changes. Save them before continuing?");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            self.save_then_continue = self.pending_action.take();
                            self.save_project(false, ctx);
                        }
                        if ui.button("Discard").clicked() {
                            let action = self.pending_action.take();
                            if let Some(action) = action {
                                if matches!(action, PendingAction::Exit) {
                                    if let Some(project) = &self.project {
                                        project.mark_saved();
                                    }
                                    self.clear_recovery();
                                    self.perform_action(action, ctx);
                                } else {
                                    self.discard_on_replace = true;
                                    self.perform_action(action, ctx);
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_action = None;
                        }
                    });
                });
        }

        if self.confirm_discard_recovery.is_some() {
            egui::Window::new("Discard recovered work?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(
                        "Opening something else will discard the available recovery snapshot.",
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Discard and continue").clicked() {
                            let action = self.confirm_discard_recovery.take();
                            if let Some(action) = action {
                                self.discard_on_replace = true;
                                self.perform_action(action, ctx);
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_discard_recovery = None;
                        }
                    });
                });
        }

        if self.pending_action.is_some()
            || self.confirm_discard_recovery.is_some()
            || self.save_then_continue.is_some()
            || self.save_in_progress
            || self.open_in_progress
        {
            #[cfg(target_arch = "wasm32")]
            self.browser_dirty
                .set(self.project.as_ref().is_some_and(EditorProject::is_dirty));
            return;
        }

        // Check for image load requests
        if let Some(pool) = &self.project {
            if let Some(object_id) = pool.take_image_load_request() {
                self.open_file_dialog(FileDialogReason::OpenImagePictureGraphics(object_id), ctx);
            }
        }

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
                // Create the object through the operation API so it is
                // immediately represented in undo/redo history.
                if let Some(pool) = &mut self.project {
                    if let Some(id) = pool.create_object(object_type, name) {
                        pool.get_mut_selected()
                            .set(NullableObjectId::new(id.value()));
                    }
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

                if let Some(project) = &self.project {
                    let dirty_marker = if project.is_dirty() { " *" } else { "" };
                    ui.label(format!("{}{}", self.project_display_name, dirty_marker))
                        .on_hover_text("An asterisk means the project has unsaved changes");
                    ui.separator();
                }

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
                        .on_hover_text(format!(
                            "Undo{} ({})",
                            pool.undo_description()
                                .map(|description| format!(": {description}"))
                                .unwrap_or_default(),
                            ctx.format_shortcut(&undo_shortcut)
                        ))
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
                        .on_hover_text(format!(
                            "Redo{} ({})",
                            pool.redo_description()
                                .map(|description| format!(": {description}"))
                                .unwrap_or_default(),
                            ctx.format_shortcut(&redo_shortcut)
                        ))
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
                        self.request_action(PendingAction::OpenProject, ctx);
                        ui.close();
                    }
                    if self.project.is_some() && ui.button("Save Project (.aitp)").clicked() {
                        self.save_project(false, ctx);
                        ui.close();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if !self.recent_projects.is_empty() {
                            ui.menu_button("Recent Projects", |ui| {
                                let projects = self.recent_projects.clone();
                                for recent in projects {
                                    if ui
                                        .button(&recent.display_name)
                                        .on_hover_text(recent.path.display().to_string())
                                        .clicked()
                                    {
                                        self.request_action(
                                            PendingAction::OpenRecent(recent.path),
                                            ctx,
                                        );
                                        ui.close();
                                    }
                                }
                            });
                        }
                    }

                    ui.separator();
                    ui.label("ISOBUS Files");

                    if ui.button("Import IOP (.iop)").clicked() {
                        self.request_action(PendingAction::ImportPool, ctx);
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
                    if self.project.is_some() && ui.button("Export Header (.h)").clicked() {
                        self.save_header();
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
                        let mut mask_size = pool.mask_size;
                        if ui
                            .add(
                                egui::Slider::new(&mut mask_size, 100..=2000)
                                    .text("Virtual Mask size"),
                            )
                            .changed()
                        {
                            pool.set_mask_size(mask_size);
                        }
                    });
                }
            });
        });

        if let Some(pool) = &mut self.project {
            egui::TopBottomPanel::bottom("selected_ancestry_panel")
                .default_height(180.0)
                .resizable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        render_selected_ancestry_panel(ui, pool);
                        ui.allocate_space(ui.available_size());
                    });
                });

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
                                    pool.sort_objects_by_name();
                                    ui.close();
                                }
                                if ui.button("Sort by id").clicked() {
                                    pool.sort_objects_by_id();
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
                                                selected_ref.set(NullableObjectId(Some(object_id)));
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
                    if let Some(original) = pool.get_pool().object_by_id(id) {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            // Display editable object name as header
                            ui.horizontal(|ui| {
                                ui.label("Name:");

                                let object_info = pool.get_object_info(original);
                                let mut name = object_info.get_name(original);
                                let response = ui.text_edit_singleline(&mut name);

                                if response.changed() {
                                    pool.queue_operation(Operation::RenameObject {
                                        object_id: original.id().value(),
                                        name,
                                    });
                                }
                            });
                            ui.separator();

                            render_property_editor(ui, original, pool);
                            let (width, height) = pool.get_pool().content_size(original);
                            ui.separator();
                            let desired_size = egui::Vec2::new(width as f32, height as f32);
                            ui.allocate_ui(desired_size, |ui| {
                                original.render(ui, pool.get_pool(), Point::default());
                            });
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

            if pool.commit_queued("Edit object pool") {
                ctx.request_repaint();
            }
            if let Some((object_id, name)) = pool.take_rename_request() {
                if pool.rename_object(object_id, name) {
                    ctx.request_repaint();
                }
            }
            if let Some(object_id) = pool.take_delete_request() {
                if pool.delete_object(object_id) {
                    ctx.request_repaint();
                }
            }
            if let Some(error) = pool.take_operation_error() {
                self.error_message = Some(error);
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
            self.render_startup(ctx);
        }

        #[cfg(target_arch = "wasm32")]
        self.browser_dirty
            .set(self.project.as_ref().is_some_and(EditorProject::is_dirty));
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.persist_recovery();
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30)
    }

    fn persist_egui_memory(&self) -> bool {
        false
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

/// Find the closest color index in the palette for a given RGB value
fn find_closest_color_index(r: u8, g: u8, b: u8) -> u8 {
    fn quantize_channel(c: u8) -> u8 {
        // ((c + 25) / 51) in integer math, capped to 0..5
        let v = (c as u16 + 25) / 51;
        v.min(5) as u8
    }
    let rq = quantize_channel(r);
    let gq = quantize_channel(g);
    let bq = quantize_channel(b);

    16 + 36 * rq + 6 * gq + bq
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
