//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
mod app_persistence;
mod designer_settings;
mod workspace;
use workspace::{Pane, Workspace};

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::NullableObjectId;
use ag_iso_stack::object_pool::ObjectId;
use ag_iso_stack::object_pool::ObjectPool;
use ag_iso_stack::object_pool::ObjectType;
#[cfg(not(target_arch = "wasm32"))]
use ag_iso_terminal_designer::ai::automation_bridge::EditorAutomationBridge;
use ag_iso_terminal_designer::canvas::{mask_view::MaskViewState, CanvasState};
use ag_iso_terminal_designer::render_property_editor;
use ag_iso_terminal_designer::{EditorProject, Operation};
#[cfg(not(target_arch = "wasm32"))]
use app_persistence::RecentProject;
use app_persistence::{AppPersistence, RecoveryRecord};
use designer_settings::{AppPreferences, DimensionsDraft};
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
    NewPool,
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
    canvas: CanvasState,
    mask_view: MaskViewState,
    file_channel: (Sender<FileEvent>, Receiver<FileEvent>),
    show_development_popup: bool,
    new_object_dialog: Option<(ObjectType, String)>,
    pending_iop_import: Option<(
        Vec<u8>,
        String,
        ag_iso_stack::object_pool::vt_version::VtVersion,
    )>,
    apply_smart_naming_on_import: bool,
    #[cfg(not(target_arch = "wasm32"))]
    project_path: Option<PathBuf>,
    project_display_name: String,
    preferences: AppPreferences,
    workspace: Workspace,
    canvas_visible: bool,
    #[cfg(not(target_arch = "wasm32"))]
    proposal_apply: bool,
    #[cfg(not(target_arch = "wasm32"))]
    proposal_reject: bool,
    project_settings_open: bool,
    dimensions_draft: Option<DimensionsDraft>,
    #[cfg(not(target_arch = "wasm32"))]
    last_proposal_id: Option<String>,
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
    #[cfg(not(target_arch = "wasm32"))]
    automation_bridge: Option<EditorAutomationBridge>,
    #[cfg(not(target_arch = "wasm32"))]
    automation_bridge_error: Option<String>,
}

impl DesignerApp {
    fn show_iop_import(&mut self, ctx: &egui::Context) {
        let Some((_, name, version)) = self.pending_iop_import.as_mut() else {
            return;
        };
        let mut open = true;
        let mut import = false;
        let mut cancel = false;
        egui::Window::new("Import ISOBUS Object Pool")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(name.as_str());
                ui.label("Select the target VT version used to encode this pool.");
                egui::ComboBox::from_id_salt("import_vt_version")
                    .icon(ag_iso_terminal_designer::ui_style::combo_box_icon)
                    .selected_text(format!("VT{}", u8::from(*version)))
                    .show_ui(ui, |ui| {
                        for number in 2..=6 {
                            ui.selectable_value(
                                version,
                                ag_iso_stack::object_pool::vt_version::VtVersion::try_from(number)
                                    .unwrap(),
                                format!("VT{number}"),
                            );
                        }
                    });
                ui.horizontal(|ui| {
                    import = ui.button("Import").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if import {
            let (content, name, version) = self.pending_iop_import.take().unwrap();
            self.open_in_progress = false;
            match EditorProject::import_iop_with_id_format(
                content,
                version,
                self.preferences.object_id_format,
            ) {
                Ok(project) => {
                    if self.apply_smart_naming_on_import {
                        project.apply_smart_naming_to_all_objects();
                    }
                    project.mark_saved();
                    let diagnostics = project.compatibility_diagnostics();
                    self.close_project_settings();
                    self.canvas = CanvasState::default();
                    self.mask_view = MaskViewState::default();
                    self.project = Some(project);
                    self.project_display_name = name;
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        self.project_path = None;
                        self.invalidate_automation_proposal();
                    }
                    if self.discard_on_replace {
                        self.clear_recovery();
                    }
                    self.discard_on_replace = false;
                    match diagnostics {
                        Ok(diagnostics) if !diagnostics.is_empty() => {
                            self.error_message = Some(format!("Imported for repair. Resolve compatibility issues before IOP export:\n{}",
                                diagnostics.iter().map(|diagnostic| ag_iso_terminal_designer::pool_validation::format_validation_diagnostic(diagnostic, self.preferences.object_id_format)).collect::<Vec<_>>().join("\n")));
                        }
                        Err(error) => {
                            self.error_message = Some(error.to_string());
                        }
                        _ => {}
                    }
                }
                Err(error) => {
                    self.error_message = Some(error);
                    self.discard_on_replace = false;
                }
            }
        } else if cancel || !open {
            self.pending_iop_import = None;
            self.open_in_progress = false;
            self.discard_on_replace = false;
        }
    }
    fn close_project_settings(&mut self) {
        self.project_settings_open = false;
        self.dimensions_draft = None;
    }

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

        ag_iso_terminal_designer::ui_style::install(&_cc.egui_ctx);
        let preferences: AppPreferences = _cc
            .storage
            .and_then(|storage| eframe::get_value(storage, designer_settings::STORAGE_KEY))
            .unwrap_or_default();
        let workspace = _cc
            .storage
            .and_then(|storage| eframe::get_value::<Workspace>(storage, workspace::STORAGE_KEY))
            .filter(Workspace::valid)
            .unwrap_or_else(|| Workspace::new(&preferences));
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

        #[cfg(not(target_arch = "wasm32"))]
        let (automation_bridge, automation_bridge_error) =
            match EditorAutomationBridge::start(_cc.egui_ctx.clone()) {
                Ok(bridge) => (Some(bridge), None),
                Err(error) => (None, Some(error)),
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
            canvas: CanvasState::default(),
            mask_view: MaskViewState::default(),
            file_channel: std::sync::mpsc::channel(),
            show_development_popup: true,
            new_object_dialog: None,
            pending_iop_import: None,
            apply_smart_naming_on_import: true, // Default to true for better UX
            #[cfg(not(target_arch = "wasm32"))]
            project_path: None,
            project_display_name: "Untitled project".to_owned(),
            preferences,
            workspace,
            canvas_visible: false,
            #[cfg(not(target_arch = "wasm32"))]
            proposal_apply: false,
            #[cfg(not(target_arch = "wasm32"))]
            proposal_reject: false,
            project_settings_open: false,
            dimensions_draft: None,
            #[cfg(not(target_arch = "wasm32"))]
            last_proposal_id: None,
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
            #[cfg(not(target_arch = "wasm32"))]
            automation_bridge,
            #[cfg(not(target_arch = "wasm32"))]
            automation_bridge_error,
        }
    }
}

impl DesignerApp {
    #[cfg(not(target_arch = "wasm32"))]
    fn invalidate_automation_proposal(&mut self) {
        if let Some(bridge) = &mut self.automation_bridge {
            bridge.invalidate_proposal();
        }
    }
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
                        let version = self
                            .project
                            .as_ref()
                            .map(EditorProject::vt_version)
                            .unwrap_or(ag_iso_stack::object_pool::vt_version::VtVersion::Version6);
                        self.pending_iop_import = Some((content, file_name, version));
                    }
                    FileDialogReason::LoadProject => match EditorProject::load_project(content) {
                        Ok(project) => {
                            match project.compatibility_diagnostics() {
                                Ok(diagnostics) if !diagnostics.is_empty() => {
                                    self.error_message = Some(format!("Opened for repair. Resolve compatibility issues before IOP export:\n{}",
                                        diagnostics.iter().map(|diagnostic| ag_iso_terminal_designer::pool_validation::format_validation_diagnostic(diagnostic, self.preferences.object_id_format)).collect::<Vec<_>>().join("\n")));
                                }
                                Err(error) => {
                                    self.error_message = Some(error.to_string());
                                }
                                _ => {}
                            }
                            self.open_in_progress = false;
                            #[cfg(not(target_arch = "wasm32"))]
                            self.invalidate_automation_proposal();
                            self.close_project_settings();
                            self.canvas = CanvasState::default();
                            self.mask_view = MaskViewState::default();
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
                                let mut picture = o.clone();
                                match ag_iso_terminal_designer::graphics::apply_image_bytes_to_picture_graphic(&mut picture, &content) {
                                    Ok(()) => {
                                        let value = serde_json::to_value(&picture).unwrap();
                                        for property in ["actual_width", "actual_height", "width", "format", "transparency_colour", "options", "data"] {
                                            pool.queue_operation(Operation::SetProperty {
                                                object: ag_iso_terminal_designer::ObjectTarget::id(id.value()),
                                                property: property.into(), value: value[property].clone(),
                                            });
                                        }
                                    }
                                    Err(e) => self.error_message = Some(e),
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
            let contents = match pool.export_iop() {
                Ok(contents) => contents,
                Err(error) => {
                    self.error_message = Some(error.to_string());
                    return;
                }
            };
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
            PendingAction::NewPool => {
                #[cfg(not(target_arch = "wasm32"))]
                self.invalidate_automation_proposal();
                self.close_project_settings();
                self.canvas = CanvasState::default();
                self.mask_view = MaskViewState::default();
                self.project = Some(EditorProject::new_pool());
                self.project_display_name = "Untitled.aitp".to_owned();
                self.new_object_dialog = None;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.project_path = None;
                }
                if self.discard_on_replace {
                    self.clear_recovery();
                    self.discard_on_replace = false;
                }
            }
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
                            #[cfg(not(target_arch = "wasm32"))]
                            self.invalidate_automation_proposal();
                            self.close_project_settings();
                            self.canvas = CanvasState::default();
                            self.mask_view = MaskViewState::default();
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
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ctx.style().visuals.faint_bg_color))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        let width = (ui.available_width() - 32.0).clamp(0.0, 520.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 0.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                egui::Frame::group(ui.style())
                                    .fill(ui.visuals().panel_fill)
                                    .inner_margin(24)
                                    .corner_radius(8)
                                    .show(ui, |ui| {
                                        ui.vertical_centered(|ui| {
                                            self.render_welcome(ui, ctx);
                                        });
                                    });
                            },
                        );
                    });
                });
            });
    }
    fn render_welcome(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            egui::RichText::new("AgIsoTerminalDesigner")
                .size(26.0)
                .strong(),
        );
        ui.weak("Design ISOBUS terminal interfaces");
        ui.add_space(20.0);

        if ui.button("New Object Pool").clicked() {
            self.request_action(PendingAction::NewPool, ctx);
        }
        ui.add_space(16.0);

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
                ui.label("This automatic snapshot may contain changes that were never saved.");
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
                                self.request_action(PendingAction::OpenRecent(recent.path), ctx);
                            }
                        }
                    });
            });
        }
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

fn render_selectable_object(ui: &mut egui::Ui, object: &Object, project: &EditorProject) -> bool {
    let this_ui_id = ui.id();
    let object_info = project.get_object_info(object);

    let renaming_object = project.get_renaming_object();
    if renaming_object
        .clone()
        .is_some_and(|(ui_id, id, _)| id == object.id() && ui_id == this_ui_id)
    {
        ui.data_mut(|data| {
            data.remove::<egui::layers::ShapeIdx>(egui::Id::new("tree_row_background"))
        });
        let mut name = renaming_object.unwrap().2;
        let response =
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width()));
        project.set_renaming_object(this_ui_id, object.id(), name); // Update the name in the project
        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if response.lost_focus() {
            project.finish_renaming_object(!cancelled);
        } else if !response.has_focus() {
            // We need to focus the text edit when we start renaming
            response.request_focus();
        }
        false
    } else {
        let is_selected = project.get_selected() == object.id().into();
        let label_text = format!(
            "{}: {}",
            project.format_object_id(object.id().value()),
            project.object_display_name(object)
        );
        let response = tree_row(ui, is_selected, label_text);

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
        response.clicked()
    }
}

/// Show flat object-list results without filtering the navigation hierarchy.
fn render_filtered_object_list(
    ui: &mut egui::Ui,
    project: &EditorProject,
    query: &str,
    mask_view: &mut MaskViewState,
) {
    let matches: Vec<_> = project
        .get_pool()
        .objects()
        .iter()
        .filter(|object| {
            ag_iso_terminal_designer::object_picker::matches_object(project, object, query)
        })
        .collect();
    let title = if query.is_empty() {
        "All objects".to_owned()
    } else {
        format!("Matching objects ({})", matches.len())
    };
    ag_iso_terminal_designer::ui_style::section_heading(ui, &title);
    if matches.is_empty() {
        ui.weak(if query.is_empty() {
            "No objects"
        } else {
            "No matching objects"
        });
    }
    for object in matches {
        if render_selectable_object(ui, object, project) {
            mask_view.navigate(project, object.id().value());
        }
    }
}

fn render_mask_object_hierarchy(
    ui: &mut egui::Ui,
    parent_id: egui::Id,
    root: u16,
    object: &Object,
    project: &EditorProject,
    mask_view: &mut MaskViewState,
    canvas: &mut CanvasState,
    path: &mut Vec<usize>,
    visiting: &mut HashSet<u16>,
    softkey_slot: Option<usize>,
    open_selection: bool,
) {
    let id_value = object.id().value();
    if !visiting.insert(id_value) {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Cycle detected at {}", project.format_object_id(id_value)),
        );
        return;
    }
    let component_instances: Vec<_> = project
        .component_source()
        .instances
        .iter()
        .filter(|instance| {
            instance.parent
                == ag_iso_terminal_designer::components::EditorParentRef::Primitive(id_value)
        })
        .collect();
    let placed = ag_iso_terminal_designer::operations::object_refs(object).unwrap_or_default();
    let pointer_target = match object {
        Object::ObjectPointer(pointer) => pointer.value.0.map(|id| id.value()),
        _ => None,
    };
    let has_children =
        !placed.is_empty() || !component_instances.is_empty() || pointer_target.is_some();
    let occurrence_id = parent_id.with((root, path.clone()));
    let selected_id = project.get_selected().0.map(|id| id.value());
    let expand = open_selection
        && selected_id.is_some_and(|selected| {
            mask_branch_contains(project, object, selected, &mut HashSet::new())
        });

    if has_children {
        let selected_in_header = std::cell::Cell::new(None);
        if expand {
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                occurrence_id,
                true,
            );
            state.set_open(true);
            state.store(ui.ctx());
        }
        ag_iso_terminal_designer::ui_style::collapsing_section(
            ui,
            occurrence_id,
            expand,
            |ui| {
                if render_selectable_object(ui, object, project) {
                    selected_in_header.set(Some((id_value, softkey_slot)));
                }
            },
            |ui| {
                for (index, reference) in placed.iter().enumerate() {
                    path.push(index);
                    if let Some(child) = project.get_pool().object_by_id(reference.id) {
                        render_mask_object_hierarchy(
                            ui,
                            occurrence_id.with(index),
                            root,
                            child,
                            project,
                            mask_view,
                            canvas,
                            path,
                            visiting,
                            softkey_slot,
                            open_selection,
                        );
                    } else {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!("Missing placed object: {}", reference.id.value()),
                        );
                    }
                    path.pop();
                }
                if let Some(target_id) = pointer_target {
                    let pointer_branch = occurrence_id.with("pointer-target");
                    let target = ag_iso_stack::object_pool::ObjectId::new(target_id)
                        .ok()
                        .and_then(|id| project.get_pool().object_by_id(id));
                    match target {
                        Some(target) => {
                            let opens_target = open_selection
                                && selected_id.is_some_and(|selected| {
                                    mask_branch_contains(
                                        project,
                                        target,
                                        selected,
                                        &mut HashSet::new(),
                                    )
                                });
                            let target_header_id = ui.make_persistent_id(pointer_branch);
                            if opens_target {
                                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(),
                                    target_header_id,
                                    true,
                                );
                                state.set_open(true);
                                state.store(ui.ctx());
                            }
                            egui::CollapsingHeader::new("Active pointer target")
                                .id_salt(pointer_branch)
                                .show(ui, |ui| {
                                    path.push(usize::MAX);
                                    render_mask_object_hierarchy(
                                        ui,
                                        pointer_branch,
                                        root,
                                        target,
                                        project,
                                        mask_view,
                                        canvas,
                                        path,
                                        visiting,
                                        softkey_slot,
                                        open_selection,
                                    );
                                    path.pop();
                                });
                        }
                        None => {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                format!("Missing active pointer target: {target_id}"),
                            );
                        }
                    };
                }
                for instance in component_instances {
                    let definition =
                        &project.component_source().definitions[&instance.component_id];
                    let component_salt = (root, instance.id, path.clone());
                    let opens_component = open_selection
                        && selected_id.is_some_and(|selected| {
                            instance.compiled_ids.values().any(|id| *id == selected)
                        });
                    let component_header_id = ui.make_persistent_id(component_salt.clone());
                    if opens_component {
                        let mut state =
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                component_header_id,
                                true,
                            );
                        state.set_open(true);
                        state.store(ui.ctx());
                    }
                    egui::CollapsingHeader::new(&definition.name)
                        .id_salt(component_salt)
                        .show(ui, |ui| {
                            for node in &definition.nodes {
                                let node_id = instance.compiled_ids[&node.id];
                                if tree_row(
                                    ui,
                                    project
                                        .get_selected()
                                        .0
                                        .is_some_and(|selected| selected.value() == node_id),
                                    &node.name,
                                )
                                .clicked()
                                {
                                    project
                                        .get_mut_selected()
                                        .set(NullableObjectId::new(node_id));
                                    if let Some(slot) = softkey_slot {
                                        canvas.select_softkey_slot(project, root, slot);
                                    }
                                }
                            }
                        });
                }
            },
        );
        if let Some((selected, slot)) = selected_in_header.get() {
            mask_view.navigate(project, selected);
            if let Some(slot) = slot {
                canvas.select_softkey_slot(project, root, slot);
            }
        }
    } else if render_selectable_object(ui, object, project) {
        mask_view.navigate(project, id_value);
        if let Some(slot) = softkey_slot {
            canvas.select_softkey_slot(project, root, slot);
        }
    }
    visiting.remove(&id_value);
}

fn mask_branch_contains(
    project: &EditorProject,
    object: &Object,
    selected: u16,
    visiting: &mut HashSet<u16>,
) -> bool {
    let id = object.id().value();
    if id == selected {
        return true;
    }
    if !visiting.insert(id) {
        return false;
    }
    let component_contains = project
        .component_source()
        .instances
        .iter()
        .filter(|instance| {
            instance.parent == ag_iso_terminal_designer::components::EditorParentRef::Primitive(id)
        })
        .any(|instance| {
            instance
                .compiled_ids
                .values()
                .any(|node_id| *node_id == selected)
        });
    let placements_contain = ag_iso_terminal_designer::operations::object_refs(object)
        .unwrap_or_default()
        .iter()
        .filter_map(|reference| project.get_pool().object_by_id(reference.id))
        .any(|child| mask_branch_contains(project, child, selected, visiting));
    let pointer_contains = match object {
        Object::ObjectPointer(pointer) => pointer
            .value
            .0
            .and_then(|target| project.get_pool().object_by_id(target))
            .is_some_and(|target| mask_branch_contains(project, target, selected, visiting)),
        _ => false,
    };
    visiting.remove(&id);
    component_contains || placements_contain || pointer_contains
}

fn render_focused_mask_hierarchy(
    ui: &mut egui::Ui,
    project: &EditorProject,
    mask_view: &mut MaskViewState,
    canvas: &mut CanvasState,
) {
    let Some(root) = mask_view.focused else {
        ui.weak("Select a mask to view its hierarchy.");
        return;
    };
    let Some(mask) = ag_iso_stack::object_pool::ObjectId::new(root)
        .ok()
        .and_then(|id| project.get_pool().object_by_id(id))
    else {
        ui.weak("The focused mask no longer exists.");
        return;
    };
    ag_iso_terminal_designer::ui_style::section_heading(
        ui,
        &format!("Hierarchy: {}", project.object_display_name(mask)),
    );
    let selection_marker = egui::Id::new("focused_mask_tree_selection");
    let selected = project.get_selected().0.map(|id| id.value());
    let previous_selection = ui.data(|data| data.get_temp::<(u16, Option<u16>)>(selection_marker));
    let open_selection = previous_selection != Some((root, selected));
    ui.data_mut(|data| data.insert_temp(selection_marker, (root, selected)));
    let mut visit = |ui: &mut egui::Ui, object: &Object, index: usize, slot| {
        render_mask_object_hierarchy(
            ui,
            egui::Id::new((OBJECT_HIERARCHY_ID, root, index)),
            root,
            object,
            project,
            mask_view,
            canvas,
            &mut vec![index],
            &mut HashSet::new(),
            slot,
            open_selection,
        );
    };
    visit(ui, mask, 0, None);

    let softkey_id = match mask {
        Object::DataMask(mask) => mask.soft_key_mask.0.map(|id| id.value()),
        Object::AlarmMask(mask) => mask.soft_key_mask.0.map(|id| id.value()),
        _ => None,
    };
    if let Some(softkey_id) = softkey_id {
        ui.separator();
        let softkeys = ag_iso_stack::object_pool::ObjectId::new(softkey_id)
            .ok()
            .and_then(|id| project.get_pool().object_by_id(id));
        match softkeys {
            Some(softkey_mask @ Object::SoftKeyMask(_)) => {
                let selected_softkey_mask = std::cell::Cell::new(false);
                let section_id = egui::Id::new((OBJECT_HIERARCHY_ID, root, "softkey-mask"));
                let opens_softkeys = open_selection
                    && selected.is_some_and(|selected| {
                        softkey_id == selected
                            || match softkey_mask {
                                Object::SoftKeyMask(softkeys) => {
                                    softkeys.objects.iter().any(|id| {
                                        project.get_pool().object_by_id(*id).is_some_and(|key| {
                                            mask_branch_contains(
                                                project,
                                                key,
                                                selected,
                                                &mut HashSet::new(),
                                            )
                                        })
                                    })
                                }
                                _ => false,
                            }
                    });
                if opens_softkeys {
                    let mut state =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            section_id,
                            true,
                        );
                    state.set_open(true);
                    state.store(ui.ctx());
                }
                ag_iso_terminal_designer::ui_style::collapsing_section(
                    ui,
                    section_id,
                    opens_softkeys,
                    |ui| {
                        if render_selectable_object(ui, softkey_mask, project) {
                            selected_softkey_mask.set(true);
                        }
                    },
                    |ui| {
                        let Object::SoftKeyMask(softkeys) = softkey_mask else {
                            return;
                        };
                        for (slot, key_id) in softkeys.objects.iter().enumerate() {
                            let key = project.get_pool().object_by_id(*key_id);
                            match key {
                                Some(Object::ObjectPointer(pointer))
                                    if pointer.value.0.is_none() =>
                                {
                                    ui.weak(format!("Slot {}: Empty", slot + 1));
                                }
                                Some(key) => visit(ui, key, slot + 1, Some(slot)),
                                None => {
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        format!(
                                            "Missing key in slot {}: {}",
                                            slot + 1,
                                            key_id.value()
                                        ),
                                    );
                                }
                            }
                        }
                    },
                );
                if selected_softkey_mask.get() {
                    mask_view.navigate(project, softkey_id);
                }
            }
            Some(_) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Object {softkey_id} is not a SoftKeyMask"),
                );
            }
            None => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Missing linked soft-key mask: {softkey_id}"),
                );
            }
        };
    }
}

fn render_mask_list(ui: &mut egui::Ui, project: &EditorProject, mask_view: &mut MaskViewState) {
    let filter_id = egui::Id::new("mask_list_filter");
    let previous_filter_id = egui::Id::new("mask_list_previous_filter");
    let mut filter = ui
        .data(|data| data.get_temp::<String>(filter_id))
        .unwrap_or_default();
    let previous_filter = ui
        .data(|data| data.get_temp::<String>(previous_filter_id))
        .unwrap_or_default();
    ui.label("Masks to preview");
    ui.horizontal(|ui| {
        let clear_width = if filter.is_empty() {
            0.0
        } else {
            28.0 + ui.spacing().item_spacing.x
        };
        ui.add(
            egui::TextEdit::singleline(&mut filter)
                .hint_text("Filter masks by name or ID…")
                .desired_width((ui.available_width() - clear_width).max(0.0)),
        );
        if !filter.is_empty()
            && ag_iso_terminal_designer::ui_style::icon_button(
                ui,
                ag_iso_terminal_designer::ui_style::Icon::Close,
                "Clear mask filter",
            )
            .clicked()
        {
            filter.clear();
        }
    });
    ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
    let query = filter.to_lowercase();
    let masks: Vec<_> = project
        .get_pool()
        .objects()
        .iter()
        .filter(|object| matches!(object, Object::DataMask(_) | Object::AlarmMask(_)))
        .filter(|object| {
            query.is_empty()
                || ag_iso_terminal_designer::object_picker::matches_object(project, object, &query)
        })
        .collect();
    let scroll = egui::ScrollArea::vertical()
        .id_salt("mask_list_scroll")
        .max_height((ui.available_height() / 3.0).min(200.0).max(40.0));
    let scroll = if filter != previous_filter {
        scroll.vertical_scroll_offset(0.0)
    } else {
        scroll
    };
    scroll.show(ui, |ui| {
        if masks.is_empty() {
            ui.weak(if query.is_empty() {
                "No data or alarm masks"
            } else {
                "No matching masks"
            });
        }
        for mask in masks {
            let id = mask.id().value();
            ui.horizontal(|ui| {
                let mut visible = mask_view.visible.contains(&id);
                if ui.checkbox(&mut visible, "").changed() {
                    mask_view.set_visible(id, visible);
                }
                let kind = if matches!(mask, Object::DataMask(_)) {
                    "Data"
                } else {
                    "Alarm"
                };
                let label = format!(
                    "{kind} {}: {}",
                    project.format_object_id(id),
                    project.object_display_name(mask)
                );
                let response = ui.selectable_label(mask_view.focused == Some(id), label);
                if response.clicked() {
                    if mask_view.focused == Some(id) {
                        mask_view.unfocus();
                    } else {
                        mask_view.focus(id);
                        project.get_mut_selected().set(mask.id().into());
                    }
                }
            });
        }
    });
    ui.data_mut(|data| data.insert_temp(previous_filter_id, filter));
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
    let row_background = ui.painter().add(egui::Shape::Noop);
    ui.data_mut(|data| data.insert_temp(egui::Id::new("tree_row_background"), row_background));
    let Some(object) = project.get_pool().object_by_id(object_id) else {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!(
                "Missing object: {}",
                project.format_object_id(object_id.value())
            ),
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
    ag_iso_terminal_designer::ui_style::collapsing_section(
        ui,
        id,
        true,
        |ui| {
            render_selectable_object(ui, object, project);
        },
        |ui| {
            for (idx, parent) in parents.iter().enumerate() {
                if path_guard.contains(parent) {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!(
                            "Cycle detected in ancestry at {}",
                            project.format_object_id(parent.value())
                        ),
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
        },
    );
}

fn render_selected_ancestry_panel(ui: &mut egui::Ui, project: &EditorProject) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
    ui.data_mut(|data| data.insert_temp(egui::Id::new("tree_row_bounds"), ui.max_rect()));
    ui.strong("Selected object ancestry");
    ui.label("Direction: selected -> parent -> grandparent -> root");
    ui.separator();

    let Some(selected_id) = project.get_selected().into() else {
        ui.label("Select an object to view its hierarchies.");
        return;
    };

    if project.get_pool().object_by_id(selected_id).is_none() {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!(
                "Selected object not found: {}",
                project.format_object_id(selected_id.value())
            ),
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
        self.mask_view.begin_frame();
        self.preferences.apply(ctx);
        ctx.style_mut(|style| {
            style.interaction.selectable_labels = false;
        });

        // Handle file dialogs and deferred writes first.
        self.handle_file_loaded(ctx);
        self.show_iop_import(ctx);
        if let Some(project) = &mut self.project {
            project.set_object_id_format(self.preferences.object_id_format);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(bridge) = &mut self.automation_bridge {
            bridge.poll(self.project.as_ref());
        }

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

        if let Some(project) = &mut self.project {
            project.take_graphic_options_request();
            project.take_graphic_format_request();
        }
        // Check for image load requests
        if let Some(pool) = &self.project {
            if let Some(object_id) = pool.take_image_load_request() {
                self.open_file_dialog(FileDialogReason::OpenImagePictureGraphics(object_id), ctx);
            }
        }

        if self.show_development_popup {
            egui::Window::new("Under Active Development")
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

        #[cfg(not(target_arch = "wasm32"))]
        {
            let proposal_id = self
                .automation_bridge
                .as_ref()
                .and_then(|bridge| bridge.proposal())
                .map(|proposal| proposal.proposal_id.clone());
            if proposal_id.is_some() && proposal_id != self.last_proposal_id {
                self.workspace.open(Pane::Proposal);
            }
            self.last_proposal_id = proposal_id;
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

                    let response = ui.add(
                        egui::TextEdit::singleline(&mut name).desired_width(ui.available_width()),
                    );

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
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Object Pool").clicked() {
                        self.request_action(PendingAction::NewPool, ctx);
                        ui.close();
                    }
                    if ui.button("Open Project…").clicked() {
                        self.request_action(PendingAction::OpenProject, ctx);
                        ui.close();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    if !self.recent_projects.is_empty() {
                        ui.menu_button("Recent Projects", |ui| {
                            for recent in self.recent_projects.clone() {
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
                    ui.separator();
                    ui.add_enabled_ui(self.project.is_some(), |ui| {
                        if ui.button("Save").clicked() {
                            self.save_project(false, ctx);
                            ui.close();
                        }
                        if ui.button("Save As…").clicked() {
                            self.save_project(true, ctx);
                            ui.close();
                        }
                    });
                    ui.separator();
                    if ui.button("Import IOP…").clicked() {
                        self.request_action(PendingAction::ImportPool, ctx);
                        ui.close();
                    }
                    ui.checkbox(
                        &mut self.apply_smart_naming_on_import,
                        "Apply smart naming on import",
                    );
                    ui.add_enabled_ui(self.project.is_some(), |ui| {
                        if ui.button("Export IOP…").clicked() {
                            self.save_pool();
                            ui.close();
                        }
                        if ui.button("Export Header…").clicked() {
                            self.save_header();
                            ui.close();
                        }
                    });
                });
                ui.menu_button("Project", |ui| {
                    ui.add_enabled_ui(self.project.is_some(), |ui| {
                        ui.menu_button("Add Object", |ui| {
                            let version = self.project.as_ref().unwrap().vt_version();
                            let unavailable = ObjectType::values()
                                .into_iter()
                                .filter(|object_type| !ag_iso_terminal_designer::vt_compatibility::supported_object_type(*object_type, version))
                                .filter_map(|object_type| {
                                    ag_iso_terminal_designer::vt_compatibility::availability_description(|v| {
                                        ag_iso_terminal_designer::vt_compatibility::supported_object_type(object_type, v)
                                    }).map(|availability| format!("{object_type:?} — {availability}"))
                                })
                                .collect();
                            egui::ScrollArea::vertical()
                                .max_height(400.0)
                                .show(ui, |ui| {
                                    ag_iso_terminal_designer::ui_style::vt_availability_note(
                                        ui, "add_object_vt_availability", version, unavailable,
                                    );
                                    for object_type in ObjectType::values() {
                                        if !ag_iso_terminal_designer::vt_compatibility::supported_object_type(
                                            object_type, self.project.as_ref().unwrap().vt_version(),
                                        ) { continue; }
                                        if ui.button(format!("{:?}", object_type)).clicked() {
                                            let default_name = self
                                                .project
                                                .as_ref()
                                                .unwrap()
                                                .generate_smart_name_for_new_object(object_type);
                                            self.new_object_dialog =
                                                Some((object_type, default_name));
                                            ui.close();
                                        }
                                    }
                                });
                        });
                        if ui.button("Project Settings…").clicked() {
                            self.project_settings_open = true;
                            self.dimensions_draft =
                                self.project.as_ref().map(DimensionsDraft::from_project);
                            ui.close();
                        }
                    });
                });
                self.show_view_menu(ui, ctx);
                ui.separator();
                let dirty_marker = if self.project.as_ref().is_some_and(EditorProject::is_dirty) {
                    " *"
                } else {
                    ""
                };
                let project_name = if self.project.is_some() {
                    format!("{}{}", self.project_display_name, dirty_marker)
                } else {
                    "No project open".to_owned()
                };
                let reserved_undo_width = if self.project.is_some() { 120.0 } else { 0.0 };
                let project_name_width = (ui.available_width() - reserved_undo_width).max(0.0);
                ui.scope(|ui| {
                    ui.set_max_width(project_name_width);
                    ui.add(egui::Label::new(project_name).truncate())
                })
                .inner
                .on_hover_text("An asterisk means the project has unsaved changes");

                if let Some(pool) = &mut self.project {
                    let undo_shortcut =
                        egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Z);
                    let redo_shortcut =
                        egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Y);

                    if ui
                        .add_enabled(pool.undo_available(), egui::widgets::Button::new("Undo"))
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
                        .add_enabled(pool.redo_available(), egui::widgets::Button::new("Redo"))
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
            });
        });

        let requested_pane = designer_settings::show_project_settings(
            ctx,
            &mut self.project_settings_open,
            self.project.as_mut(),
            &mut self.preferences,
            &mut self.dimensions_draft,
            &mut self.error_message,
        );
        if let Some(pane) = requested_pane {
            self.workspace.open(pane);
        }

        if self.project.is_some() {
            self.render_workspace(ctx);
        } else {
            self.render_startup(ctx);
        }
        if let Some(pool) = &mut self.project {
            pool.set_object_id_format(self.preferences.object_id_format);
            let selected_before_navigation = pool.get_selected();
            let mut explicit_navigation = false;
            if ctx.input(|i| i.pointer.button_released(egui::PointerButton::Extra1)) {
                pool.set_previous_selected();
            } else if ctx.input(|i| i.pointer.button_released(egui::PointerButton::Extra2)) {
                pool.set_next_selected();
            }
            let search_id = egui::Id::new("find_project_object");
            let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F);
            if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
                ag_iso_terminal_designer::object_picker::open(ctx, search_id);
            }
            if let Some(selected) = ag_iso_terminal_designer::object_picker::object_window(
                ctx,
                search_id,
                "Navigate to this object and open its inspector (no project changes)",
                pool,
                pool.get_selected().0,
                false,
                |_| true,
                |ui, choice| {
                    ui.label("Navigation only; no objects will change.");
                    ag_iso_terminal_designer::object_picker::review_object(ui, pool, choice)
                },
            ) {
                pool.get_mut_selected().set(selected);
                explicit_navigation = true;
            }

            if pool.commit_queued("Edit object pool") {
                ctx.request_repaint();
            }
            if pool.commit_graphic_assignment() {
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
            if pool.update_selected()
                || pool.get_selected() != selected_before_navigation
                || explicit_navigation
            {
                if let Some(selected) = pool.get_selected().0 {
                    if let Some(root) = self.mask_view.navigate(pool, selected.value()) {
                        if let Ok(materialized) = pool.materialize_shared() {
                            self.canvas.navigate_to_object(
                                pool,
                                &materialized,
                                root,
                                selected.value(),
                            );
                        }
                    }
                }
                ctx.request_repaint();
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        self.finish_proposal_actions();

        #[cfg(target_arch = "wasm32")]
        self.browser_dirty
            .set(self.project.as_ref().is_some_and(EditorProject::is_dirty));
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.faint_bg_color.to_normalized_gamma_f32()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, designer_settings::STORAGE_KEY, &self.preferences);
        eframe::set_value(storage, workspace::STORAGE_KEY, &self.workspace);
        self.persist_recovery();
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30)
    }

    fn persist_egui_memory(&self) -> bool {
        // Dock windows keep their position and resize state in egui memory.
        true
    }
}

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
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

impl DesignerApp {
    fn render_pane(&mut self, ui: &mut egui::Ui, pane: Pane) {
        let ctx = ui.ctx().clone();
        #[cfg(not(target_arch = "wasm32"))]
        if pane == Pane::Proposal {
            self.render_proposal(ui);
            return;
        }
        let Some(pool) = self.project.as_mut() else {
            ui.label("Open a project to use this pane.");
            return;
        };
        match pane {
            Pane::Objects => {
                self.mask_view.reconcile(pool);
                render_mask_list(ui, pool, &mut self.mask_view);
                ui.separator();
                let filter_id = egui::Id::new("objects_list_filter");
                let mut filter_text = ui
                    .data(|data| data.get_temp::<String>(filter_id))
                    .unwrap_or_default();
                let previous_filter = filter_text.clone();
                ui.label("Filter object list");
                ui.horizontal(|ui| {
                    let show_clear = !filter_text.is_empty();
                    let clear_width = if !show_clear {
                        0.0
                    } else {
                        28.0 + ui.spacing().item_spacing.x
                    };
                    ui.add(
                        egui::TextEdit::singleline(&mut filter_text)
                            .hint_text("Name or ID…")
                            .desired_width((ui.available_width() - clear_width).max(0.0)),
                    );
                    if show_clear
                        && ag_iso_terminal_designer::ui_style::icon_button(
                            ui,
                            ag_iso_terminal_designer::ui_style::Icon::Close,
                            "Clear object filter",
                        )
                        .clicked()
                    {
                        filter_text.clear();
                    }
                });
                ui.data_mut(|data| data.insert_temp(filter_id, filter_text.clone()));
                ui.horizontal_wrapped(|ui| {
                    let shortcut =
                        egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F);
                    if ui
                        .button("Find…")
                        .on_hover_text(format!(
                            "Find an object with previews ({})",
                            ctx.format_shortcut(&shortcut)
                        ))
                        .clicked()
                    {
                        ag_iso_terminal_designer::object_picker::open(
                            &ctx,
                            egui::Id::new("find_project_object"),
                        );
                    }
                    ui.menu_button("Sort", |ui| {
                        if ui.button("Sort by name").clicked() {
                            pool.sort_objects_by_name();
                            ui.close();
                        }
                        if ui.button("Sort by ID").clicked() {
                            pool.sort_objects_by_id();
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Sort object list");
                });
                ui.separator();
                let query = filter_text.to_lowercase();
                let scroll = egui::ScrollArea::vertical();
                let scroll = if filter_text != previous_filter {
                    scroll.vertical_scroll_offset(0.0)
                } else {
                    scroll
                };
                scroll.show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    ui.data_mut(|data| {
                        data.insert_temp(egui::Id::new("tree_row_bounds"), ui.max_rect())
                    });
                    if !query.is_empty() {
                        render_filtered_object_list(ui, pool, &query, &mut self.mask_view);
                        ui.separator();
                    }
                    render_focused_mask_hierarchy(ui, pool, &mut self.mask_view, &mut self.canvas);
                    ui.allocate_space(ui.available_size());
                });
            }
            Pane::Graphics => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ag_iso_terminal_designer::graphics::ui::show(ui, pool)
                });
            }
            Pane::Components => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ag_iso_terminal_designer::components::ui::show(ui, pool)
                });
            }
            Pane::Properties => {
                let materialized = pool.materialize_shared();
                if let Some(
                    ag_iso_terminal_designer::components::EditorSelection::ComponentNode {
                        instance_id,
                        node_id,
                    },
                ) = pool.editor_selection()
                {
                    ag_iso_terminal_designer::components::ui::inspector(
                        ui,
                        pool,
                        instance_id,
                        node_id,
                    );
                } else if let Some(id) = pool.get_selected().into() {
                    if let Some(original) = pool.get_pool().object_by_id(id) {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            // Display editable object name as header
                            ui.horizontal(|ui| {
                                ui.label("Name:");

                                let object_info = pool.get_object_info(original);
                                let mut name = object_info.get_name(original);
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut name)
                                        .desired_width(ui.available_width()),
                                );

                                if response.changed() {
                                    pool.queue_operation(Operation::RenameObject {
                                        object: ag_iso_terminal_designer::ObjectTarget::id(
                                            original.id().value(),
                                        ),
                                        name,
                                    });
                                }
                            });
                            ui.separator();

                            render_property_editor(ui, original, pool);
                            let effective = materialized
                                .as_ref()
                                .map(|m| &m.pool)
                                .unwrap_or(pool.get_pool());
                            let rendered =
                                effective.object_by_id(original.id()).unwrap_or(original);
                            let (width, height) = effective.content_size(rendered);
                            ui.separator();
                            let desired_size = egui::Vec2::new(width as f32, height as f32);
                            ui.allocate_ui(desired_size, |ui| {
                                ag_iso_terminal_designer::render_object_with_version(
                                    ui,
                                    effective,
                                    rendered,
                                    Point::default(),
                                    pool.vt_version(),
                                );
                            });
                        });
                    } else {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!(
                                "Selected object not found: {}",
                                pool.format_object_id(u16::from(id))
                            ),
                        );
                    }
                }
                if pool.get_selected().0.is_none() {
                    ui.weak("Select an object in the tree or canvas to edit its properties.");
                }
                ui.allocate_space(ui.available_size());
            }
            Pane::Canvas => {
                self.canvas_visible = true;
                let materialized = pool.materialize_shared();
                match &materialized {
                    Ok(materialized) => {
                        self.mask_view.reconcile(pool);
                        self.canvas
                            .show_masks(ui, pool, materialized, &mut self.mask_view);
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    }
                }
            }
            Pane::Ancestry => {
                ui.horizontal(|ui| {
                    if ag_iso_terminal_designer::ui_style::icon_button(
                        ui,
                        if self.workspace.ancestry_expanded {
                            ag_iso_terminal_designer::ui_style::Icon::ChevronDown
                        } else {
                            ag_iso_terminal_designer::ui_style::Icon::ChevronRight
                        },
                        if self.workspace.ancestry_expanded {
                            "Collapse ancestry details"
                        } else {
                            "Expand ancestry details"
                        },
                    )
                    .clicked()
                    {
                        self.workspace.ancestry_expanded = !self.workspace.ancestry_expanded;
                    }
                    if let Some(id) = pool.get_selected().0 {
                        if let Some(object) = pool.get_pool().object_by_id(id) {
                            ui.label(format!(
                                "{} · {}",
                                pool.format_object_id(id.value()),
                                pool.object_display_name(object)
                            ));
                        }
                    } else {
                        ui.weak("Select an object to inspect its ancestry.");
                    }
                });
                if self.workspace.ancestry_expanded {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| render_selected_ancestry_panel(ui, pool));
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Pane::Proposal => unreachable!(),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn render_proposal(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(error) = &self.automation_bridge_error {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Automation bridge unavailable: {error}"),
                );
            } else if let Some(bridge) = &self.automation_bridge {
                ui.label(bridge.status());
                if let Some(path) = bridge.trace_path() {
                    ui.small(format!("Automation trace: {}", path.display()));
                }
                if let Some(error) = bridge.last_error() {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if let Some(proposal) = bridge.proposal() {
                    ui.separator();
                    ui.heading("Proposed change");
                    ui.label(
                        proposal
                            .transaction
                            .description
                            .as_deref()
                            .unwrap_or("Untitled change"),
                    );
                    if proposal.component_intents.is_empty() {
                        ui.label(format!(
                            "{} operations, {} affected objects",
                            proposal.summary.operation_count,
                            proposal.summary.affected_objects.len()
                        ));
                    } else {
                        ui.label(format!(
                            "{} component actions",
                            proposal.component_intents.len()
                        ));
                    }
                    for action in &proposal.component_intents {
                        use ag_iso_terminal_designer::components::ComponentIntent;
                        match action {
                            ComponentIntent::InstantiateComponent {
                                component_id,
                                x,
                                y,
                                overrides,
                                ..
                            } => {
                                let definition = &proposal.components.definitions[component_id];
                                ui.label(format!("Add {} at ({x}, {y})", definition.name));
                                for (id, value) in overrides {
                                    if let Some(parameter) =
                                        definition.parameters.iter().find(|p| p.id == *id)
                                    {
                                        ui.label(format!("{}: {value}", parameter.name));
                                    }
                                }
                            }
                            ComponentIntent::SetComponentOverride {
                                parameter, value, ..
                            } => {
                                ui.label(format!("Set declared parameter {parameter}: {value}"));
                            }
                            ComponentIntent::ResetComponentOverride { parameter, .. } => {
                                ui.label(format!("Reset declared parameter {parameter}"));
                            }
                            ComponentIntent::SetComponentMasterProperty {
                                component_id,
                                property,
                                value,
                                ..
                            } => {
                                ui.label(format!(
                                    "All {} instances: {property} = {value}",
                                    proposal.components.definitions[component_id].name
                                ));
                            }
                        }
                    }
                    ui.label("Validation passed");
                    if let Some(project) = self.project.as_ref() {
                        let design = project.ai_design_state();
                        if let Ok(preview) =
                            ag_iso_terminal_designer::ai::render_proposal_preview(&design, proposal)
                        {
                            ui.label(format!(
                                "Proposed mask preview: {} × {}",
                                preview.width, preview.height
                            ));
                            if preview.diagnostics.is_empty() {
                                ui.label("Design checks: no deterministic issues found");
                            } else {
                                ui.label(format!(
                                    "Design checks: {} issue(s)",
                                    preview.diagnostics.len()
                                ));
                                for diagnostic in preview.diagnostics.iter().take(4) {
                                    ui.label(format!(
                                        "{:?}: {}",
                                        diagnostic.severity, diagnostic.message
                                    ));
                                }
                            }
                            let _ = ag_iso_terminal_designer::ai::render_preview_ui(
                                ui, &design, proposal,
                            );
                        } else {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                "Could not render proposal preview.",
                            );
                        }
                    } else {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            "The project was closed; this proposal cannot be applied.",
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.project.is_some(), egui::Button::new("Apply"))
                            .clicked()
                        {
                            self.proposal_apply = true;
                        }
                        if ui.button("Reject").clicked() {
                            self.proposal_reject = true;
                        }
                    });
                } else {
                    ui.separator();
                    ui.label("No proposal is waiting for review.");
                    ui.label("Use aitd while this editor has a project open to stage a proposal.");
                }
            } else {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "Automation bridge did not start.",
                );
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn finish_proposal_actions(&mut self) {
        if std::mem::take(&mut self.proposal_reject) {
            if let Some(bridge) = &mut self.automation_bridge {
                bridge.reject_proposal();
            }
        }
        if std::mem::take(&mut self.proposal_apply) {
            if let (Some(project), Some(bridge)) = (&mut self.project, &mut self.automation_bridge)
            {
                if bridge
                    .proposal()
                    .is_some_and(|proposal| proposal.base_revision != project.revision())
                {
                    bridge.invalidate_proposal();
                    self.error_message = Some(
                            "The project changed after this proposal was staged. Request a new proposal.".into(),
                        );
                } else if let Some(proposal) = bridge.take_proposal() {
                    let proposal_id = proposal.proposal_id.clone();
                    match project.apply_ai_proposal(proposal) {
                        Ok(_) => bridge.record_applied(&proposal_id),
                        Err(diagnostics) => {
                            bridge.record_apply_failed(&proposal_id);
                            self.error_message =
                                Some(format!("Could not apply proposal: {diagnostics:?}"));
                        }
                    }
                }
            }
        }
    }

    fn render_workspace(&mut self, ctx: &egui::Context) {
        let mut dock =
            std::mem::replace(&mut self.workspace.dock, egui_dock::DockState::new(vec![]));
        self.canvas_visible = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ctx.style().visuals.faint_bg_color))
            .show(ctx, |ui| {
                egui_dock::DockArea::new(&mut dock)
                    .id(egui::Id::new("designer_dock"))
                    .show_leaf_collapse_buttons(false)
                    .style(egui_dock::Style::from_egui(ui.style()))
                    .window_bounds(ui.max_rect())
                    .show_inside(ui, &mut PaneViewer { app: self });
            });
        self.workspace.dock = dock;
        if !self.canvas_visible {
            self.canvas.cancel_interaction();
        }
    }
    fn show_view_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("View", |ui| {
            ui.menu_button("Theme", |ui| {
                for (theme, label) in [
                    (egui::ThemePreference::System, "System"),
                    (egui::ThemePreference::Light, "Light"),
                    (egui::ThemePreference::Dark, "Dark"),
                ] {
                    if ui
                        .selectable_value(&mut self.preferences.theme, theme, label)
                        .clicked()
                    {
                        ctx.set_theme(theme);
                    }
                }
            });
            ui.menu_button("Panels", |ui| {
                for &pane in Pane::ALL {
                    ui.horizontal(|ui| {
                        let mut visible = self.workspace.contains(pane);
                        if ui.checkbox(&mut visible, pane.title()).changed() {
                            if visible {
                                self.workspace.open(pane);
                            } else {
                                self.workspace.close(pane);
                            }
                        }
                        if visible
                            && ag_iso_terminal_designer::ui_style::icon_button(
                                ui,
                                ag_iso_terminal_designer::ui_style::Icon::ChevronRight,
                                "Activate pane",
                            )
                            .clicked()
                        {
                            self.workspace.open(pane);
                            ui.close();
                        }
                    });
                }
                ui.separator();
                if ui.button("Reset Panel Layout").clicked() {
                    self.workspace = Workspace::new(&AppPreferences::default());
                    self.canvas.cancel_interaction();
                    ctx.memory_mut(|memory| memory.reset_areas());
                    ui.close();
                }
            });
            ui.menu_button("Object IDs", |ui| {
                for (format, label) in [
                    (ag_iso_terminal_designer::ObjectIdFormat::Decimal, "Decimal"),
                    (
                        ag_iso_terminal_designer::ObjectIdFormat::Hexadecimal,
                        "Hexadecimal (0x00AF)",
                    ),
                ] {
                    ui.selectable_value(&mut self.preferences.object_id_format, format, label);
                }
            });
        });
    }
}
struct PaneViewer<'a> {
    app: &'a mut DesignerApp,
}
impl egui_dock::TabViewer for PaneViewer<'_> {
    type Tab = Pane;
    fn title(&mut self, pane: &mut Pane) -> egui::WidgetText {
        pane.title().into()
    }
    fn id(&mut self, pane: &mut Pane) -> egui::Id {
        egui::Id::new(("workspace_pane", *pane))
    }
    fn ui(&mut self, ui: &mut egui::Ui, pane: &mut Pane) {
        self.app.render_pane(ui, *pane);
    }
    fn scroll_bars(&self, _: &Pane) -> [bool; 2] {
        [false, false]
    }
}

/// Draw a tree selection across the pane while retaining the indented label and arrow.
fn tree_row(ui: &mut egui::Ui, selected: bool, label: impl Into<String>) -> egui::Response {
    let background = ui
        .data_mut(|data| {
            let key = egui::Id::new("tree_row_background");
            let value = data.get_temp::<egui::layers::ShapeIdx>(key);
            data.remove::<egui::layers::ShapeIdx>(key);
            value
        })
        .unwrap_or_else(|| ui.painter().add(egui::Shape::Noop));
    let bounds = ui
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("tree_row_bounds")))
        .unwrap_or(ui.clip_rect());
    let height = ui.spacing().interact_size.y;
    let (label_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(0.0), height),
        egui::Sense::hover(),
    );
    let row_rect = egui::Rect::from_min_max(
        egui::pos2(bounds.left(), label_rect.top()),
        egui::pos2(bounds.right(), label_rect.bottom()),
    );
    // The expand arrow owns its hit area; row selection starts at the label.
    let response = ui.interact(label_rect, ui.next_auto_id(), egui::Sense::click());
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter()
        .set(background, egui::Shape::rect_filled(row_rect, 0, fill));
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(row_rect.min, egui::vec2(3.0, row_rect.height())),
            0,
            ui.visuals().selection.stroke.color,
        );
    }
    let text = egui::RichText::new(label.into()).color(if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    });
    ui.put(
        label_rect,
        egui::Label::new(text)
            .truncate()
            .sense(egui::Sense::hover()),
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            row_rect.shrink(1.0),
            0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    response
}
