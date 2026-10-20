//! Application preferences and the project settings window.

use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectType};
use ag_iso_terminal_designer::{
    EditorProject, ObjectIdFormat, ObjectIdRanges, SoftKeyArrangement, SoftKeyLayout,
};
use eframe::egui;
use serde::{Deserialize, Serialize};

pub const STORAGE_KEY: &str = "designer_preferences_v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPreferences {
    pub theme: egui::ThemePreference,
    pub show_objects: bool,
    pub show_graphics: bool,
    pub show_components: bool,
    pub show_properties: bool,
    pub show_ancestry: bool,
    /// Show numeric IDs alongside object names in authoring labels.
    pub show_object_ids: bool,
    /// Show the automatically scaled canvas grid overlay.
    #[serde(default)]
    pub show_grid: bool,
    pub object_id_format: ObjectIdFormat,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            theme: egui::ThemePreference::System,
            show_objects: true,
            show_graphics: true,
            show_components: true,
            show_properties: true,
            show_ancestry: true,
            show_object_ids: false,
            show_grid: false,
            object_id_format: ObjectIdFormat::Decimal,
        }
    }
}

impl AppPreferences {
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(self.theme);
    }
}

#[derive(Clone, Debug)]
pub struct DimensionsDraft {
    pub mask: u16,
    pub soft_key_width: u16,
    pub soft_key_height: u16,
    pub vt_version: VtVersion,
    pub soft_key_layout: SoftKeyLayout,
    pub object_id_ranges: ObjectIdRanges,
}

impl DimensionsDraft {
    pub fn from_project(project: &EditorProject) -> Self {
        let (soft_key_width, soft_key_height) = project.soft_key_size();
        Self {
            mask: project.mask_size,
            soft_key_width,
            soft_key_height,
            vt_version: project.vt_version(),
            soft_key_layout: project.soft_key_layout(),
            object_id_ranges: project.object_id_ranges().clone(),
        }
    }
}

pub enum SettingsAction {
    OpenPane(crate::workspace::Pane),
    SharedAttributes,
}

pub fn show_project_settings(
    ctx: &egui::Context,
    open: &mut bool,
    mut project: Option<&mut EditorProject>,
    preferences: &mut AppPreferences,
    draft: &mut Option<DimensionsDraft>,
    error_message: &mut Option<String>,
) -> Option<SettingsAction> {
    if !*open {
        return None;
    }
    if draft.is_none() {
        *draft = project.as_deref().map(DimensionsDraft::from_project);
    }
    let mut apply = false;
    let mut cancel = false;
    let mut requested_pane = None;
    egui::Window::new("Project Settings")
        .open(open)
        .default_width(520.0)
        .resizable(true)
        .show(ctx, |ui| {
            let Some(project) = project.as_deref() else {
                ui.label("Open a project to configure its settings.");
                return;
            };
            let Some(values) = draft.as_mut() else {
                return;
            };
            egui::ScrollArea::vertical().show(ui, |ui| {
                ag_iso_terminal_designer::ui_style::section_heading(ui, "Dimensions");
                ui.horizontal(|ui| {
                    ui.label("Data/alarm mask");
                    ui.add(egui::DragValue::new(&mut values.mask).range(0..=u16::MAX));
                    ui.label(format!("{} × {} px", values.mask, values.mask));
                });
                ui.horizontal(|ui| {
                    ui.label("Softkey design size");
                    ui.add(egui::DragValue::new(&mut values.soft_key_width).range(0..=u16::MAX));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut values.soft_key_height).range(0..=u16::MAX));
                    ui.label("px");
                });
                ag_iso_terminal_designer::ui_style::section_heading(ui, "Softkey layout");
                ui.horizontal(|ui| {
                    ui.label("Arrangement");
                    ui.selectable_value(&mut values.soft_key_layout.arrangement, SoftKeyArrangement::Right, "Right columns");
                    ui.selectable_value(&mut values.soft_key_layout.arrangement, SoftKeyArrangement::Bottom, "Bottom rows");
                });
                ui.horizontal(|ui| {
                    ui.label("Physical keys");
                    ui.add(egui::DragValue::new(&mut values.soft_key_layout.physical_key_count).range(1..=64));
                    ui.label("Keys per column/row");
                    ui.add(egui::DragValue::new(&mut values.soft_key_layout.keys_per_line).range(1..=values.soft_key_layout.physical_key_count));
                    ui.label("Navigation keys");
                    ui.add(egui::DragValue::new(&mut values.soft_key_layout.navigation_key_count).range(0..=2.min(values.soft_key_layout.physical_key_count.saturating_sub(1))));
                });
                values.soft_key_layout.keys_per_line = values.soft_key_layout.keys_per_line.min(values.soft_key_layout.physical_key_count).max(1);
                values.soft_key_layout.navigation_key_count = values.soft_key_layout.navigation_key_count.min(2).min(values.soft_key_layout.physical_key_count.saturating_sub(1));
                if values.soft_key_layout.validate().is_err() {
                    ag_iso_terminal_designer::ui_style::error_text(ui, "Choose 1–64 physical keys, a valid row/column size, and fewer navigation keys than physical keys.");
                } else if u8::from(values.vt_version) >= u8::from(VtVersion::Version4) && values.soft_key_layout.physical_key_count < 6 {
                    ui.colored_label(ui.visuals().warn_fg_color, "Terminal minimum advisory: VT4+ terminals require at least 6 physical softkeys.");
                }
                if values.mask == 0 || values.soft_key_width == 0 || values.soft_key_height == 0 {
                    ag_iso_terminal_designer::ui_style::error_text(ui, "Dimensions must be positive.");
                }
                let (mask_minimum, key_minimum) = if values.vt_version == VtVersion::Version6 {
                    (480, (60, 60))
                } else {
                    (200, (60, 32))
                };
                if values.mask < mask_minimum {
                    ui.colored_label(ui.visuals().warn_fg_color, format!("Terminal minimum advisory: this VT version requires a terminal mask area of at least {mask_minimum} × {mask_minimum} px. Smaller design dimensions are allowed."));
                }
                if values.soft_key_width < key_minimum.0 || values.soft_key_height < key_minimum.1 {
                    ui.colored_label(ui.visuals().warn_fg_color, format!("Terminal minimum advisory: this VT version requires terminal softkey designators of at least {} × {} px. Smaller design dimensions are allowed.", key_minimum.0, key_minimum.1));
                }

                ui.separator();
                ag_iso_terminal_designer::ui_style::section_heading(ui, "Compatibility");
                ui.horizontal(|ui| {
                    ui.label("Target VT version");
                    egui::ComboBox::from_id_salt("project_vt_version")
                        .icon(ag_iso_terminal_designer::ui_style::combo_box_icon)
                        .selected_text(format!("VT{}", u8::from(values.vt_version)))
                        .show_ui(ui, |ui| {
                            for number in 2..=6 {
                                let version = VtVersion::try_from(number).unwrap();
                                ui.selectable_value(&mut values.vt_version, version, format!("VT{number}"));
                            }
                        });
                });
                ui.small("Apply checks the whole project. Incompatible version changes are blocked.");
                match project.compatibility_diagnostics() {
                    Ok(diagnostics) => {
                        for diagnostic in diagnostics {
                            ag_iso_terminal_designer::ui_style::error_text_with_color(
                                ui,
                                &project.format_validation_diagnostic(&diagnostic),
                                ui.visuals().warn_fg_color,
                            );
                        }
                    }
                    Err(error) => { ag_iso_terminal_designer::ui_style::error_text_with_color(ui, &error.to_string(), ui.visuals().warn_fg_color); }
                }
                disabled_section(
                    ui,
                    "Terminal capabilities",
                    "Terminal capability profiles are not available yet.",
                    &[
                        ("Terminal colour capability", DisabledControl::Choice(&["Not configured"])),
                        ("Supported fonts and styles", DisabledControl::Configure),
                        ("Input capabilities", DisabledControl::Configure),
                        ("Supported object types", DisabledControl::Configure),
                    ],
                );
                disabled_section(
                    ui,
                    "Languages",
                    "Project language and translation management are not available yet.",
                    &[
                        ("Supported languages", DisabledControl::Configure),
                        ("Default language", DisabledControl::Choice(&["Not configured"])),
                        ("Translation resources", DisabledControl::Configure),
                    ],
                );
                ag_iso_terminal_designer::ui_style::section_heading(ui, "Authoring");
                ui.horizontal(|ui| {
                    ui.label("Object ID format");
                    egui::ComboBox::from_id_salt("settings_object_id_format")
                        .icon(ag_iso_terminal_designer::ui_style::combo_box_icon)
                        .selected_text(match preferences.object_id_format {
                            ag_iso_terminal_designer::ObjectIdFormat::Decimal => "Decimal",
                            ag_iso_terminal_designer::ObjectIdFormat::Hexadecimal => "Hexadecimal (0x00AF)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut preferences.object_id_format, ag_iso_terminal_designer::ObjectIdFormat::Decimal, "Decimal");
                            ui.selectable_value(&mut preferences.object_id_format, ag_iso_terminal_designer::ObjectIdFormat::Hexadecimal, "Hexadecimal (0x00AF)");
                        });
                });
                ui.small("Application preference shared with View → Object IDs.");
                ui.checkbox(&mut preferences.show_object_ids, "Show object IDs");
                ui.small("Append each assigned ID to object names in trees, lists, and pickers.");
                ag_iso_terminal_designer::ui_style::section_heading(ui, "Automatic ID ranges");
                ui.horizontal(|ui| {
                    ui.label("Inclusive ranges by object type");
                    if ui.button("Reset to defaults").clicked() {
                        values.object_id_ranges = ObjectIdRanges::defaults();
                    }
                });
                egui::Grid::new("project_object_id_ranges")
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Object type");
                        ui.strong("First ID");
                        ui.strong("Last ID");
                        ui.end_row();
                        for object_type in ObjectType::values() {
                            let mut range = values.object_id_ranges.range_for(object_type);
                            ui.label(format!("{object_type:?}"));
                            ui.add(egui::DragValue::new(&mut range.start).range(0..=65534));
                            ui.add(egui::DragValue::new(&mut range.end).range(0..=65534));
                            values.object_id_ranges.set_range(object_type, range);
                            ui.end_row();
                        }
                    });
                match values.object_id_ranges.validate(values.vt_version) {
                    Ok(()) => {
                        ui.small("Ranges are inclusive, unique across types, and exclude reserved ID 65535. VT2–VT4 macro ranges must stay within 0–255.");
                    }
                    Err(error) => {
                        ag_iso_terminal_designer::ui_style::error_text(ui, &error);
                    }
                }
                ui.small("Changing ranges affects future automatic assignments. Existing IDs are preserved.");
                ui.add_enabled_ui(false, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Naming conventions");
                        ui.add_enabled(false, egui::Button::new("Configure…"));
                        ui.label("Not configured");
                    });
                });
                disabled_section(
                    ui,
                    "Export",
                    "Export grouping and code-generation options are not available yet.",
                    &[
                        ("Export groups", DisabledControl::Configure),
                        ("Language variants", DisabledControl::Configure),
                        ("Generated-code options", DisabledControl::Choice(&["Not configured"])),
                    ],
                );
                ag_iso_terminal_designer::ui_style::section_heading(ui, "Shared resources");
                ui.small("Manage reusable attributes, graphics, and components for this project.");
                ui.label("Edit shared resources in the existing editors:");
                ui.horizontal(|ui| {
                    if ui.button("Open Graphics editor").clicked() {
                        requested_pane = Some(SettingsAction::OpenPane(crate::workspace::Pane::Graphics));
                    }
                    if ui.button("Open Components editor").clicked() {
                        requested_pane = Some(SettingsAction::OpenPane(crate::workspace::Pane::Components));
                    }
                });
                if ui.button("Manage Shared Attributes…").clicked() {
                    requested_pane = Some(SettingsAction::SharedAttributes);
                }
                ui.small("Reuse Font, Line, and Fill Attributes across objects. Attribute edits apply immediately and are undoable. Opening the manager discards unapplied settings.");
                ui.add_enabled_ui(false, |ui| {
                    for label in ["Project colour styles", "Font presets", "Reusable graphic defaults"] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.add_enabled(false, egui::Button::new("Configure…"));
                            ui.label("Not configured");
                        });
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if apply {
        if let (Some(project), Some(values)) = (project.as_deref_mut(), draft.take()) {
            if values.mask == 0 || values.soft_key_width == 0 || values.soft_key_height == 0 {
                *error_message = Some("Project dimensions must be positive.".into());
                *draft = Some(values);
            } else if let Err(error) = project.set_project_configuration_with_id_ranges(
                values.mask,
                (values.soft_key_width, values.soft_key_height),
                values.vt_version,
                values.soft_key_layout,
                values.object_id_ranges.clone(),
            ) {
                *error_message = Some(error.to_string());
                *draft = Some(values);
            } else {
                *open = false;
            }
        }
    } else if cancel || requested_pane.is_some() || !*open {
        if cancel {
            *open = false;
        }
        if requested_pane.is_some() {
            *open = false;
        }
        *draft = None;
    }
    requested_pane
}

#[derive(Clone, Copy)]
enum DisabledControl {
    Choice(&'static [&'static str]),
    Configure,
}

fn disabled_section(
    ui: &mut egui::Ui,
    title: &str,
    explanation: &str,
    controls: &[(&str, DisabledControl)],
) {
    ag_iso_terminal_designer::ui_style::section_heading(ui, title);
    ui.add_enabled_ui(false, |ui| {
        for (index, (label, control)) in controls.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(*label);
                match control {
                    DisabledControl::Choice(options) => {
                        let mut selected = 0;
                        egui::ComboBox::from_id_salt((title, index))
                            .icon(ag_iso_terminal_designer::ui_style::combo_box_icon)
                            .selected_text(options[selected])
                            .show_ui(ui, |ui| {
                                for (option_index, option) in options.iter().enumerate() {
                                    ui.selectable_value(&mut selected, option_index, *option);
                                }
                            });
                    }
                    DisabledControl::Configure => {
                        ui.add_enabled(false, egui::Button::new("Configure…"));
                        ui.label("Not configured");
                    }
                }
            });
        }
        ui.small(explanation);
    });
}
