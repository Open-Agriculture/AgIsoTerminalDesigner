//! Application preferences and the project settings window.

use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_terminal_designer::{EditorProject, ObjectIdFormat};
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
    #[cfg(not(target_arch = "wasm32"))]
    pub show_proposal: bool,
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
            #[cfg(not(target_arch = "wasm32"))]
            show_proposal: true,
            object_id_format: ObjectIdFormat::Decimal,
        }
    }
}

impl AppPreferences {
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(self.theme);
    }

    pub fn show_view_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("View", |ui| {
            ui.menu_button("Theme", |ui| {
                for (theme, label) in [
                    (egui::ThemePreference::System, "System"),
                    (egui::ThemePreference::Light, "Light"),
                    (egui::ThemePreference::Dark, "Dark"),
                ] {
                    if ui.selectable_value(&mut self.theme, theme, label).clicked() {
                        ctx.set_theme(self.theme);
                    }
                }
            });
            ui.menu_button("Panels", |ui| {
                ui.checkbox(&mut self.show_objects, "Objects");
                ui.checkbox(&mut self.show_graphics, "Graphics");
                ui.checkbox(&mut self.show_components, "Components");
                ui.checkbox(&mut self.show_properties, "Properties");
                ui.checkbox(&mut self.show_ancestry, "Selection Ancestry");
                #[cfg(not(target_arch = "wasm32"))]
                ui.checkbox(&mut self.show_proposal, "Proposal");
                ui.separator();
                if ui.button("Reset Panel Layout").clicked() {
                    self.show_objects = true;
                    self.show_graphics = true;
                    self.show_components = true;
                    self.show_properties = true;
                    self.show_ancestry = true;
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        self.show_proposal = true;
                    }
                    ctx.memory_mut(|memory| memory.reset_areas());
                    ctx.data_mut(|data| {
                        for id in [
                            "left_panel",
                            "components_panel",
                            "right_panel",
                            "selected_ancestry_panel",
                        ] {
                            data.remove::<egui::containers::panel::PanelState>(egui::Id::new(id));
                        }
                    });
                    ui.close();
                }
            });
            ui.menu_button("Object IDs", |ui| {
                for (format, label) in [
                    (ObjectIdFormat::Decimal, "Decimal"),
                    (ObjectIdFormat::Hexadecimal, "Hexadecimal (0x00AF)"),
                ] {
                    ui.selectable_value(&mut self.object_id_format, format, label);
                }
            });
        });
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DimensionsDraft {
    pub mask: u16,
    pub soft_key_width: u16,
    pub soft_key_height: u16,
}

impl DimensionsDraft {
    pub fn from_project(project: &EditorProject) -> Self {
        let (soft_key_width, soft_key_height) = project.soft_key_size();
        Self {
            mask: project.mask_size,
            soft_key_width,
            soft_key_height,
        }
    }
}

pub fn show_project_settings(
    ctx: &egui::Context,
    open: &mut bool,
    mut project: Option<&mut EditorProject>,
    preferences: &mut AppPreferences,
    draft: &mut Option<DimensionsDraft>,
    error_message: &mut Option<String>,
) {
    if !*open {
        return;
    }
    if draft.is_none() {
        *draft = project.as_deref().map(DimensionsDraft::from_project);
    }
    let mut apply = false;
    let mut cancel = false;
    let mut open_graphics = false;
    let mut open_components = false;
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
                ui.heading("Dimensions");
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
                ui.small("Softkey dimensions are saved design values; softkey preview is not available yet.");
                if values.mask == 0 || values.soft_key_width == 0 || values.soft_key_height == 0 {
                    ui.colored_label(egui::Color32::RED, "Dimensions must be positive.");
                }
                let (mask_minimum, key_minimum) = if project.vt_version == VtVersion::Version6 {
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
                ui.heading("Compatibility");
                ui.label(format!("Target VT version: VT{} (read-only)", u8::from(project.vt_version)));
                disabled_section(
                    ui,
                    "Terminal capabilities",
                    "Terminal capability profiles are not available yet.",
                    &[
                        ("VT version selection", DisabledControl::Choice(&["Not configured"])),
                        ("Terminal colour capability", DisabledControl::Choice(&["Not configured"])),
                        ("Supported fonts and styles", DisabledControl::Configure),
                        ("Input capabilities", DisabledControl::Configure),
                        ("Supported object types", DisabledControl::Configure),
                    ],
                );
                disabled_section(
                    ui,
                    "Softkey layout",
                    "Physical terminal key layouts are not available yet.",
                    &[
                        ("Physical softkey count", DisabledControl::Number),
                        ("Navigation-key count", DisabledControl::Number),
                        ("Key arrangement", DisabledControl::Choice(&["Not configured"])),
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
                ui.heading("Authoring");
                ui.horizontal(|ui| {
                    ui.label("Object ID format");
                    egui::ComboBox::from_id_salt("settings_object_id_format")
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
                ui.add_enabled_ui(false, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("ID allocation ranges");
                        ui.add_enabled(false, egui::Button::new("Configure…"));
                        ui.label("Not configured");
                    });
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
                ui.heading("Shared resources");
                ui.small("Project-specific resource defaults are not available yet.");
                ui.label("Edit shared resources in the existing editors:");
                ui.horizontal(|ui| {
                    if ui.button("Open Graphics editor").clicked() {
                        open_graphics = true;
                    }
                    if ui.button("Open Components editor").clicked() {
                        open_components = true;
                    }
                });
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
            } else if let Err(error) = project
                .set_design_dimensions(values.mask, (values.soft_key_width, values.soft_key_height))
            {
                *error_message = Some(error.to_string());
                *draft = Some(values);
            } else {
                *open = false;
            }
        }
    } else if cancel || open_graphics || open_components || !*open {
        if cancel {
            *open = false;
        }
        if open_graphics {
            preferences.show_graphics = true;
            *open = false;
        }
        if open_components {
            preferences.show_components = true;
            *open = false;
        }
        *draft = None;
    }
}

#[derive(Clone, Copy)]
enum DisabledControl {
    Number,
    Choice(&'static [&'static str]),
    Configure,
}

fn disabled_section(
    ui: &mut egui::Ui,
    title: &str,
    explanation: &str,
    controls: &[(&str, DisabledControl)],
) {
    ui.heading(title);
    ui.add_enabled_ui(false, |ui| {
        for (index, (label, control)) in controls.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(*label);
                match control {
                    DisabledControl::Number => {
                        let mut value = 0_u16;
                        ui.add(egui::DragValue::new(&mut value).range(0..=u16::MAX));
                        ui.label("Not configured");
                    }
                    DisabledControl::Choice(options) => {
                        let mut selected = 0;
                        egui::ComboBox::from_id_salt((title, index))
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
