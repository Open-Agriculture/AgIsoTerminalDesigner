//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
mod app_persistence;
#[path = "cli/zip/ui.rs"]
mod cli_zip_export;
mod designer_settings;
#[cfg(test)]
mod hierarchy_tests;
mod shared_attributes;
mod workspace;
use workspace::{Pane, Workspace};

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::NullableObjectId;
use ag_iso_stack::object_pool::ObjectId;
use ag_iso_stack::object_pool::ObjectPool;
use ag_iso_stack::object_pool::ObjectType;
use ag_iso_terminal_designer::canvas::{
    edits::{self, PlacementTarget},
    mask_view::MaskViewState,
    CanvasState, HierarchyOccurrence,
};
use ag_iso_terminal_designer::components::{EditorParentRef, EditorSelection};
use ag_iso_terminal_designer::render_property_editor;
use ag_iso_terminal_designer::{ui_style, EditorProject, Operation};
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

#[derive(Clone, Default)]
struct HierarchyFrameSelection {
    session: uuid::Uuid,
    revision: u64,
    occurrences: Vec<HierarchyOccurrence>,
}

#[derive(Clone)]
struct HierarchyClick {
    root: u16,
    path: Vec<usize>,
    modifiers: egui::Modifiers,
    right_click: bool,
}

#[derive(Clone)]
struct GroupRequest {
    session: uuid::Uuid,
    revision: u64,
    root: u16,
    parent_path: Vec<usize>,
    targets: Vec<PlacementTarget>,
    object_type: ObjectType,
}

#[derive(Clone)]
struct HierarchyDragSelection {
    session: uuid::Uuid,
    revision: u64,
    targets: Vec<PlacementTarget>,
    valid: bool,
}

struct PendingHierarchySelection {
    session: uuid::Uuid,
    revision: u64,
    root: u16,
    parent_path: Vec<usize>,
    targets: Vec<PlacementTarget>,
}

impl PendingHierarchySelection {
    fn new(
        project: &EditorProject,
        root: u16,
        parent_path: Vec<usize>,
        targets: Vec<PlacementTarget>,
    ) -> Self {
        Self {
            session: project.ui_session_id(),
            revision: project.revision(),
            root,
            parent_path,
            targets,
        }
    }

    fn is_current(&self, project: &EditorProject) -> bool {
        self.session == project.ui_session_id() && self.revision == project.revision()
    }
}

fn hierarchy_selection_snapshot(ui: &egui::Ui) -> HierarchyFrameSelection {
    ui.data(|data| data.get_temp(egui::Id::new("hierarchy_frame_selection")))
        .unwrap_or_default()
}

fn begin_hierarchy_frame(ui: &egui::Ui, project: &EditorProject, canvas: &mut CanvasState) {
    canvas.begin_hierarchy_frame(project.revision());
    ui.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("hierarchy_frame_selection"),
            HierarchyFrameSelection {
                session: project.ui_session_id(),
                revision: project.revision(),
                occurrences: canvas.selected_hierarchy_occurrences().to_vec(),
            },
        )
    });
}

fn finish_hierarchy_click(ui: &egui::Ui, project: &EditorProject, canvas: &mut CanvasState) {
    let click = ui.data_mut(|data| {
        let key = egui::Id::new("hierarchy_click");
        let click = data.get_temp::<HierarchyClick>(key);
        data.remove::<HierarchyClick>(key);
        click
    });
    if let Some(click) = click {
        canvas.hierarchy_click(
            project,
            click.root,
            &click.path,
            click.modifiers,
            click.right_click,
        );
    }
}

fn hierarchy_selected(ui: &egui::Ui, root: u16, path: &[usize]) -> bool {
    hierarchy_selection_snapshot(ui)
        .occurrences
        .iter()
        .any(|row| row.root == root && row.path == path)
}

fn hierarchy_row_event(ui: &egui::Ui, response: &egui::Response, root: u16, path: &[usize]) {
    if response.clicked() || response.secondary_clicked() || response.drag_started() {
        ui.ctx().request_repaint();
        if response.drag_started() && hierarchy_selected(ui, root, path) {
            return;
        }
        let modifiers = if response.drag_started() {
            egui::Modifiers::NONE
        } else {
            ui.input(|i| i.modifiers)
        };
        let right_click = response.secondary_clicked();
        ui.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("hierarchy_click"),
                HierarchyClick {
                    root,
                    path: path.to_vec(),
                    modifiers,
                    right_click,
                },
            )
        });
    }
}

fn hierarchy_group_menu(
    ui: &mut egui::Ui,
    project: &EditorProject,
    root: u16,
    path: &[usize],
    source: Option<&TreeDragPayload>,
) -> usize {
    let snapshot = hierarchy_selection_snapshot(ui);
    let selected = snapshot
        .occurrences
        .iter()
        .any(|row| row.root == root && row.path == path);
    let (targets, parent_path, valid, count) = if selected {
        let rows = &snapshot.occurrences;
        let first = rows.first();
        let valid = first.is_some_and(|first| {
            first.parent_path.is_some()
                && rows.iter().all(|row| {
                    row.root == first.root
                        && row.parent_path == first.parent_path
                        && row.target.is_some()
                })
        });
        (
            rows.iter()
                .filter_map(|row| row.target.clone())
                .collect::<Vec<_>>(),
            first
                .and_then(|row| row.parent_path.clone())
                .unwrap_or_default(),
            valid,
            rows.len(),
        )
    } else {
        (
            source
                .map(|source| vec![source.target.clone()])
                .unwrap_or_default(),
            source
                .map(|source| source.source_parent_path.clone())
                .unwrap_or_default(),
            source.is_some(),
            1,
        )
    };
    if !valid {
        ui.add_enabled(false, egui::Button::new("Create parent for object(s)"))
            .on_disabled_hover_text("Select editable sibling occurrences under the same parent.");
    } else {
        ui.menu_button("Create parent for object(s)", |ui| {
            let candidates = edits::grouping_candidates(project, &targets);
            if candidates.is_empty() {
                ui.add_enabled(
                    false,
                    egui::Label::new("No compatible parent types for this VT version"),
                );
            }
            for candidate in candidates {
                let response = ui.add_enabled(
                    candidate.enabled,
                    egui::Button::new(format!("{:?}", candidate.object_type)),
                );
                if let Some(reason) = candidate.reason {
                    response.clone().on_disabled_hover_text(reason);
                }
                if response.clicked() {
                    ui.data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new("hierarchy_group_request"),
                            GroupRequest {
                                session: project.ui_session_id(),
                                revision: snapshot.revision,
                                root,
                                parent_path: parent_path.clone(),
                                targets: targets.clone(),
                                object_type: candidate.object_type,
                            },
                        )
                    });
                    ui.close();
                }
            }
        });
    }
    count
}

#[derive(Clone)]
struct TreeDragPayload {
    root: u16,
    source_parent_path: Vec<usize>,
    target: PlacementTarget,
    sibling_index: usize,
    parent_id: u16,
    occurrence_id: egui::Id,
    label: String,
    selection: Option<HierarchyDragSelection>,
}

#[derive(Clone)]
struct TreeDropCandidate {
    source: TreeDragPayload,
    root: u16,
    parent_id: u16,
    parent_path: Vec<usize>,
    insertion_index: Option<usize>,
    rect: egui::Rect,
    layer: egui::LayerId,
    clip_rect: egui::Rect,
    edge_y: Option<f32>,
}

fn hierarchy_drag_payload(ui: &egui::Ui, source: &TreeDragPayload) -> TreeDragPayload {
    let mut payload = source.clone();
    let snapshot = hierarchy_selection_snapshot(ui);
    payload.selection = Some(HierarchyDragSelection {
        session: snapshot.session,
        revision: snapshot.revision,
        targets: vec![source.target.clone()],
        valid: true,
    });
    if snapshot.occurrences.iter().any(|row| {
        row.target.as_ref() == Some(&source.target)
            && row.root == source.root
            && row.parent_path.as_ref() == Some(&source.source_parent_path)
    }) {
        let valid = snapshot.occurrences.iter().all(|row| {
            row.root == source.root
                && row.parent_path.as_ref() == Some(&source.source_parent_path)
                && row.target.is_some()
        });
        payload.selection = Some(HierarchyDragSelection {
            session: snapshot.session,
            revision: snapshot.revision,
            targets: snapshot
                .occurrences
                .iter()
                .filter_map(|row| row.target.clone())
                .collect(),
            valid,
        });
    }
    payload
}

/// Collect one geometric destination; validation and commit share the same edit path.
fn hierarchy_drag_row(
    ui: &mut egui::Ui,
    response: &egui::Response,
    source: Option<&TreeDragPayload>,
    inside: Option<(u16, u16, Vec<usize>)>,
) {
    if let Some(source) = source {
        response.dnd_set_drag_payload(hierarchy_drag_payload(ui, source));
    }
    let Some(dragged) = egui::DragAndDrop::payload::<TreeDragPayload>(ui.ctx()) else {
        return;
    };
    if !response.contains_pointer() {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let edge = if pointer.y < response.rect.top() + response.rect.height() * 0.25 {
        Some(false)
    } else if pointer.y > response.rect.bottom() - response.rect.height() * 0.25 {
        Some(true)
    } else {
        None
    };
    let destination = if let (Some(after), Some(source)) = (edge, source) {
        Some((
            source.root,
            source.parent_id,
            source.source_parent_path.clone(),
            Some(source.sibling_index + usize::from(after)),
            Some(if after {
                response.rect.bottom()
            } else {
                response.rect.top()
            }),
        ))
    } else {
        inside.map(|(root, parent, path)| (root, parent, path, None, None))
    };
    if let Some((root, parent_id, parent_path, insertion_index, edge_y)) = destination {
        let candidate = TreeDropCandidate {
            source: (*dragged).clone(),
            root,
            parent_id,
            parent_path,
            insertion_index,
            rect: response.rect,
            layer: ui.layer_id(),
            clip_rect: ui.clip_rect(),
            edge_y,
        };
        ui.data_mut(|data| data.insert_temp(egui::Id::new("tree_drop_candidate"), candidate));
    }
}

fn clone_tree_path(path: &[usize]) -> Vec<usize> {
    path.to_vec()
}

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

#[cfg(not(target_arch = "wasm32"))]
fn absolute_project_path(path: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

pub struct DesignerApp {
    project: Option<EditorProject>,
    canvas: CanvasState,
    mask_view: MaskViewState,
    file_channel: (Sender<FileEvent>, Receiver<FileEvent>),
    show_development_popup: bool,
    new_object_dialog: Option<(ObjectType, String)>,
    new_object_group: Option<GroupRequest>,
    pending_hierarchy_selection: Option<PendingHierarchySelection>,
    pending_iop_import: Option<(
        Vec<u8>,
        String,
        ag_iso_stack::object_pool::vt_version::VtVersion,
    )>,
    apply_smart_naming_on_import: bool,
    #[cfg(not(target_arch = "wasm32"))]
    project_path: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    original_source_path: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    pending_iop_source_path: Option<PathBuf>,
    project_display_name: String,
    preferences: AppPreferences,
    workspace: Workspace,
    canvas_visible: bool,
    project_settings_open: bool,
    shared_attributes: shared_attributes::SharedAttributes,
    reassign_ids_open: bool,
    reassign_include_overrides: bool,
    dimensions_draft: Option<DimensionsDraft>,
    pending_action: Option<PendingAction>,
    confirm_discard_recovery: Option<PendingAction>,
    save_then_continue: Option<PendingAction>,
    discard_on_replace: bool,
    open_in_progress: bool,
    save_in_progress: bool,
    cli_zip_export: Option<cli_zip_export::Export>,
    persistence: AppPersistence,
    recovery: Option<RecoveryRecord>,
    recovery_load_error: bool,
    error_message: Option<String>,
    property_name_draft: Option<ObjectNameDraft>,
    property_name_draft_seen: bool,
    add_object_query: String,
    add_object_menu_open: bool,
    add_object_menu_seen: bool,
    replace_dialog: Option<ReplaceDialog>,
    #[cfg(not(target_arch = "wasm32"))]
    recent_projects: Vec<RecentProject>,
    #[cfg(target_arch = "wasm32")]
    browser_dirty: Rc<Cell<bool>>,
    #[cfg(target_arch = "wasm32")]
    _before_unload: Option<Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>>,
}

#[derive(Clone)]
struct ReplaceDialog {
    source_id: u16,
    source_name: String,
    revision: u64,
    occurrence: Option<PlacementTarget>,
    compatible_ids: Option<Vec<u16>>,
    candidate_query: Option<String>,
}

impl DesignerApp {
    fn show_replace_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.replace_dialog.as_ref() else {
            return;
        };
        let Some(project) = self.project.as_ref() else {
            self.replace_dialog = None;
            return;
        };
        if project.revision() != dialog.revision
            || project
                .get_pool()
                .object_by_id(ObjectId::new(dialog.source_id).unwrap_or_default())
                .is_none()
            || project.get_selected().0.map(|id| id.value()) != Some(dialog.source_id)
            || self.canvas.selected_placement(project) != dialog.occurrence
            || (dialog.occurrence.is_none() && self.canvas.has_placement_selection(project))
        {
            self.replace_dialog = None;
            return;
        }
        let picker = egui::Id::new("replace_object_picker");
        let query = ctx.data(|data| {
            data.get_temp::<String>(picker.with("query"))
                .unwrap_or_default()
        });
        let needs_candidates = dialog.candidate_query.as_deref() != Some(query.as_str());
        let candidate_ids = if needs_candidates {
            let ids: Vec<_> = project
                .get_pool()
                .objects()
                .iter()
                .filter(|object| object.id().value() != dialog.source_id)
                .filter(|object| {
                    ag_iso_terminal_designer::object_picker::matches_object(project, object, &query)
                })
                .filter_map(|object| {
                    let id = object.id().value();
                    let mut trial = project.clone();
                    let result = if let Some(target) = &dialog.occurrence {
                        ag_iso_terminal_designer::replacement::replace_placement(
                            &mut trial, target, id,
                        )
                    } else {
                        trial.replace_object_references(dialog.source_id, id)
                    };
                    result.is_ok().then_some(id)
                })
                .collect();
            if let Some(dialog) = self.replace_dialog.as_mut() {
                dialog.candidate_query = Some(query.clone());
                dialog.compatible_ids = Some(ids.clone());
            }
            ids
        } else {
            dialog.compatible_ids.clone().unwrap_or_default()
        };
        let dialog = self.replace_dialog.as_ref().unwrap().clone();
        let selected = ag_iso_terminal_designer::object_picker::window(
            ctx,
            picker,
            "Replace with",
            |ui, query| {
                ag_iso_terminal_designer::object_picker::objects(
                    ui,
                    project,
                    query,
                    None,
                    false,
                    |object| candidate_ids.contains(&object.id().value()),
                )
            },
            |ui, choice| {
                let Some(target_id) = choice.0.map(|id| id.value()) else {
                    return false;
                };
                let Some(target_object) = project.get_pool().object_by_id(choice.0.unwrap()) else {
                    return false;
                };
                ui.label(if dialog.occurrence.is_some() {
                    "Replace this placement. The shared source object remains unchanged."
                } else {
                    "Redirect every local reference. Both object definitions remain."
                });
                ui.label(format!(
                    "Before: {} ({})",
                    dialog.source_name, dialog.source_id
                ));
                ui.label(format!(
                    "After: {} ({})",
                    project.object_display_name(target_object),
                    target_id
                ));
                ag_iso_terminal_designer::object_picker::review_preview(ui, |ui, rect| {
                    ag_iso_terminal_designer::object_picker::object_preview_with_settings(
                        ui,
                        rect,
                        project.get_pool(),
                        target_object,
                        project.vt_version(),
                        project.soft_key_size(),
                    );
                });
                ag_iso_terminal_designer::object_picker::impact::review_change(
                    ui,
                    project,
                    |candidate| {
                        if let Some(target) = &dialog.occurrence {
                            ag_iso_terminal_designer::replacement::replace_placement(
                                candidate, target, target_id,
                            )
                        } else {
                            candidate.replace_object_references(dialog.source_id, target_id)
                        }
                    },
                )
            },
        );
        let Some(target_id) = selected.and_then(|choice| choice.0.map(|id| id.value())) else {
            if !ag_iso_terminal_designer::object_picker::is_open(ctx, picker) {
                self.replace_dialog = None;
            }
            return;
        };
        let dialog = self.replace_dialog.take().unwrap();
        let Some(project) = self.project.as_mut() else {
            return;
        };
        let result = if let Some(target) = &dialog.occurrence {
            ag_iso_terminal_designer::replacement::replace_placement(project, target, target_id)
        } else {
            project.replace_object_references(dialog.source_id, target_id)
        };
        match result {
            Ok(()) => {
                project
                    .get_mut_selected()
                    .set(NullableObjectId::new(target_id));
                if let Some(target) = dialog.occurrence {
                    self.canvas.replaced_placement(project, &target, target_id);
                }
            }
            Err(error) => self.error_message = Some(format!("Could not replace object: {error}")),
        }
    }

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
            #[cfg(not(target_arch = "wasm32"))]
            let source_path = self.pending_iop_source_path.take();
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
                        self.original_source_path = source_path;
                    }
                    if self.discard_on_replace {
                        self.clear_recovery();
                    }
                    self.discard_on_replace = false;
                    match diagnostics {
                        Ok(diagnostics) if !diagnostics.is_empty() => {
                            self.error_message = Some(format!(
                                "Imported for repair. Resolve compatibility issues before IOP export:\n{}",
                                diagnostics
                                    .iter()
                                    .map(|diagnostic| self
                                        .project
                                        .as_ref()
                                        .unwrap()
                                        .format_validation_diagnostic(diagnostic))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            ));
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
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.pending_iop_source_path = None;
            }
            self.open_in_progress = false;
            self.discard_on_replace = false;
        }
    }
    fn show_id_reassignment(&mut self, ctx: &egui::Context) {
        if !self.reassign_ids_open {
            return;
        }
        let Some(project) = self.project.as_ref() else {
            self.reassign_ids_open = false;
            return;
        };
        let mut apply = None;
        let mut cancel = false;
        egui::Window::new("Reassign object IDs")
            .open(&mut self.reassign_ids_open)
            .default_width(620.0)
            .show(ctx, |ui| {
                ui.checkbox(&mut self.reassign_include_overrides, "Include manual overrides");
                ui.small("Manual overrides are preserved by default. External ECU code needs regenerated ID headers after reassignment.");
                match project.plan_id_reassignment(self.reassign_include_overrides) {
                    Ok(plan) => {
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            egui::Grid::new("id_reassignment_preview").striped(true).show(ui, |ui| {
                                ui.strong("Name");
                                ui.strong("Old ID");
                                ui.strong("New ID");
                                ui.strong("Assignment");
                                ui.end_row();
                                for row in &plan.rows {
                                    ui.label(&row.name);
                                    ui.monospace(project.format_object_id(row.old_id));
                                    ui.monospace(project.format_object_id(row.new_id));
                                    let assignment = if row.overridden && !plan.includes_overrides {
                                        "Override preserved"
                                    } else if row.overridden {
                                        "Override reassigned"
                                    } else if row.component_node.is_some() {
                                        "Component node"
                                    } else {
                                        "Automatic"
                                    };
                                    ui.label(assignment);
                                    ui.end_row();
                                }
                            });
                        });
                        if ui.add_enabled(!plan.mappings.is_empty(), egui::Button::new("Apply reassignment")).clicked() {
                            apply = Some(plan);
                        }
                    }
                    Err(error) => {
                        ui_style::error_text(ui, &error);
                    }
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        if let Some(plan) = apply {
            match self.project.as_mut().unwrap().apply_id_reassignment(plan) {
                Ok(_) => {
                    self.reassign_ids_open = false;
                    self.mask_view = MaskViewState::default();
                    self.canvas.cancel_interaction();
                }
                Err(error) => self.error_message = Some(error.to_string()),
            }
        }
        if cancel {
            self.reassign_ids_open = false;
        }
    }

    fn close_project_settings(&mut self) {
        self.project_settings_open = false;
        self.shared_attributes = shared_attributes::SharedAttributes::default();
        self.reassign_ids_open = false;
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
            new_object_group: None,
            pending_hierarchy_selection: None,
            pending_iop_import: None,
            apply_smart_naming_on_import: true, // Default to true for better UX
            #[cfg(not(target_arch = "wasm32"))]
            project_path: None,
            #[cfg(not(target_arch = "wasm32"))]
            original_source_path: None,
            #[cfg(not(target_arch = "wasm32"))]
            pending_iop_source_path: None,
            project_display_name: "Untitled project".to_owned(),
            preferences,
            workspace,
            canvas_visible: false,
            project_settings_open: false,
            shared_attributes: shared_attributes::SharedAttributes::default(),
            reassign_ids_open: false,
            reassign_include_overrides: false,
            dimensions_draft: None,
            pending_action: None,
            confirm_discard_recovery: None,
            save_then_continue: None,
            discard_on_replace: false,
            open_in_progress: false,
            save_in_progress: false,
            cli_zip_export: None,
            persistence,
            recovery,
            recovery_load_error,
            error_message,
            property_name_draft: None,
            property_name_draft_seen: false,
            add_object_query: String::new(),
            add_object_menu_open: false,
            add_object_menu_seen: false,
            replace_dialog: None,
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
                        let version = self
                            .project
                            .as_ref()
                            .map(EditorProject::vt_version)
                            .unwrap_or(ag_iso_stack::object_pool::vt_version::VtVersion::Version6);
                        self.pending_iop_import = Some((content, file_name, version));
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            self.pending_iop_source_path = Some(absolute_project_path(&path));
                        }
                    }
                    FileDialogReason::LoadProject => match EditorProject::load_project(content) {
                        Ok(project) => {
                            match project.compatibility_diagnostics() {
                                Ok(diagnostics) if !diagnostics.is_empty() => {
                                    self.error_message = Some(format!(
                                        "Opened for repair. Resolve compatibility issues before IOP export:\n{}",
                                        diagnostics
                                            .iter()
                                            .map(|diagnostic| project
                                                .format_validation_diagnostic(diagnostic))
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    ));
                                }
                                Err(error) => {
                                    self.error_message = Some(error.to_string());
                                }
                                _ => {}
                            }
                            self.open_in_progress = false;
                            self.close_project_settings();
                            self.canvas = CanvasState::default();
                            self.mask_view = MaskViewState::default();
                            self.project = Some(project);
                            self.project_display_name = file_name;
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                self.project_path = Some(absolute_project_path(&path));
                                self.original_source_path = self.project_path.clone();
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
                                        for property in [ag_iso_terminal_designer::PropertyKey::ActualWidth, ag_iso_terminal_designer::PropertyKey::ActualHeight, ag_iso_terminal_designer::PropertyKey::Width, ag_iso_terminal_designer::PropertyKey::Format, ag_iso_terminal_designer::PropertyKey::TransparencyColour, ag_iso_terminal_designer::PropertyKey::Options, ag_iso_terminal_designer::PropertyKey::Data] {
                                            pool.queue_operation(Operation::SetProperty {
                                                object: ag_iso_terminal_designer::ObjectTarget::id(id.value()),
                                                property, value: value[property.as_str()].clone(),
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
                        self.project_path = Some(absolute_project_path(&path));
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
                self.close_project_settings();
                self.canvas = CanvasState::default();
                self.mask_view = MaskViewState::default();
                self.project = Some(EditorProject::new_pool());
                self.project_display_name = "Untitled.aitp".to_owned();
                self.new_object_dialog = None;
                self.new_object_group = None;
                self.pending_hierarchy_selection = None;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.project_path = None;
                    self.original_source_path = None;
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
                            self.close_project_settings();
                            self.canvas = CanvasState::default();
                            self.mask_view = MaskViewState::default();
                            self.project = Some(project);
                            self.project_display_name = record.display_name;
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                self.project_path = None;
                                self.original_source_path = None;
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

    /// Open a file dialog to save a C header file with object IDs
    fn save_header(&mut self) {
        if let Some(project) = &self.project {
            let contents = match ag_iso_terminal_designer::cli::export::header_for_project(project)
            {
                Ok(contents) => contents,
                Err(error) => {
                    self.error_message =
                        Some(format!("Could not generate object ID header: {error}"));
                    return;
                }
            };
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

    fn save_cli_zip(&mut self, ctx: &egui::Context) {
        if self.cli_zip_export.is_some() {
            return;
        }
        let Some(project) = &self.project else {
            return;
        };
        self.cli_zip_export = Some(cli_zip_export::Export::new(project.clone(), ctx));
        ctx.request_repaint();
    }

    fn save_cli_zip_contents(&mut self, contents: Vec<u8>, ctx: &egui::Context) {
        let sender = self.file_channel.0.clone();
        let ctx = ctx.clone();
        let task = rfd::AsyncFileDialog::new()
            .set_file_name("project.zip")
            .add_filter("CLI project ZIP", &["zip"])
            .save_file();
        execute(async move {
            if let Some(file) = task.await {
                if let Err(error) = file.write(&contents).await {
                    let _ = sender.send(FileEvent::Failed(format!(
                        "Failed to save CLI ZIP: {error}"
                    )));
                    ctx.request_repaint();
                }
            }
        });
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

type ObjectNameDraft = (u128, u64, u16, String, String);

fn finish_object_name_on_switch(
    project: &mut EditorProject,
    name_draft: &mut Option<ObjectNameDraft>,
) {
    if let Some((draft_session, draft_revision, draft_id, baseline, draft)) = name_draft.clone() {
        let current_name = ObjectId::new(draft_id)
            .ok()
            .and_then(|id| project.get_pool().object_by_id(id))
            .map(|object| project.get_object_info(object).get_name(object));
        let switching = draft_session != project.ui_session_id().as_u128()
            || draft_revision != project.revision()
            || project.get_selected().0.map(|id| id.value()) != Some(draft_id)
            || matches!(
                project.editor_selection(),
                Some(EditorSelection::ComponentNode { .. })
            )
            || current_name.as_deref() != Some(baseline.as_str());
        if switching {
            *name_draft = None;
            if draft_session == project.ui_session_id().as_u128()
                && draft_revision == project.revision()
                && current_name.as_deref() == Some(baseline.as_str())
            {
                if let Ok(draft_id) = ObjectId::new(draft_id) {
                    project.rename_object(draft_id, draft);
                }
            }
        }
    }
}

fn render_object_name(
    ui: &mut egui::Ui,
    project: &mut EditorProject,
    original: &Object,
    draft: &mut Option<ObjectNameDraft>,
) {
    let project_key = project.ui_session_id().as_u128();
    let project_revision = project.revision();
    let object_id = original.id().value();
    let original_name = project.get_object_info(original).get_name(original);
    ui.horizontal(|ui| {
        ui.label("Name:");
        let mut name = draft
            .as_ref()
            .filter(|(key, revision, draft_id, _, _)| {
                *key == project_key && *revision == project_revision && *draft_id == object_id
            })
            .map(|(_, _, _, _, draft)| draft.clone())
            .unwrap_or_else(|| original_name.clone());
        let response = ui.add(
            egui::TextEdit::singleline(&mut name)
                .id(egui::Id::new((
                    "property_object_name",
                    project_key,
                    object_id,
                )))
                .desired_width(ui.available_width()),
        );
        if response.changed() {
            *draft = Some((
                project_key,
                project_revision,
                object_id,
                original_name.clone(),
                name.clone(),
            ));
        }
        let active = response.has_focus() || response.lost_focus();
        let enter = active && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let escape = active && ui.input(|input| input.key_pressed(egui::Key::Escape));
        if escape {
            *draft = None;
            response.surrender_focus();
        } else if enter || response.lost_focus() {
            if let Some((_, _, _, _, draft)) = draft.take() {
                if draft != original_name {
                    if let Ok(object_id) = ObjectId::new(object_id) {
                        project.rename_object(object_id, draft);
                    }
                }
            }
        }
    });
}

fn duplicate_standalone_object(project: &mut EditorProject, source_id: u16) -> bool {
    let Ok(source_object_id) = ObjectId::new(source_id) else {
        return false;
    };
    let Some(source) = project.get_pool().object_by_id(source_object_id).cloned() else {
        return false;
    };
    let Ok(new_id) = project.allocate_object_id(source.object_type()) else {
        return false;
    };
    let source_info = project.get_object_info(&source);
    let base_name = format!("Copy of {}", source_info.get_name(&source));
    let names: HashSet<_> = project
        .get_pool()
        .objects()
        .iter()
        .map(|object| project.get_object_info(object).get_name(object))
        .collect();
    let mut name = base_name.clone();
    let mut suffix = 2;
    while names.contains(&name) {
        name = format!("{base_name} ({suffix})");
        suffix += 1;
    }
    let mut duplicate = source;
    if duplicate.mut_id().set_value(new_id.value()).is_err() {
        return false;
    }
    let mut info = ag_iso_terminal_designer::ObjectInfo::new(&duplicate);
    info.name = Some(name);
    info.description = source_info.description;
    info.notes = source_info.notes;
    info.tags = source_info.tags;
    info.allowed_references = source_info.allowed_references;
    project.queue_operation(Operation::RestoreObject {
        object: duplicate,
        info: Some(info),
    });
    project
        .get_mut_selected()
        .set(NullableObjectId::new(new_id.value()));
    true
}

fn render_selectable_object(
    ui: &mut egui::Ui,
    object: &Object,
    project: &EditorProject,
    drag_payload: Option<TreeDragPayload>,
    drop_destination: Option<(u16, u16, Vec<usize>)>,
    primary: bool,
) -> bool {
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
        let confirmed = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if cancelled {
            project.finish_renaming_object(false);
            response.surrender_focus();
        } else if confirmed || response.lost_focus() {
            project.finish_renaming_object(true);
        } else if !response.has_focus() {
            // We need to focus the text edit when we start renaming
            response.request_focus();
        }
        false
    } else {
        let occurrence_selected = drop_destination
            .as_ref()
            .is_some_and(|(root, _, path)| hierarchy_selected(ui, *root, path));
        let is_selected = if drop_destination.is_some()
            && !hierarchy_selection_snapshot(ui).occurrences.is_empty()
        {
            occurrence_selected
        } else {
            project.get_selected() == object.id().into()
        };
        let label_text = project.object_display_name(object);
        let response = tree_row_with_highlight(
            ui,
            ui.id().with(("object_row", object.id().value())),
            is_selected,
            primary,
            label_text,
        );
        hierarchy_drag_row(
            ui,
            &response,
            drag_payload.as_ref(),
            drop_destination.clone(),
        );
        if let Some((root, _, path)) = &drop_destination {
            hierarchy_row_event(ui, &response, *root, path);
        }

        if response.clicked() {
            project
                .get_mut_selected()
                .set(NullableObjectId(Some(object.id())));
        }
        if response.double_clicked() {
            project.set_renaming_object(this_ui_id, object.id(), object_info.get_name(object));
        }
        if is_selected
            && ui.input(|input| input.key_pressed(egui::Key::F2))
            && (!ui.ctx().wants_keyboard_input() || response.has_focus())
        {
            ui.ctx().input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::F2);
            });
            project.set_renaming_object(this_ui_id, object.id(), object_info.get_name(object));
        }

        response.context_menu(|ui| {
            let count = if let Some((root, _, path)) = &drop_destination {
                hierarchy_group_menu(ui, project, *root, path, drag_payload.as_ref())
            } else {
                1
            };
            if ui
                .add_enabled(count == 1, egui::Button::new("Rename"))
                .on_hover_text("Rename object")
                .clicked()
            {
                project.set_renaming_object(this_ui_id, object.id(), object_info.get_name(object));
                ui.close();
            }
            if ui
                .add_enabled(count == 1, egui::Button::new("Delete"))
                .on_hover_text("Delete object")
                .clicked()
            {
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
    canvas: &mut CanvasState,
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
        if render_selectable_object(ui, object, project, None, None, true) {
            canvas.select_all_hierarchy_references(Some(object.id().value()));
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
    placement_source: Option<TreeDragPayload>,
) {
    let occurrence_id = parent_id.with((root, path.clone()));
    ui.push_id(occurrence_id, |ui| {
        if egui::DragAndDrop::payload::<TreeDragPayload>(ui.ctx())
            .is_some_and(|source| source.occurrence_id == occurrence_id)
        {
            ui.multiply_opacity(0.4);
        }
        render_mask_object_hierarchy_inner(
            ui,
            parent_id,
            root,
            object,
            project,
            mask_view,
            canvas,
            path,
            visiting,
            softkey_slot,
            open_selection,
            placement_source,
        );
    });
}

fn render_mask_object_hierarchy_inner(
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
    placement_source: Option<TreeDragPayload>,
) {
    let id_value = object.id().value();
    if !visiting.insert(id_value) {
        ui_style::error_text(
            ui,
            &format!("Cycle detected at {}", project.format_object_id(id_value)),
        );
        return;
    }
    let component_instances: Vec<_> = project
        .component_source()
        .instances
        .iter()
        .filter(|instance| instance.parent == EditorParentRef::Primitive(id_value))
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
    canvas.register_hierarchy_occurrence(HierarchyOccurrence {
        root,
        path: path.clone(),
        object: id_value,
        target: placement_source
            .as_ref()
            .map(|source| source.target.clone()),
        parent_path: placement_source
            .as_ref()
            .map(|source| source.source_parent_path.clone()),
    });
    let primary = canvas.hierarchy_occurrence_is_primary(project, root, path, id_value);
    let reveal = ui
        .data(|data| data.get_temp::<(u16, Vec<usize>)>(egui::Id::new("hierarchy_reveal_path")))
        .is_some_and(|(reveal_root, reveal_path)| {
            reveal_root == root && reveal_path.starts_with(path)
        });
    let expand = reveal
        || open_selection
            && selected_id.is_some_and(|selected| {
                selected != id_value && {
                    mask_branch_contains(project, object, selected, &mut HashSet::new())
                }
            });

    if has_children {
        let selected_in_header = std::cell::Cell::new(false);
        let header_path = path.clone();
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
                if render_selectable_object(
                    ui,
                    object,
                    project,
                    placement_source.clone(),
                    Some((root, id_value, clone_tree_path(&header_path))),
                    primary,
                ) {
                    selected_in_header.set(true);
                }
            },
            |ui| {
                let mut children: Vec<_> =
                    (0..placed.len()).map(|index| (Some(index), None)).collect();
                let mut instances = component_instances;
                instances.sort_by_key(|instance| {
                    (instance.child_index.unwrap_or(usize::MAX), instance.id)
                });
                for instance in instances {
                    let index = instance
                        .child_index
                        .unwrap_or(children.len())
                        .min(children.len());
                    children.insert(index, (None, Some(instance)));
                }
                for (sibling_index, (primitive_index, instance)) in children.into_iter().enumerate()
                {
                    if let Some(index) = primitive_index {
                        let reference = &placed[index];
                        let source_parent_path = clone_tree_path(path);
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
                                Some(TreeDragPayload {
                                    root,
                                    selection: None,
                                    source_parent_path,
                                    sibling_index,
                                    parent_id: id_value,
                                    occurrence_id: occurrence_id
                                        .with(index)
                                        .with((root, path.clone())),
                                    label: project.object_display_name(child),
                                    target: PlacementTarget::Primitive {
                                        parent_id: id_value,
                                        child_index: index,
                                    },
                                }),
                            );
                        } else {
                            ui_style::error_text(
                                ui,
                                &format!("Missing placed object: {}", reference.id.value()),
                            );
                        }
                        path.pop();
                    } else if let Some(instance) = instance {
                        let definition =
                            &project.component_source().definitions[&instance.component_id];
                        let component_id = occurrence_id.with(("component", instance.id));
                        let payload = TreeDragPayload {
                            root,
                            selection: None,
                            source_parent_path: path.clone(),
                            sibling_index,
                            parent_id: id_value,
                            occurrence_id: component_id,
                            label: definition.name.clone(),
                            target: PlacementTarget::Component {
                                instance_id: instance.id,
                            },
                        };
                        let opens_component = open_selection
                            && selected_id.is_some_and(|selected| {
                                instance.compiled_ids.values().any(|id| *id == selected)
                            });
                        if opens_component {
                            let mut state =
                                egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(),
                                    component_id,
                                    true,
                                );
                            state.set_open(true);
                            state.store(ui.ctx());
                        }
                        let mut component_path = path.clone();
                        component_path.extend([usize::MAX - 1, sibling_index]);
                        let component_object = instance.compiled_ids[&definition.root_node];
                        canvas.register_hierarchy_occurrence(HierarchyOccurrence {
                            root,
                            path: component_path.clone(),
                            object: component_object,
                            target: Some(payload.target.clone()),
                            parent_path: Some(path.clone()),
                        });
                        let component_selected =
                            canvas.hierarchy_occurrence_selected(root, &component_path);
                        ui.push_id(component_id, |ui| {
                            if egui::DragAndDrop::payload::<TreeDragPayload>(ui.ctx())
                                .is_some_and(|source| source.occurrence_id == component_id)
                            {
                                ui.multiply_opacity(0.4);
                            }
                            let header_clicked = std::cell::Cell::new(false);
                            ag_iso_terminal_designer::ui_style::collapsing_section(
                                ui,
                                component_id,
                                opens_component,
                                |ui| {
                                    let selected = component_selected ||
                                        matches!(project.editor_selection(),
                                            Some(EditorSelection::ComponentInstance(id))
                                            | Some(EditorSelection::ComponentNode { instance_id: id, .. })
                                            if id == instance.id);
                                    let row = tree_row_with_id(
                                        ui,
                                        component_id.with("row"),
                                        selected,
                                        &definition.name,
                                    );
                                    hierarchy_drag_row(ui, &row, Some(&payload), None);
                                    hierarchy_row_event(ui, &row, root, &component_path);
                                    row.context_menu(|ui| { hierarchy_group_menu(ui, project, root, &component_path, Some(&payload)); });
                                    header_clicked.set(row.clicked());
                                },
                                |ui| {
                                    for node in &definition.nodes {
                                        let node_id = instance.compiled_ids[&node.id];
                                        let row = tree_row_with_id(
                                            ui,
                                            component_id.with(node.id),
                                            project.get_selected().0.is_some_and(|selected| {
                                                selected.value() == node_id
                                            }),
                                            &node.name,
                                        );
                                        // A compiled node still drags its complete owning instance.
                                        row.dnd_set_drag_payload(hierarchy_drag_payload(ui, &payload));
                                        if row.clicked() {
                                            project
                                                .get_mut_selected()
                                                .set(NullableObjectId::new(node_id));
                                            if let Some(slot) = softkey_slot {
                                                canvas.select_softkey_slot(project, root, slot);
                                            }
                                            canvas.select_hierarchy_placement(
                                                project, root, &payload.source_parent_path, payload.target.clone(),
                                            );
                                        }
                                    }
                                },
                            );
                            if header_clicked.get() {
                                if let Some(slot) = softkey_slot {
                                    canvas.select_softkey_slot(project, root, slot);
                                }

                            }
                        });
                    }
                }
                if let Some(target_id) = pointer_target {
                    let pointer_branch = occurrence_id.with("pointer-target");
                    let target = ag_iso_stack::object_pool::ObjectId::new(target_id)
                        .ok()
                        .and_then(|id| project.get_pool().object_by_id(id));
                    match target {
                        Some(target) => {
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
                                None,
                            );
                            path.pop();
                        }
                        None => {
                            ui_style::error_text(
                                ui,
                                &format!("Missing active pointer target: {target_id}"),
                            );
                        }
                    };
                }
            },
        );
        if selected_in_header.get() {
            mask_view.navigate(project, id_value);
            if let Some(slot) = softkey_slot {
                canvas.select_softkey_slot(project, root, slot);
            }
        }
    } else if render_selectable_object(
        ui,
        object,
        project,
        placement_source.clone(),
        Some((root, id_value, clone_tree_path(path))),
        primary,
    ) {
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
        .filter(|instance| instance.parent == EditorParentRef::Primitive(id))
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
            None,
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
                        if render_selectable_object(ui, softkey_mask, project, None, None, true) {
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
                                    ui_style::error_text(
                                        ui,
                                        &format!(
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
                ui_style::error_text(ui, &format!("Object {softkey_id} is not a SoftKeyMask"));
            }
            None => {
                ui_style::error_text(ui, &format!("Missing linked soft-key mask: {softkey_id}"));
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
                let label = format!("{kind}: {}", project.object_display_name(mask));
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
        ui_style::error_text(
            ui,
            &format!(
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
            render_selectable_object(ui, object, project, None, None, true);
        });
        return;
    }

    let id = parent_id.with(project.get_object_info(object).get_unique_id());
    ag_iso_terminal_designer::ui_style::collapsing_section(
        ui,
        id,
        true,
        |ui| {
            render_selectable_object(ui, object, project, None, None, true);
        },
        |ui| {
            for (idx, parent) in parents.iter().enumerate() {
                if path_guard.contains(parent) {
                    ui_style::error_text(
                        ui,
                        &format!(
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
        ui_style::error_text(
            ui,
            &format!(
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
        self.canvas.set_grid_visible(self.preferences.show_grid);
        self.add_object_menu_seen = false;
        self.property_name_draft_seen = false;
        ctx.data_mut(|data| data.remove::<bool>(egui::Id::new("tree_row_had_focus")));
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
            project.set_show_object_ids(self.preferences.show_object_ids);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.cli_zip_export.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.project.as_ref().is_some_and(EditorProject::is_dirty) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if self.save_in_progress {
                    self.save_then_continue = Some(PendingAction::Exit);
                } else if self.pending_action.is_none() && self.save_then_continue.is_none() {
                    self.pending_action = Some(PendingAction::Exit);
                }
            }
        }

        if let Some(export) = &mut self.cli_zip_export {
            export.show(ctx);
            let outcome = export.poll();
            if let Some(outcome) = outcome {
                self.cli_zip_export = None;
                match outcome {
                    cli_zip_export::Outcome::Complete(contents) => {
                        self.save_cli_zip_contents(contents, ctx);
                    }
                    cli_zip_export::Outcome::Cancelled => (),
                    cli_zip_export::Outcome::Failed(error) => {
                        self.error_message = Some(format!("Could not generate CLI ZIP: {error}"));
                    }
                }
            }
            ctx.request_repaint();
            #[cfg(target_arch = "wasm32")]
            self.browser_dirty
                .set(self.project.as_ref().is_some_and(EditorProject::is_dirty));
            return;
        }

        if let Some(error) = self.error_message.clone() {
            egui::Window::new("Something went wrong")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui_style::error_text(ui, &error);
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
                if let Some(pool) = &mut self.project {
                    if let Some(group) = self.new_object_group.take() {
                        let result = if pool.ui_session_id() != group.session
                            || pool.revision() != group.revision
                        {
                            Err("The project changed while the parent dialog was open. Select the occurrences again.".to_owned())
                        } else {
                            edits::group_placements(pool, &group.targets, object_type, &name)
                        };
                        match result {
                            Ok(target) => {
                                self.canvas.select_hierarchy_placement(
                                    pool,
                                    group.root,
                                    &group.parent_path,
                                    target.clone(),
                                );
                                self.pending_hierarchy_selection =
                                    Some(PendingHierarchySelection::new(
                                        pool,
                                        group.root,
                                        group.parent_path,
                                        vec![target],
                                    ));
                            }
                            Err(error) => self.error_message = Some(error),
                        }
                    } else if let Some(id) = pool.create_object(object_type, name) {
                        pool.get_mut_selected()
                            .set(NullableObjectId::new(id.value()));
                    }
                }
                self.new_object_dialog = None;
            } else if should_cancel {
                self.new_object_dialog = None;
                self.new_object_group = None;
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
                        if ui.button("Export CLI ZIP…").clicked() {
                            self.save_cli_zip(ctx);
                            ui.close();
                        }
                    });
                });
                let add_object_shortcut =
                    egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::N);
                let open_add_object = self.project.is_some()
                    && self.new_object_dialog.is_none()
                    && ctx.input_mut(|input| input.consume_shortcut(&add_object_shortcut));
                let project_menu = ui.menu_button("Project", |ui| {
                    ui.add_enabled_ui(self.project.is_some(), |ui| {
                        egui::containers::menu::SubMenuButton::from_button(
                            egui::Button::new("Add Object").right_text(format!(
                                "{}  ⏵",
                                ctx.format_shortcut(&add_object_shortcut),
                            )),
                        )
                        .ui(ui, |ui| self.show_add_object_menu(ui));
                        if ui.button("Reassign object IDs…").clicked() {
                            self.reassign_ids_open = true;
                            self.reassign_include_overrides = false;
                            ui.close();
                        }
                        if ui.button("Project Settings…").clicked() {
                            self.project_settings_open = true;
                            self.dimensions_draft =
                                self.project.as_ref().map(DimensionsDraft::from_project);
                            ui.close();
                        }
                    });
                });
                if self.project.is_some() {
                    let popup_id = egui::Id::new("add_object_shortcut_menu");
                    if open_add_object {
                        egui::Popup::open_id(ctx, popup_id);
                    }
                    egui::Popup::menu(&project_menu.response)
                        .id(popup_id)
                        .open_memory(None)
                        .show(|ui| self.show_add_object_menu(ui));
                }
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
        self.add_object_menu_open = self.add_object_menu_seen;

        self.show_id_reassignment(ctx);
        self.show_replace_dialog(ctx);

        let requested_pane = designer_settings::show_project_settings(
            ctx,
            &mut self.project_settings_open,
            self.project.as_mut(),
            &mut self.preferences,
            &mut self.dimensions_draft,
            &mut self.error_message,
        );
        match requested_pane {
            Some(designer_settings::SettingsAction::OpenPane(pane)) => self.workspace.open(pane),
            Some(designer_settings::SettingsAction::SharedAttributes) => {
                if let Some(project) = self.project.as_ref() {
                    self.shared_attributes.open(ctx, project);
                }
            }
            None => {}
        }
        if let Some(project) = self.project.as_mut() {
            if self
                .shared_attributes
                .show(ctx, project, &mut self.error_message)
            {
                self.workspace.open(Pane::Properties);
            }
        }

        if self.project.is_some() {
            self.render_workspace(ctx);
        } else {
            self.render_startup(ctx);
        }
        self.preferences.show_grid = self.canvas.grid_visible();
        let tree_row_focused = ctx.data(|data| {
            data.get_temp::<bool>(egui::Id::new("tree_row_had_focus"))
                .unwrap_or(false)
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Delete))
            && (!ctx.wants_keyboard_input() || tree_row_focused)
            && self.new_object_dialog.is_none()
        {
            if let Some(project) = self.project.as_mut() {
                if !self.canvas.remove_selection(project) {
                    if !self.canvas.has_placement_selection(project) {
                        if let Some(id) = project.get_selected().0.map(|id| id.value()) {
                            if project
                                .get_pool()
                                .object_by_id(ObjectId::new(id).unwrap())
                                .is_some_and(|object| {
                                    !matches!(
                                        object,
                                        Object::DataMask(_)
                                            | Object::AlarmMask(_)
                                            | Object::SoftKeyMask(_)
                                    )
                                })
                            {
                                if let Ok(id) = ObjectId::new(id) {
                                    project.request_delete_object(id);
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(pool) = &mut self.project {
            pool.set_object_id_format(self.preferences.object_id_format);
            pool.set_show_object_ids(self.preferences.show_object_ids);
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
                self.canvas
                    .select_all_hierarchy_references(selected.0.map(|id| id.value()));
                explicit_navigation = true;
            }

            if !self.property_name_draft_seen {
                if let Some((session, revision, draft_id, baseline, draft)) =
                    self.property_name_draft.take()
                {
                    let unchanged = session == pool.ui_session_id().as_u128()
                        && revision == pool.revision()
                        && ObjectId::new(draft_id)
                            .ok()
                            .and_then(|id| pool.get_pool().object_by_id(id))
                            .is_some_and(|object| {
                                pool.get_object_info(object).get_name(object) == baseline
                            });
                    if unchanged {
                        if let Ok(id) = ObjectId::new(draft_id) {
                            pool.rename_object(id, draft);
                        }
                    }
                }
            }
            ag_iso_terminal_designer::finish_property_edits(ctx, pool);
            ag_iso_terminal_designer::components::ui::finish_inspector_edits(ctx, pool);
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
                        render_filtered_object_list(
                            ui,
                            pool,
                            &query,
                            &mut self.mask_view,
                            &mut self.canvas,
                        );
                        ui.separator();
                    }
                    begin_hierarchy_frame(ui, pool, &mut self.canvas);
                    if self
                        .pending_hierarchy_selection
                        .as_ref()
                        .is_some_and(|selection| !selection.is_current(pool))
                    {
                        self.pending_hierarchy_selection = None;
                    }
                    if let Some(selection) = &self.pending_hierarchy_selection {
                        ui.data_mut(|data| {
                            data.insert_temp(
                                egui::Id::new("hierarchy_reveal_path"),
                                (selection.root, selection.parent_path.clone()),
                            )
                        });
                    }
                    render_focused_mask_hierarchy(ui, pool, &mut self.mask_view, &mut self.canvas);
                    if let Some(selection) = self.pending_hierarchy_selection.take() {
                        self.canvas.select_hierarchy_placements(
                            pool,
                            selection.root,
                            &selection.parent_path,
                            &selection.targets,
                        );
                        ui.data_mut(|data| {
                            data.remove::<(u16, Vec<usize>)>(egui::Id::new("hierarchy_reveal_path"))
                        });
                    }
                    finish_hierarchy_click(ui, pool, &mut self.canvas);
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
                finish_object_name_on_switch(pool, &mut self.property_name_draft);
                if let Some(EditorSelection::ComponentNode {
                    instance_id,
                    node_id,
                }) = pool.editor_selection()
                {
                    ag_iso_terminal_designer::components::ui::inspector(
                        ui,
                        pool,
                        instance_id,
                        node_id,
                    );
                } else if let Some(id) = pool.get_selected().into() {
                    if let Some(original) = pool.get_pool().object_by_id(id).cloned() {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let object_id = original.id().value();
                            let has_placement = self.canvas.has_placement_selection(pool);
                            let placement = self.canvas.selected_placement(pool);
                            let placement_supported =
                                matches!(placement, Some(PlacementTarget::Primitive { .. }));
                            let replacement_enabled = !has_placement || placement_supported;
                            let duplicate_enabled = placement.is_some() || !has_placement;
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(duplicate_enabled, egui::Button::new("Duplicate"))
                                    .clicked()
                                {
                                    if !self.canvas.duplicate_selection(pool) {
                                        if !has_placement {
                                            duplicate_standalone_object(pool, object_id);
                                        }
                                    }
                                }
                                if ui.button("Delete").clicked() {
                                    if !self.canvas.remove_selection(pool) {
                                        if !self.canvas.has_placement_selection(pool)
                                            && !matches!(
                                                &original,
                                                Object::DataMask(_)
                                                    | Object::AlarmMask(_)
                                                    | Object::SoftKeyMask(_)
                                            )
                                        {
                                            pool.request_delete_object(original.id());
                                        }
                                    }
                                }
                                if ui
                                    .add_enabled(
                                        replacement_enabled,
                                        egui::Button::new("Replace with…"),
                                    )
                                    .clicked()
                                {
                                    ag_iso_terminal_designer::object_picker::open(
                                        ui.ctx(),
                                        egui::Id::new("replace_object_picker"),
                                    );
                                    self.replace_dialog = Some(ReplaceDialog {
                                        source_id: object_id,
                                        source_name: pool
                                            .get_object_info(&original)
                                            .get_name(&original)
                                            .to_owned(),
                                        revision: pool.revision(),
                                        occurrence: if placement_supported {
                                            placement.clone()
                                        } else {
                                            None
                                        },
                                        compatible_ids: None,
                                        candidate_query: None,
                                    });
                                }
                            });
                            self.property_name_draft_seen = true;
                            render_object_name(ui, pool, &original, &mut self.property_name_draft);
                            ui.separator();

                            ui.push_id(
                                ("property_occurrence", self.canvas.selected_placement(pool)),
                                |ui| {
                                    render_property_editor(ui, &original, pool);
                                },
                            );
                            let effective = materialized
                                .as_ref()
                                .map(|m| &m.pool)
                                .unwrap_or(pool.get_pool());
                            let rendered =
                                effective.object_by_id(original.id()).unwrap_or(&original);
                            let (width, height) = effective.content_size(rendered);
                            ui.separator();
                            let desired_size = egui::Vec2::new(width as f32, height as f32);
                            ui.allocate_ui(desired_size, |ui| {
                                ag_iso_terminal_designer::render_object_with_settings(
                                    ui,
                                    effective,
                                    rendered,
                                    Point::default(),
                                    pool.vt_version(),
                                    pool.soft_key_size(),
                                );
                            });
                        });
                    } else {
                        ui_style::error_text(
                            ui,
                            &format!(
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
                        ui_style::error_text(ui, &error.to_string());
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
                            ui.label(pool.object_display_name(object));
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
        }
    }
    fn finish_hierarchy_drag(&mut self, ctx: &egui::Context) {
        let Some(payload) = egui::DragAndDrop::payload::<TreeDragPayload>(ctx) else {
            return;
        };
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            egui::DragAndDrop::clear_payload(ctx);
            return;
        }
        let candidate = ctx
            .data(|data| data.get_temp::<TreeDropCandidate>(egui::Id::new("tree_drop_candidate")));
        let valid = candidate.as_ref().is_some_and(|candidate| {
            let Some(project) = self.project.as_ref() else {
                return false;
            };
            if let Some(selection) = &payload.selection {
                if !selection.valid
                    || selection.session != project.ui_session_id()
                    || selection.revision != project.revision()
                {
                    return false;
                }
                if selection.targets.len() > 1 {
                    let same_occurrence = candidate.root == payload.root
                        && candidate.parent_path == payload.source_parent_path;
                    if candidate.parent_id == payload.parent_id && !same_occurrence {
                        return false;
                    }
                    let delta = self
                        .canvas
                        .hierarchy_child_origin(project, payload.root, &payload.source_parent_path)
                        .and_then(|source| {
                            self.canvas
                                .hierarchy_child_origin(
                                    project,
                                    candidate.root,
                                    &candidate.parent_path,
                                )
                                .map(|dest| source - dest)
                        });
                    return delta.is_ok_and(|delta| {
                        edits::validate_move_placements(
                            project,
                            &selection.targets,
                            candidate.parent_id,
                            candidate.insertion_index,
                            (delta.x.round() as i32, delta.y.round() as i32),
                        )
                        .is_ok()
                    });
                }
            }
            self.canvas
                .validate_hierarchy_drop(
                    project,
                    &candidate.source.target,
                    candidate.source.root,
                    &candidate.source.source_parent_path,
                    candidate.root,
                    &candidate.parent_path,
                    candidate.parent_id,
                    candidate.insertion_index,
                )
                .is_ok()
        });
        if valid {
            let candidate = candidate.as_ref().unwrap();
            let painter = ctx
                .layer_painter(candidate.layer)
                .with_clip_rect(candidate.clip_rect);
            let stroke = egui::Stroke::new(2.0, ctx.style().visuals.selection.stroke.color);
            if let Some(y) = candidate.edge_y {
                painter.line_segment(
                    [
                        egui::pos2(candidate.rect.left(), y),
                        egui::pos2(candidate.rect.right(), y),
                    ],
                    stroke,
                );
            } else {
                painter.rect_stroke(candidate.rect, 2, stroke, egui::StrokeKind::Inside);
            }
        }
        ctx.set_cursor_icon(if valid {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
        if let Some(position) = ctx.pointer_interact_pos() {
            egui::Area::new(egui::Id::new("hierarchy_drag_preview"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(position + egui::vec2(14.0, 14.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        if let Some(selection) = payload
                            .selection
                            .as_ref()
                            .filter(|selection| selection.targets.len() > 1)
                        {
                            ui.label(format!("{} occurrences", selection.targets.len()));
                        } else {
                            ui.label(&payload.label);
                        }
                    });
                });
        }
        if ctx.input(|input| input.pointer.any_released()) {
            if valid {
                let candidate = candidate.unwrap();
                if let Some(project) = self.project.as_mut() {
                    if let Some(selection) = payload
                        .selection
                        .as_ref()
                        .filter(|selection| selection.targets.len() > 1)
                    {
                        let delta = self
                            .canvas
                            .hierarchy_child_origin(
                                project,
                                payload.root,
                                &payload.source_parent_path,
                            )
                            .and_then(|source| {
                                self.canvas
                                    .hierarchy_child_origin(
                                        project,
                                        candidate.root,
                                        &candidate.parent_path,
                                    )
                                    .map(|dest| source - dest)
                            });
                        let destination_path = edits::hierarchy_path_after_removals(
                            project,
                            candidate.root,
                            &candidate.parent_path,
                            &selection.targets,
                        );
                        let result = destination_path
                            .and_then(|path| delta.map(|delta| (path, delta)))
                            .and_then(|(path, delta)| {
                                edits::move_placements(
                                    project,
                                    &selection.targets,
                                    candidate.parent_id,
                                    candidate.insertion_index,
                                    (delta.x.round() as i32, delta.y.round() as i32),
                                )
                                .map(|targets| (path, targets))
                            });
                        match result {
                            Ok((path, targets)) => {
                                self.pending_hierarchy_selection =
                                    Some(PendingHierarchySelection::new(
                                        project,
                                        candidate.root,
                                        path,
                                        targets,
                                    ))
                            }
                            Err(error) => self.error_message = Some(error),
                        }
                        egui::DragAndDrop::clear_payload(ctx);
                        ctx.request_repaint();
                        return;
                    }
                    let parent_path =
                        ag_iso_terminal_designer::canvas::edits::hierarchy_path_after_removal(
                            project,
                            candidate.root,
                            &candidate.parent_path,
                            &payload.target,
                        );
                    match self.canvas.reparent_in_hierarchy_at(
                        project,
                        &payload.target,
                        payload.root,
                        &payload.source_parent_path,
                        candidate.root,
                        &candidate.parent_path,
                        candidate.parent_id,
                        candidate.insertion_index,
                    ) {
                        Ok(moved) => {
                            if let Ok(parent_path) = parent_path {
                                self.canvas.select_hierarchy_placement(
                                    project,
                                    candidate.root,
                                    &parent_path,
                                    moved,
                                );
                            }
                        }
                        Err(error) => self.error_message = Some(error),
                    }
                }
            }
            egui::DragAndDrop::clear_payload(ctx);
            ctx.request_repaint();
        }
    }

    fn render_workspace(&mut self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.remove::<TreeDropCandidate>(egui::Id::new("tree_drop_candidate")));
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
        self.finish_hierarchy_drag(ctx);
        if let Some(request) = ctx.data_mut(|data| {
            let value = data.get_temp::<GroupRequest>(egui::Id::new("hierarchy_group_request"));
            data.remove::<GroupRequest>(egui::Id::new("hierarchy_group_request"));
            value
        }) {
            if let Some(project) = &self.project {
                let name = project.generate_smart_name_for_new_object(request.object_type);
                self.new_object_dialog = Some((request.object_type, name));
                self.new_object_group = Some(request);
            }
        }
        if !self.canvas_visible {
            self.canvas.cancel_interaction();
        }
    }
    fn show_add_object_menu(&mut self, ui: &mut egui::Ui) {
        let opening = !self.add_object_menu_open && !self.add_object_menu_seen;
        if opening {
            self.add_object_query.clear();
        }
        self.add_object_menu_seen = true;
        let version = self.project.as_ref().unwrap().vt_version();
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.add_object_query)
                .hint_text("Search object types…")
                .desired_width(ui.available_width()),
        );
        if opening && !response.has_focus() {
            response.request_focus();
        }
        let query = self.add_object_query.to_lowercase();
        egui::ScrollArea::vertical()
            .max_height(400.0)
            .show(ui, |ui| {
                for object_type in ObjectType::values() {
                    if !format!("{object_type:?}").to_lowercase().contains(&query) {
                        continue;
                    }
                    let supported = ag_iso_terminal_designer::vt_compatibility::supported_object_type(
                        object_type, version,
                    );
                    let response = ui.add_enabled(
                        supported,
                        egui::Button::new(format!("{object_type:?}")),
                    );
                    if !supported && response.contains_pointer() {
                        if let Some(availability) = ag_iso_terminal_designer::vt_compatibility::availability_description(|v| {
                            ag_iso_terminal_designer::vt_compatibility::supported_object_type(object_type, v)
                        }) {
                            response.show_tooltip_text(availability);
                        }
                    }
                    if response.clicked() {
                        let default_name = self
                            .project
                            .as_ref()
                            .unwrap()
                            .generate_smart_name_for_new_object(object_type);
                        self.new_object_group = None;
                        self.new_object_dialog = Some((object_type, default_name));
                        ui.close();
                    }
                }
            });
    }

    fn show_view_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("View", |ui| {
            if ui
                .checkbox(&mut self.preferences.show_grid, "Show grid overlay")
                .changed()
            {
                self.canvas.set_grid_visible(self.preferences.show_grid);
            }
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
                ui.checkbox(&mut self.preferences.show_object_ids, "Show object IDs");
                ui.separator();
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
fn tree_row_with_id(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: bool,
    label: impl Into<String>,
) -> egui::Response {
    tree_row_with_highlight(ui, id, selected, true, label)
}

fn tree_row_with_highlight(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: bool,
    primary: bool,
    label: impl Into<String>,
) -> egui::Response {
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
    let (_, label_rect) = ui.allocate_space(egui::vec2(ui.available_width().max(0.0), height));

    let row_rect = egui::Rect::from_min_max(
        egui::pos2(bounds.left(), label_rect.top()),
        egui::pos2(bounds.right(), label_rect.bottom()),
    );
    // The expand arrow owns its hit area; row selection starts at the label.
    let response = ui.interact(label_rect, id, egui::Sense::click_and_drag());
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(egui::Id::new("tree_row_had_focus"), true));
    }
    let fill = if selected && !primary {
        ui.visuals().selection.bg_fill.gamma_multiply(0.5)
    } else if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter()
        .set(background, egui::Shape::rect_filled(row_rect, 0, fill));
    if selected && primary {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(row_rect.min, egui::vec2(3.0, row_rect.height())),
            0,
            ui.visuals().selection.stroke.color,
        );
    }
    let text_color = if selected && primary {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    ui.painter().with_clip_rect(label_rect).text(
        label_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label.into(),
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
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
