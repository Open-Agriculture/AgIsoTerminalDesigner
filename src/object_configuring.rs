//! Descriptor-driven object configuration.
//!
//! This module is deliberately generic: object types expose properties through
//! `PropertyAccess`, and this renderer is the sole configuration UI.

use crate::object_properties::{
    get_object_property, get_property_descriptors, property_editor_descriptors, EditorKind,
    EditorTarget, PropertyKey, PropertySemantic,
};
use crate::operations::{
    ChildPlacement, MacroReferenceTarget, ObjectReferenceList, ObjectTarget, ObjectTargetList,
    OperationTransaction,
};
use crate::{EditorProject, Operation};
use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef};
use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::{NullableObjectId, ObjectId, ObjectRef, ObjectType};
use eframe::egui;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
enum PendingValue {
    Number(f64),
    Text(String),
}

#[derive(Clone)]
struct PendingPosition {
    x: i16,
    y: i16,
    revision: u64,
}

#[derive(Clone)]
struct PendingOperation {
    session: uuid::Uuid,
    revision: u64,
    operation: Option<Operation>,
}

#[derive(Clone, Default)]
struct PropertyDraftRegistry {
    entries: HashMap<egui::Id, PendingOperation>,
    seen: HashSet<egui::Id>,
}

fn registry_id() -> egui::Id {
    egui::Id::new("property_editor_pending_drafts")
}

fn touch_draft(ctx: &egui::Context, key: egui::Id) {
    ctx.data_mut(|data| {
        let registry_id = registry_id();
        let mut registry = data
            .get_temp::<PropertyDraftRegistry>(registry_id)
            .unwrap_or_default();
        if registry.entries.contains_key(&key) {
            registry.seen.insert(key);
            data.insert_temp(registry_id, registry);
        }
    });
}

fn update_draft(
    ctx: &egui::Context,
    key: egui::Id,
    design: &EditorProject,
    operation: Option<Operation>,
) {
    ctx.data_mut(|data| {
        let registry_id = registry_id();
        let mut registry = data
            .get_temp::<PropertyDraftRegistry>(registry_id)
            .unwrap_or_default();
        registry.entries.insert(
            key,
            PendingOperation {
                session: design.ui_session_id(),
                revision: design.revision(),
                operation,
            },
        );
        registry.seen.insert(key);
        data.insert_temp(registry_id, registry);
    });
}

fn clear_draft(ctx: &egui::Context, key: egui::Id) {
    ctx.data_mut(|data| {
        let registry_id = registry_id();
        if let Some(mut registry) = data.get_temp::<PropertyDraftRegistry>(registry_id) {
            registry.entries.remove(&key);
            registry.seen.remove(&key);
            data.insert_temp(registry_id, registry);
        }
    });
}

/// Finish property drafts whose widgets disappeared since the previous frame.
/// Call after rendering the editor panels and before committing queued edits.
pub fn finish_property_edits(ctx: &egui::Context, design: &EditorProject) {
    let operations = ctx.data_mut(|data| {
        let registry_id = registry_id();
        let mut registry = data
            .get_temp::<PropertyDraftRegistry>(registry_id)
            .unwrap_or_default();
        let mut operations = Vec::new();
        let mut expired_keys = Vec::new();
        let seen = registry.seen.clone();
        registry.entries.retain(|key, pending| {
            if seen.contains(key) {
                if pending.session == design.ui_session_id()
                    && pending.revision == design.revision()
                {
                    return true;
                }
                expired_keys.push(*key);
                return false;
            }
            if pending.session == design.ui_session_id() && pending.revision == design.revision() {
                if let Some(operation) = pending.operation.take() {
                    operations.push(operation);
                }
            }
            expired_keys.push(*key);
            false
        });
        registry.seen.clear();
        data.insert_temp(registry_id, registry);
        for key in expired_keys {
            data.remove::<PendingValue>(key);
            data.remove::<PendingPosition>(key);
            data.remove::<u16>(key);
        }
        operations
    });
    for operation in operations {
        design.queue_operation(operation);
    }
}

/// A drag value changes on every pointer-move frame. Keep displaying those
/// intermediate values, but only turn the edit into an operation when the
/// pointer is released (or a keyboard edit loses focus).
fn drag_value_finished(response: &egui::Response) -> bool {
    response.drag_stopped() || response.lost_focus()
}

/// Render every writable property declared for `object`.
pub fn render_property_editor(ui: &mut egui::Ui, object: &Object, design: &EditorProject) {
    ui.push_id((design.ui_session_id(), design.revision()), |ui| {
        render_property_editor_scoped(ui, object, design);
    });
}

fn render_property_editor_scoped(ui: &mut egui::Ui, object: &Object, design: &EditorProject) {
    ui.style_mut().spacing.combo_width = 0.0;
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
    ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
    ui.spacing_mut().interact_size.y = 26.0;
    let id = object.id();
    let object_type = object.object_type();
    let properties = get_property_descriptors(object_type);

    render_identity(ui, id, object_type, design);
    let mut unavailable = unavailable_features(object, design.vt_version);
    if object_type == ObjectType::ObjectPointer {
        for target in design.get_object_info(object).allowed_references {
            if let Some(target) = design.get_pool().object_by_id(target) {
                if !crate::vt_compatibility::supported_object_type(
                    target.object_type(),
                    design.vt_version,
                ) {
                    if let Some(availability) =
                        crate::vt_compatibility::availability_description(|version| {
                            crate::vt_compatibility::supported_object_type(
                                target.object_type(),
                                version,
                            )
                        })
                    {
                        unavailable.push(format!(
                            "Allowed references: {:?} — {availability}",
                            target.object_type()
                        ));
                    }
                }
            }
        }
    }
    crate::ui_style::vt_availability_note(
        ui,
        ("object_vt_availability", id.value()),
        design.vt_version,
        unavailable,
    );
    ui.separator();
    if let Object::PictureGraphic(picture) = object {
        let current_format = picture.format;
        ui.horizontal(|ui| {
            ui.label("Format:");
            use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat;
            for (format, label) in [
                (PictureGraphicFormat::Monochrome, "Monochrome"),
                (PictureGraphicFormat::FourBit, "4-bit colour"),
                (PictureGraphicFormat::EightBit, "8-bit colour"),
            ] {
                if ui.radio(current_format == format, label).clicked() && current_format != format {
                    let mut converted = picture.clone();
                    crate::graphics::convert_picture_graphic_format(&mut converted, format);
                    let value = serde_json::to_value(converted).unwrap();
                    for property in [PropertyKey::Format, PropertyKey::Options, PropertyKey::Data] {
                        queue(design, id, property, value[property.as_str()].clone());
                    }
                }
            }
        });
        let mut transparent = picture.options.transparent;
        let mut colour = picture.transparency_colour;
        let mut flashing = picture.options.flashing;
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui
                .checkbox(&mut transparent, "Transparent Pixels")
                .changed();
            if transparent {
                changed |= ui
                    .add(
                        egui::Slider::new(&mut colour, 0..=255)
                            .text("Transparent Colour")
                            .drag_value_speed(1.0),
                    )
                    .changed();
            }
        });
        changed |= ui.checkbox(&mut flashing, "Flashing").changed();
        if changed {
            let mut options = picture.options.clone();
            options.transparent = transparent;
            options.flashing = flashing;
            queue(
                design,
                id,
                PropertyKey::Options,
                serde_json::to_value(options).unwrap(),
            );
            queue(
                design,
                id,
                PropertyKey::TransparencyColour,
                serde_json::json!(colour),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Load Image")
                .on_hover_text("Load an image file (PNG, JPG, BMP, etc.)")
                .clicked()
            {
                design.request_image_load(id);
            }
            if let Some(asset) = crate::graphics::ui::available_button(
                ui,
                ("choose_available", id.value()),
                design,
                id,
            ) {
                design.request_graphic_assignment(asset, id);
            }
        });
    }
    for editor in property_editor_descriptors(object_type) {
        let EditorTarget::Property(property_key) = editor.property else {
            continue;
        };
        let property_name = property_key.as_str();
        if object_type == ObjectType::PictureGraphic
            && matches!(
                property_key,
                PropertyKey::Format | PropertyKey::Options | PropertyKey::TransparencyColour
            )
        {
            continue;
        }

        if object_type == ObjectType::PictureGraphic
            && matches!(
                property_key,
                PropertyKey::Data | PropertyKey::ActualWidth | PropertyKey::ActualHeight
            )
        {
            continue;
        }
        let Some(property) = properties.iter().find(|item| item.name == property_key) else {
            continue;
        };
        let Ok(value) = get_object_property(object, property_key) else {
            continue;
        };
        let label = human_readable_property_name(property_name);
        if object_type == ObjectType::ExternalObjectPointer
            && property_key == PropertyKey::ExternalObjectId
        {
            let key = ui.make_persistent_id((id.value(), property_name));
            touch_draft(ui.ctx(), key);
            let original_foreign_id = value.as_u64().map_or(u16::MAX, |value| value as u16);
            let mut foreign_id = ui
                .data(|data| data.get_temp::<u16>(key))
                .unwrap_or(original_foreign_id);
            ui.horizontal(|ui| {
                ui.label(&label);
                let format = design.object_id_format();
                let response = ui.add(
                    egui::DragValue::new(&mut foreign_id)
                        .range(0..=u16::MAX)
                        .custom_formatter(move |value, _| format.format(value as u16))
                        .custom_parser(move |text| format.parse(text).ok().map(f64::from)),
                );
                if response.changed() {
                    ui.data_mut(|data| data.insert_temp(key, foreign_id));
                    let operation = if foreign_id == u16::MAX {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!(foreign_id)
                    };
                    update_draft(
                        ui.ctx(),
                        key,
                        design,
                        Some(property_operation(design, id, property_key, operation)),
                    );
                }
                let cancelled =
                    response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape));
                if cancelled {
                    ui.data_mut(|data| data.remove::<u16>(key));
                    clear_draft(ui.ctx(), key);
                } else if drag_value_finished(&response)
                    || response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
                {
                    let value = if foreign_id == u16::MAX {
                        serde_json::Value::Null
                    } else {
                        foreign_id.into()
                    };
                    if foreign_id != original_foreign_id {
                        queue(design, id, property_key, value);
                    }
                    ui.data_mut(|data| data.remove::<u16>(key));
                    clear_draft(ui.ctx(), key);
                }
            });
            ui.small("Object ID in the external ECU's pool; 65535 means no reference.");
            continue;
        }
        if !crate::vt_compatibility::supported_property(
            object_type,
            property_name,
            design.vt_version,
        ) {
            let default = crate::default_object(object_type);
            if get_object_property(&default, property_key).ok().as_ref() == Some(&value) {
                continue;
            }
            repair_warning(ui, &label, design.vt_version, |version| {
                crate::vt_compatibility::supported_property(object_type, property_name, version)
            });
        }
        if property_key == PropertyKey::Value {
            if let Some(decoded) = decoded_string_value(object, design.get_pool()) {
                match decoded {
                    Ok(text) => {
                        ui.push_id(property_name, |ui| {
                            render_text(ui, &label, &text, id, property_key, design)
                        });
                    }
                    Err(message) => {
                        crate::ui_style::error_text(ui, &message);
                    }
                }
                continue;
            }
        }
        if object_type == ObjectType::InputBoolean && property_key == PropertyKey::Value {
            if value.as_u64().is_some_and(|value| value > 1)
                && design.vt_version < VtVersion::Version4
            {
                repair_warning(ui, &label, design.vt_version, |version| {
                    crate::vt_compatibility::supported_input_boolean_value(2, version)
                });
            }
            let maximum = if design.vt_version < VtVersion::Version4 {
                1.0
            } else {
                u8::MAX as f64
            };
            render_number(ui, &label, &value, 0.0..=maximum, id, property_key, design);
            continue;
        }
        if matches!(editor.editor, EditorKind::ObjectReference) {
            if let Some(target) = value
                .as_u64()
                .and_then(|value| ObjectId::new(value as u16).ok())
                .and_then(|target| design.get_pool().object_by_id(target))
            {
                if !crate::vt_compatibility::supported_object_type(
                    target.object_type(),
                    design.vt_version,
                ) {
                    repair_warning(ui, &label, design.vt_version, |version| {
                        crate::vt_compatibility::supported_object_type(
                            target.object_type(),
                            version,
                        )
                    });
                }
            }
        }
        ui.push_id(property_name, |ui| match editor.editor {
            EditorKind::Colour => render_colour(ui, &label, &value, id, property_key, design),
            EditorKind::ObjectReference => {
                if object_type == ObjectType::ObjectPointer && property_key == PropertyKey::Value {
                    render_pointer_reference(
                        ui,
                        &label,
                        &value,
                        property,
                        id,
                        property_key,
                        design,
                    );
                } else {
                    render_reference(ui, &label, &value, property, id, property_key, design)
                }
            }
            EditorKind::Enum => {
                if !crate::vt_compatibility::supported_enum_value(
                    object_type,
                    property_key,
                    &value,
                    design.vt_version,
                ) {
                    repair_warning(ui, &label, design.vt_version, |version| {
                        crate::vt_compatibility::supported_enum_value(
                            object_type,
                            property_key,
                            &value,
                            version,
                        )
                    });
                }
                ui.horizontal(|ui| {
                    let label_response = ui.label(&label);
                    egui::ComboBox::from_id_salt(property_name)
                        .icon(crate::ui_style::combo_box_icon)
                        .selected_text(enum_choice_label(&value))
                        .show_ui(ui, |ui| {
                            for choice in &property.enum_choices {
                                let supported = crate::vt_compatibility::supported_enum_value(
                                    object_type,
                                    property_key,
                                    choice,
                                    design.vt_version,
                                );
                                let valid = supported
                                    && (choice == &value
                                        || property_choice_is_valid(
                                            design,
                                            id,
                                            property_key,
                                            choice.clone(),
                                        ));
                                let response = ui.add_enabled(
                                    valid,
                                    egui::Button::selectable(
                                        choice == &value,
                                        enum_choice_label(choice),
                                    ),
                                );
                                if !supported && response.contains_pointer() {
                                    if let Some(availability) =
                                        crate::vt_compatibility::availability_description(
                                            |version| {
                                                crate::vt_compatibility::supported_enum_value(
                                                    object_type,
                                                    property_key,
                                                    choice,
                                                    version,
                                                )
                                            },
                                        )
                                    {
                                        response.show_tooltip_text(availability);
                                    }
                                }
                                if response.clicked() && choice != &value {
                                    queue(design, id, property_key, choice.clone());
                                }
                            }
                        })
                        .response
                        .labelled_by(label_response.id);
                });
            }
            EditorKind::FlagSet => {
                render_flag_set(ui, &label, &value, id, object_type, property_key, design)
            }
            EditorKind::Justification => {
                render_justification(ui, &label, &value, id, property_key, design)
            }
            EditorKind::Auto => render_auto(ui, &label, &value, id, property_key, design),
            EditorKind::MacroReferences => unreachable!("macro references are rendered separately"),
        });
    }

    if let Object::PictureGraphic(picture) = object {
        render_picture_dimensions(ui, picture, id, design);
    }

    if let Some(children) = crate::operations::operation::object_refs(object) {
        ui.separator();
        ui.label("Objects");
        render_positioned_children(ui, object, children, design);
    }
    if let Some(objects) = crate::operations::operation::object_list(object) {
        ui.separator();
        ui.label("Objects");
        render_object_list(ui, object, objects, design);
    }
    if crate::operations::operation::macro_refs(object).is_some() {
        ui.separator();
        render_macro_references(ui, object, id, design);
    }
}

/// Encoded values retain their bytes until the operator actually edits text.
/// A shared variable has no font of its own; use the first referencing string
/// field in pool order to choose its editor character set.
fn decoded_string_value(
    object: &Object,
    pool: &ag_iso_stack::object_pool::ObjectPool,
) -> Option<Result<String, String>> {
    use ag_iso_stack::object_pool::object_attributes::FontType;
    let (value, font_id) = match object {
        Object::OutputString(o) => (&o.value, Some(o.font_attributes)),
        Object::InputString(o) => (&o.value, Some(o.font_attributes)),
        Object::StringVariable(o) => {
            let font = pool.objects().iter().find_map(|candidate| match candidate {
                Object::OutputString(s) if s.variable_reference.0 == Some(o.id) => {
                    Some(s.font_attributes)
                }
                Object::InputString(s) if s.variable_reference.0 == Some(o.id) => {
                    Some(s.font_attributes)
                }
                _ => None,
            });
            (&o.value, font)
        }
        _ => return None,
    };
    let font_type = font_id
        .and_then(|id| match pool.object_by_nullable_id(id) {
            Some(Object::FontAttributes(font)) => Some(font.font_type),
            _ => None,
        })
        .unwrap_or(FontType::Latin1);
    Some(
        value
            .decode(font_type)
            .map_err(|_| "Unsupported string encoding".to_owned()),
    )
}

/// Keep payload enums readable without exposing their JSON representation.
fn enum_choice_label(value: &serde_json::Value) -> String {
    if let Some(name) = value.as_str() {
        return human_readable_property_name(name);
    }
    if let Some(fields) = value.as_object() {
        if let Some(size) = fields
            .get("NonProportional")
            .and_then(serde_json::Value::as_str)
        {
            return format!("{} px", size.trim_start_matches("Px").replace('x', " × "));
        }
        if let Some(height) = fields.get("Proportional") {
            return format!("Proportional: {height} px");
        }
        if let Some((variant, payload)) = fields.iter().next() {
            return format!("{}: {payload}", human_readable_property_name(variant));
        }
    }
    value.to_string()
}

/// Commit a palette selection through the same undoable operation as other properties.
fn render_colour(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    use crate::isobus_colour::{self, FIRST_PROPRIETARY};
    let Some(current) = value.as_u64().and_then(|v| u8::try_from(v).ok()) else {
        return;
    };
    ui.label(label);
    ui.horizontal(|ui| {
        if let Some([r, g, b]) = isobus_colour::standard_rgb(current) {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
        }
        ui.menu_button(isobus_colour::label(current), |ui| {
            ui.label("ISOBUS standard palette (8-bit index)");
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| {
                    for (title, start, end) in [
                        ("Basic colours", 0, 15),
                        ("Colour cube", 16, FIRST_PROPRIETARY - 1),
                        (
                            "Manufacturer-specific (appearance undefined)",
                            FIRST_PROPRIETARY,
                            255,
                        ),
                    ] {
                        ui.label(title);
                        egui::Grid::new(title)
                            .spacing(egui::vec2(3.0, 3.0))
                            .show(ui, |ui| {
                                for index in start..=end {
                                    let mut button = egui::Button::new(index.to_string())
                                        .min_size(egui::vec2(32.0, 24.0));
                                    if let Some([r, g, b]) = isobus_colour::standard_rgb(index) {
                                        let text_colour =
                                            if 299 * r as u32 + 587 * g as u32 + 114 * b as u32
                                                > 128_000
                                            {
                                                egui::Color32::BLACK
                                            } else {
                                                egui::Color32::WHITE
                                            };
                                        button = egui::Button::new(
                                            egui::RichText::new(index.to_string())
                                                .color(text_colour),
                                        )
                                        .fill(egui::Color32::from_rgb(r, g, b))
                                        .min_size(egui::vec2(32.0, 24.0));
                                    }
                                    if index == current {
                                        button = button.stroke(egui::Stroke::new(
                                            2.0,
                                            ui.visuals().selection.stroke.color,
                                        ));
                                    }
                                    if ui
                                        .add(button)
                                        .on_hover_text(isobus_colour::label(index))
                                        .clicked()
                                    {
                                        if index != current {
                                            queue(design, id, property, serde_json::json!(index));
                                        }
                                        ui.close();
                                    }
                                    if (index as usize - start as usize + 1) % 8 == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    }
                });
        });
    });
}

/// Convert a snake_case property identifier to a title-cased editor label.
fn human_readable_property_name(property: &str) -> String {
    let mut label = String::with_capacity(property.len());
    let mut capitalise_next = true;
    for character in property.chars() {
        if character == '_' {
            label.push(' ');
            capitalise_next = true;
        } else if capitalise_next {
            label.extend(character.to_uppercase());
            capitalise_next = false;
        } else {
            label.push(character);
        }
    }
    label
}

#[derive(Clone)]
enum ListAction {
    MoveUp(usize),
    MoveDown(usize),
    Remove(usize),
    ReplaceMacro(usize, MacroRef),
}

fn row_actions(ui: &mut egui::Ui, index: usize, len: usize, action: &mut Option<ListAction>) {
    if ui
        .add_enabled(index > 0, egui::Button::new("\u{23F6}"))
        .on_hover_text("Move up")
        .clicked()
    {
        *action = Some(ListAction::MoveUp(index));
    }
    if ui
        .add_enabled(index + 1 < len, egui::Button::new("\u{23F7}"))
        .on_hover_text("Move down")
        .clicked()
    {
        *action = Some(ListAction::MoveDown(index));
    }
    if ui.button("\u{1F5D9}").on_hover_text("Remove").clicked() {
        *action = Some(ListAction::Remove(index));
    }
}

fn object_label(design: &EditorProject, object: &Object) -> String {
    design.object_display_name(object)
}

fn object_selector_label(design: &EditorProject, object: &Object) -> String {
    design.object_display_name(object)
}

fn select_object(design: &EditorProject, id: ObjectId) {
    design.get_mut_selected().set(NullableObjectId(Some(id)));
}

fn candidate_is_valid(design: &EditorProject, parent: ObjectId, child: ObjectId) -> bool {
    crate::pool_validation::would_create_circular_reference(design.get_pool(), parent, child)
        .is_ok()
        && crate::pool_validation::validate_parent_child_relationship(
            design.get_pool(),
            parent,
            child,
        )
        .is_ok()
}

fn render_object_selector(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    design: &EditorProject,
    parent: ObjectId,
    selected: ObjectId,
    index: usize,
    allowed_types: &[ObjectType],
) -> Option<ObjectId> {
    let selected_text = design.get_pool().object_by_id(selected).map_or_else(
        || format!("{}: Missing object", selected.value()),
        |object| object_selector_label(design, object),
    );
    child_picker(
        ui,
        salt,
        &selected_text,
        design,
        parent,
        Some(selected),
        Some(index),
        allowed_types,
        false,
    )
    .and_then(|id| id.0)
    .filter(|id| *id != selected)
}

fn child_picker(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    label: &str,
    design: &EditorProject,
    parent: ObjectId,
    selected: Option<ObjectId>,
    index: Option<usize>,
    allowed: &[ObjectType],
    allow_none: bool,
) -> Option<NullableObjectId> {
    let action = format!(
        "{} {}",
        if index.is_some() {
            "Replace child of"
        } else {
            "Add child to"
        },
        crate::object_picker::object_label(design, Some(parent))
    );
    crate::object_picker::button(
        ui,
        salt,
        label,
        &action,
        design,
        selected,
        allow_none,
        |candidate| {
            allowed.contains(&candidate.object_type())
                && crate::vt_compatibility::supported_object_type(
                    candidate.object_type(),
                    design.vt_version,
                )
                && candidate_is_valid(design, parent, candidate.id())
        },
        |ui, choice| {
            crate::object_picker::impact::review_queued(ui, design, |candidate| {
                queue_child_selection(candidate, parent, index, choice);
            })
        },
    )
}

fn queue_child_selection(
    design: &EditorProject,
    parent: ObjectId,
    index: Option<usize>,
    choice: NullableObjectId,
) {
    let Some(object) = design.get_pool().object_by_id(parent) else {
        return;
    };
    if let Some(children) = crate::operations::operation::object_refs(object) {
        let Some(child) = choice.0 else {
            return;
        };
        if let Some(index) = index {
            let mut updated = children.to_vec();
            if let Some(entry) = updated.get_mut(index) {
                entry.id = child;
            } else {
                return;
            }
            design.queue_operation(Operation::SetChildren {
                parent: ObjectTarget::id(parent.value()),
                children: updated
                    .into_iter()
                    .map(|child| ChildPlacement {
                        child: ObjectTarget::id(child.id.value()),
                        x: child.offset.x,
                        y: child.offset.y,
                    })
                    .collect(),
            });
        } else {
            design.queue_operation(Operation::AddChild {
                parent: ObjectTarget::id(parent.value()),
                child: ObjectTarget::id(child.value()),
                x: 0,
                y: 0,
            });
        }
    } else if let Some(mut objects) = crate::operations::operation::object_list(object) {
        match &mut objects {
            ObjectReferenceList::Required(ids) => {
                let Some(child) = choice.0 else {
                    return;
                };
                if let Some(index) = index {
                    ids[index] = child;
                } else {
                    ids.push(child);
                }
            }
            ObjectReferenceList::Nullable(ids) => {
                if let Some(index) = index {
                    ids[index] = choice;
                } else {
                    ids.push(choice);
                }
            }
        }
        queue_object_list(design, parent, objects);
    }
}

fn render_positioned_children(
    ui: &mut egui::Ui,
    parent: &Object,
    children: &[ObjectRef],
    design: &EditorProject,
) {
    let parent_id = parent.id();
    let allowed = crate::allowed_object_relationships::get_allowed_child_refs(
        parent.object_type(),
        design.vt_version,
    );
    let mut replacement = None;
    let mut position = None;
    let mut action = None;
    egui::Grid::new(("positioned_children", parent_id.value()))
        .striped(true)
        .min_col_width(0.0)
        .show(ui, |ui| {
            ui.label("Object");
            ui.label("Type");
            ui.label("X");
            ui.label("Y");
            ui.label("");
            ui.label("");
            ui.label("");
            ui.end_row();
            for (index, child_ref) in children.iter().enumerate() {
                if let Some(new_id) = render_object_selector(
                    ui,
                    ("positioned_child", index),
                    design,
                    parent_id,
                    child_ref.id,
                    index,
                    &allowed,
                ) {
                    replacement = Some((index, new_id));
                }
                let child = design.get_pool().object_by_id(child_ref.id);
                if let Some(child) = child {
                    if ui.link(format!("{:?}", child.object_type())).clicked() {
                        select_object(design, child.id());
                    }
                } else {
                    crate::ui_style::error_text(ui, "Missing object");
                }
                let position_key =
                    ui.make_persistent_id(("positioned_child_position", parent_id.value(), index));
                touch_draft(ui.ctx(), position_key);
                let revision = design.revision();
                let mut edited = ui
                    .data(|data| data.get_temp::<PendingPosition>(position_key))
                    .filter(|pending| pending.revision == revision)
                    .unwrap_or(PendingPosition {
                        x: child_ref.offset.x,
                        y: child_ref.offset.y,
                        revision,
                    });
                let x_response = ui.add(
                    egui::DragValue::new(&mut edited.x)
                        .range(i16::MIN..=i16::MAX)
                        .speed(1.0),
                );
                let y_response = ui.add(
                    egui::DragValue::new(&mut edited.y)
                        .range(i16::MIN..=i16::MAX)
                        .speed(1.0),
                );
                if x_response.changed() || y_response.changed() {
                    ui.data_mut(|data| data.insert_temp(position_key, edited.clone()));
                    let updated =
                        positioned_children_with_position(children, index, edited.x, edited.y);
                    update_draft(
                        ui.ctx(),
                        position_key,
                        design,
                        Some(Operation::SetChildren {
                            parent: ObjectTarget::id(parent_id.value()),
                            children: updated,
                        }),
                    );
                }
                let cancelled = (x_response.has_focus() || y_response.has_focus())
                    && ui.input(|input| input.key_pressed(egui::Key::Escape));
                if cancelled {
                    ui.data_mut(|data| data.remove::<PendingPosition>(position_key));
                    clear_draft(ui.ctx(), position_key);
                    edited.x = child_ref.offset.x;
                    edited.y = child_ref.offset.y;
                } else if drag_value_finished(&x_response)
                    || drag_value_finished(&y_response)
                    || (x_response.has_focus() || y_response.has_focus())
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                {
                    if edited.x != child_ref.offset.x || edited.y != child_ref.offset.y {
                        position = Some((index, edited.x, edited.y));
                    }
                    ui.data_mut(|data| data.remove::<PendingPosition>(position_key));
                    clear_draft(ui.ctx(), position_key);
                }
                row_actions(ui, index, children.len(), &mut action);
                ui.end_row();
            }
        });

    if let Some((index, child_id)) = replacement {
        queue_child_selection(
            design,
            parent_id,
            Some(index),
            NullableObjectId(Some(child_id)),
        );
    } else if let Some((index, x, y)) = position {
        let updated = positioned_children_with_position(children, index, x, y);
        design.queue_operation(Operation::SetChildren {
            parent: ObjectTarget::id(parent_id.value()),
            children: updated,
        });
    } else if let Some(action) = action {
        let mut updated = children.to_vec();
        apply_list_action(&mut updated, action);
        design.queue_operation(Operation::SetChildren {
            parent: ObjectTarget::id(parent_id.value()),
            children: updated
                .into_iter()
                .map(|child| ChildPlacement {
                    child: ObjectTarget::id(child.id.value()),
                    x: child.offset.x,
                    y: child.offset.y,
                })
                .collect(),
        });
    }

    ui.horizontal(|ui| {
        ui.label("Add object:");
        if let Some(Some(child)) = child_picker(
            ui,
            ("add_positioned_child", parent_id.value()),
            "Select existing object…",
            design,
            parent_id,
            None,
            None,
            &allowed,
            false,
        )
        .map(|id| id.0)
        {
            queue_child_selection(design, parent_id, None, NullableObjectId(Some(child)));
        }
    });
}

fn positioned_children_with_position(
    children: &[ObjectRef],
    index: usize,
    x: i16,
    y: i16,
) -> Vec<ChildPlacement> {
    children
        .iter()
        .enumerate()
        .map(|(child_index, child)| ChildPlacement {
            child: ObjectTarget::id(child.id.value()),
            x: if child_index == index {
                x
            } else {
                child.offset.x
            },
            y: if child_index == index {
                y
            } else {
                child.offset.y
            },
        })
        .collect()
}

fn apply_list_action<T>(list: &mut Vec<T>, action: ListAction) {
    match action {
        ListAction::MoveUp(index) => list.swap(index, index - 1),
        ListAction::MoveDown(index) => list.swap(index, index + 1),
        ListAction::Remove(index) => {
            list.remove(index);
        }
        ListAction::ReplaceMacro(_, _) => unreachable!(),
    }
}

fn render_object_list(
    ui: &mut egui::Ui,
    parent: &Object,
    objects: ObjectReferenceList,
    design: &EditorProject,
) {
    let parent_id = parent.id();
    let allowed = crate::allowed_object_relationships::get_allowed_child_refs(
        parent.object_type(),
        design.vt_version,
    );
    match objects {
        ObjectReferenceList::Required(objects) => {
            let mut replacement = None;
            let mut action = None;
            egui::Grid::new(("required_object_list", parent_id.value()))
                .striped(true)
                .show(ui, |ui| {
                    for (index, object_id) in objects.iter().copied().enumerate() {
                        ui.label((index + 1).to_string());
                        if let Some(new_id) = render_object_selector(
                            ui,
                            ("required_object", index),
                            design,
                            parent_id,
                            object_id,
                            index,
                            &allowed,
                        ) {
                            replacement = Some((index, new_id));
                        }
                        render_reference_links(ui, design, object_id);
                        row_actions(ui, index, objects.len(), &mut action);
                        ui.end_row();
                    }
                });
            if let Some((index, object_id)) = replacement {
                queue_child_selection(
                    design,
                    parent_id,
                    Some(index),
                    NullableObjectId(Some(object_id)),
                );
            } else if let Some(action) = action {
                let mut updated = objects.clone();
                apply_list_action(&mut updated, action);
                queue_object_list(design, parent_id, ObjectReferenceList::Required(updated));
            }
            render_add_object(ui, design, parent_id, &allowed, false, |object_id| {
                let mut updated = objects.clone();
                if let Some(object_id) = object_id {
                    updated.push(object_id);
                    queue_object_list(design, parent_id, ObjectReferenceList::Required(updated));
                }
            });
        }
        ObjectReferenceList::Nullable(objects) => {
            let mut replacement = None;
            let mut action = None;
            egui::Grid::new(("nullable_object_list", parent_id.value()))
                .striped(true)
                .show(ui, |ui| {
                    for (index, object_id) in objects.iter().copied().enumerate() {
                        ui.label((index + 1).to_string());
                        let mut selected = object_id;
                        let label = object_id.0.map_or_else(
                            || "None".to_owned(),
                            |id| {
                                design.get_pool().object_by_id(id).map_or_else(
                                    || {
                                        format!(
                                            "#{}: Missing object",
                                            design.format_object_id(id.value())
                                        )
                                    },
                                    |object| object_label(design, object),
                                )
                            },
                        );
                        if let Some(value) = child_picker(
                            ui,
                            ("nullable_object", index),
                            &label,
                            design,
                            parent_id,
                            object_id.0,
                            Some(index),
                            &allowed,
                            true,
                        ) {
                            selected = value;
                        }
                        if selected != object_id {
                            replacement = Some((index, selected));
                        }
                        if let Some(id) = object_id.0 {
                            render_reference_links(ui, design, id);
                        } else {
                            ui.label("");
                            ui.label("");
                        }
                        row_actions(ui, index, objects.len(), &mut action);
                        ui.end_row();
                    }
                });
            if let Some((index, object_id)) = replacement {
                queue_child_selection(design, parent_id, Some(index), object_id);
            } else if let Some(action) = action {
                let mut updated = objects.clone();
                apply_list_action(&mut updated, action);
                queue_object_list(design, parent_id, ObjectReferenceList::Nullable(updated));
            }
            render_add_object(ui, design, parent_id, &allowed, true, |object_id| {
                let mut updated = objects.clone();
                updated.push(NullableObjectId(object_id));
                queue_object_list(design, parent_id, ObjectReferenceList::Nullable(updated));
            });
        }
    }
}

fn render_reference_links(ui: &mut egui::Ui, design: &EditorProject, object_id: ObjectId) {
    if let Some(object) = design.get_pool().object_by_id(object_id) {
        if ui.link(format!("{:?}", object.object_type())).clicked() {
            select_object(design, object_id);
        }
        if ui.link(design.object_display_name(object)).clicked() {
            select_object(design, object_id);
        }
    } else {
        crate::ui_style::error_text(ui, "Missing object");
        ui.label("");
    }
}

fn render_add_object(
    ui: &mut egui::Ui,
    design: &EditorProject,
    parent_id: ObjectId,
    allowed: &[ObjectType],
    allow_none: bool,
    mut add: impl FnMut(Option<ObjectId>),
) {
    ui.horizontal(|ui| {
        ui.label("Add object:");
        if let Some(id) = child_picker(
            ui,
            ("add_object_list_item", parent_id.value()),
            "Select existing object…",
            design,
            parent_id,
            None,
            None,
            allowed,
            allow_none,
        ) {
            add(id.0);
        }
    });
}

fn queue_object_list(design: &EditorProject, id: ObjectId, objects: ObjectReferenceList) {
    design.queue_operation(Operation::SetObjectList {
        object: ObjectTarget::id(id.value()),
        objects: match objects {
            ObjectReferenceList::Required(ids) => ObjectTargetList::Required(
                ids.into_iter()
                    .map(|id| ObjectTarget::id(id.value()))
                    .collect(),
            ),
            ObjectReferenceList::Nullable(ids) => ObjectTargetList::Nullable(
                ids.into_iter()
                    .map(|id| id.0.map(|id| ObjectTarget::id(id.value())))
                    .collect(),
            ),
        },
    });
}

fn macro_object(design: &EditorProject, macro_id: u16) -> Option<&Object> {
    ObjectId::new(macro_id)
        .ok()
        .and_then(|id| design.get_pool().object_by_id(id))
        .filter(|object| object.object_type() == ObjectType::Macro)
}

fn apply_macro_action(references: &mut Vec<MacroRef>, action: ListAction) {
    match action {
        ListAction::ReplaceMacro(index, reference) => references[index] = reference,
        action => apply_list_action(references, action),
    }
}

fn queue_macro_references(design: &EditorProject, id: ObjectId, macro_refs: Vec<MacroRef>) {
    design.queue_operation(Operation::SetMacroReferences {
        object: ObjectTarget::id(id.value()),
        macro_refs: macro_refs
            .into_iter()
            .map(|reference| MacroReferenceTarget {
                event_id: reference.event_id,
                macro_object: ObjectTarget::id(reference.macro_id),
            })
            .collect(),
    });
}

fn render_add_macro_reference(
    ui: &mut egui::Ui,
    design: &EditorProject,
    id: ObjectId,
    references: &[MacroRef],
    events: &[Event],
) {
    if events.is_empty() {
        return;
    }
    let event_key = ui.make_persistent_id((id.value(), "new_macro_event"));
    let mut selected_event = ui.data(|data| data.get_temp::<Event>(event_key));
    ui.horizontal(|ui| {
        ui.label("Add macro:");
        egui::ComboBox::from_id_salt(("new_macro_event", id.value()))
            .icon(crate::ui_style::combo_box_icon)
            .selected_text(
                selected_event
                    .map_or_else(|| "Select event".to_owned(), |event| format!("{event:?}")),
            )
            .show_ui(ui, |ui| {
                for event in events {
                    if ui
                        .selectable_value(&mut selected_event, Some(*event), format!("{event:?}"))
                        .changed()
                    {
                        ui.data_mut(|data| data.insert_temp(event_key, *event));
                    }
                }
            });
        if let Some(event_id) = selected_event.filter(|event| events.contains(event)) {
            if let Some(Some(candidate)) = crate::object_picker::button(
                ui,
                ("new_macro_object", id.value()),
                "Select macro…",
                &format!(
                    "Attach macro for {event_id:?} to {}",
                    crate::object_picker::object_label(design, Some(id))
                ),
                design,
                None,
                false,
                |candidate| {
                    candidate.object_type() == ObjectType::Macro
                        && crate::vt_compatibility::supported_event(
                            design
                                .get_pool()
                                .object_by_id(id)
                                .expect("macro owner is rendered from the pool"),
                            event_id,
                            design.vt_version,
                        )
                        && (candidate.id().value() <= u8::MAX.into()
                            || design.vt_version
                                >= ag_iso_stack::object_pool::vt_version::VtVersion::Version5)
                },
                |ui, choice| {
                    crate::object_picker::impact::review_queued(ui, design, |candidate| {
                        if let Some(choice) = choice.0 {
                            let mut updated = references.to_vec();
                            updated.push(MacroRef {
                                event_id,
                                macro_id: choice.value(),
                            });
                            queue_macro_references(candidate, id, updated);
                        }
                    })
                },
            )
            .map(|id| id.0)
            {
                let mut updated = references.to_vec();
                updated.push(MacroRef {
                    event_id,
                    macro_id: candidate.value(),
                });
                queue_macro_references(design, id, updated);
                ui.data_mut(|data| data.remove::<Event>(event_key));
            }
        }
    });
}

fn render_macro_references(
    ui: &mut egui::Ui,
    object: &Object,
    id: ObjectId,
    design: &EditorProject,
) {
    let Some(references) = crate::operations::operation::macro_refs(object) else {
        return;
    };
    ui.label("Macros");
    let events = crate::possible_events::get_possible_events_for_version(object, design.vt_version);
    for (index, reference) in references.iter().enumerate() {
        if !crate::vt_compatibility::supported_event(object, reference.event_id, design.vt_version)
        {
            repair_warning(
                ui,
                &format!("Macro {}: {:?}", index + 1, reference.event_id),
                design.vt_version,
                |version| {
                    crate::vt_compatibility::supported_event(object, reference.event_id, version)
                },
            );
        }
        if reference.macro_id > u8::MAX.into() && design.vt_version < VtVersion::Version5 {
            repair_warning(
                ui,
                &format!("Macro {}: extended reference", index + 1),
                design.vt_version,
                |version| {
                    crate::vt_compatibility::supported_property(
                        object.object_type(),
                        "extended_macro_reference",
                        version,
                    )
                },
            );
        }
    }
    let mut action = None;
    egui::Grid::new(("macro_grid", id.value()))
        .striped(true)
        .min_col_width(0.0)
        .show(ui, |ui| {
            for (index, reference) in references.iter().enumerate() {
                let mut edited = reference.clone();
                egui::ComboBox::from_id_salt(("macro_event", index))
                    .icon(crate::ui_style::combo_box_icon)
                    .selected_text(format!("{:?}", reference.event_id))
                    .show_ui(ui, |ui| {
                        for event in &events {
                            ui.selectable_value(&mut edited.event_id, *event, format!("{event:?}"));
                        }
                    });

                let macro_object = macro_object(design, reference.macro_id);
                let selected_text = macro_object.map_or_else(
                    || {
                        format!(
                            "{} - Missing macro",
                            design.format_object_id(reference.macro_id)
                        )
                    },
                    |object| object_label(design, object),
                );
                if let Some(Some(candidate)) = crate::object_picker::button(
                    ui,
                    ("macro_object", index),
                    &selected_text,
                    &format!(
                        "Replace {:?} macro on {}",
                        reference.event_id,
                        crate::object_picker::object_label(design, Some(id))
                    ),
                    design,
                    macro_object.map(|object| object.id()),
                    false,
                    |candidate| {
                        candidate.object_type() == ObjectType::Macro
                            && (candidate.id().value() <= u8::MAX.into()
                                || design.vt_version
                                    >= ag_iso_stack::object_pool::vt_version::VtVersion::Version5)
                    },
                    |ui, choice| {
                        crate::object_picker::impact::review_queued(ui, design, |candidate| {
                            if let Some(choice) = choice.0 {
                                let mut updated = references.to_vec();
                                updated[index] = edited.clone();
                                updated[index].macro_id = choice.value();
                                queue_macro_references(candidate, id, updated);
                            }
                        })
                    },
                )
                .map(|id| id.0)
                {
                    edited.macro_id = candidate.value();
                }
                if let Some(macro_object) = macro_object {
                    if ui.link("View").clicked() {
                        select_object(design, macro_object.id());
                    }
                } else {
                    ui.label("");
                }
                row_actions(ui, index, references.len(), &mut action);
                if &edited != reference && action.is_none() {
                    action = Some(ListAction::ReplaceMacro(index, edited));
                }
                ui.end_row();
            }
        });

    if let Some(action) = action {
        let mut updated = references.to_vec();
        apply_macro_action(&mut updated, action);
        queue_macro_references(design, id, updated);
    }
    render_add_macro_reference(ui, design, id, references, &events);
}

fn render_identity(
    ui: &mut egui::Ui,
    id: ObjectId,
    object_type: ObjectType,
    design: &EditorProject,
) {
    let key = ui.make_persistent_id(("object_id", id.value()));
    touch_draft(ui.ctx(), key);
    let mut overridden = design
        .get_pool()
        .object_by_id(id)
        .is_some_and(|object| design.get_object_info(object).id_overridden);
    let mut value = ui
        .data(|data| match data.get_temp::<PendingValue>(key) {
            Some(PendingValue::Number(value)) if overridden => Some(value as u16),
            _ => None,
        })
        .unwrap_or_else(|| id.value());
    ui.horizontal(|ui| {
        ui.label("Assigned ID");
        ui.monospace(design.format_object_id(id.value()));
        ui.label(if overridden {
            "Manual override"
        } else {
            "Automatic"
        });
        ui.label(format!("Type: {object_type:?}"));
    });
    ui.horizontal(|ui| {
        let toggled = ui.checkbox(&mut overridden, "Override ID").changed();
        if toggled {
            let assignment = if overridden {
                Ok(id)
            } else {
                design.allocate_object_id(object_type)
            };
            match assignment {
                Ok(new_id) => {
                    design.queue_object_id_assignment(id, new_id, overridden);
                    clear_draft(ui.ctx(), key);
                    value = new_id.value();
                    ui.data_mut(|data| {
                        data.remove::<PendingValue>(key);
                        data.remove::<String>(key.with("error"));
                    });
                }
                Err(error) => {
                    overridden = true;
                    ui.data_mut(|data| data.insert_temp(key.with("error"), error.to_string()));
                }
            }
        }
        let format = design.object_id_format();
        let response = ui.add_enabled(
            overridden,
            egui::DragValue::new(&mut value)
                .range(0..=65534)
                .custom_formatter(|value, _| design.format_object_id(value as u16))
                .custom_parser(move |text| format.parse(text).ok().map(f64::from)),
        );
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(key, PendingValue::Number(value.into())));
        }
        let conflict = value != id.value()
            && design.materialize_shared().map_or(true, |pool| {
                ObjectId::new(value)
                    .ok()
                    .is_some_and(|candidate| pool.pool.object_by_id(candidate).is_some())
            });
        let invalid_macro = object_type == ObjectType::Macro
            && design.vt_version() < ag_iso_stack::object_pool::vt_version::VtVersion::Version5
            && value > 255;
        if response.changed() {
            let operation =
                (!conflict && !invalid_macro).then_some(Operation::SetObjectIdAssignment {
                    object_id: id.value(),
                    new_id: value,
                    overridden: true,
                });
            update_draft(ui.ctx(), key, design, operation);
        }
        if overridden
            && !toggled
            && value != id.value()
            && !conflict
            && !invalid_macro
            && (drag_value_finished(&response)
                || response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
        {
            design.queue_operation(Operation::SetObjectIdAssignment {
                object_id: id.value(),
                new_id: value,
                overridden: true,
            });
            ui.data_mut(|data| {
                data.remove::<PendingValue>(key);
                data.remove::<String>(key.with("error"));
            });
            clear_draft(ui.ctx(), key);
        }
        if conflict {
            crate::ui_style::error_text(ui, "ID already in use");
        }
        if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            value = id.value();
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        }
        if invalid_macro {
            crate::ui_style::error_text(ui, "VT2–4 macro IDs must be 0–255");
        }
    });
    if let Some(error) = ui.data(|data| data.get_temp::<String>(key.with("error"))) {
        crate::ui_style::error_text(ui, &error);
    }
}

fn queue(design: &EditorProject, id: ObjectId, property: PropertyKey, value: serde_json::Value) {
    design.queue_operation(property_operation(design, id, property, value));
}

fn property_operation(
    design: &EditorProject,
    id: ObjectId,
    property: PropertyKey,
    value: serde_json::Value,
) -> Operation {
    let is_reference = design.get_pool().object_by_id(id).is_some_and(|object| {
        get_property_descriptors(object.object_type())
            .into_iter()
            .any(|descriptor| {
                descriptor.name == property
                    && matches!(
                        descriptor.semantic,
                        PropertySemantic::ObjectReference { .. }
                    )
            })
    });
    if is_reference {
        let reference = if value.is_null() {
            None
        } else {
            serde_json::from_value::<ObjectId>(value)
                .ok()
                .map(|id| ObjectTarget::id(id.value()))
        };
        Operation::SetObjectReference {
            object: ObjectTarget::id(id.value()),
            property,
            reference,
        }
    } else {
        Operation::SetProperty {
            object: ObjectTarget::id(id.value()),
            property,
            value,
        }
    }
}

fn repair_warning(
    ui: &mut egui::Ui,
    label: &str,
    version: VtVersion,
    supports: impl FnMut(VtVersion) -> bool,
) {
    let availability = crate::vt_compatibility::availability_description(supports)
        .unwrap_or_else(|| "Unavailable in supported VT versions".to_owned());
    ui.colored_label(
        ui.visuals().warn_fg_color,
        format!(
            "{label}: incompatible with VT{}. {availability}. Change the value to repair it.",
            u8::from(version)
        ),
    );
}

fn unavailable_features(object: &Object, version: VtVersion) -> Vec<String> {
    use crate::vt_compatibility as compatibility;
    let object_type = object.object_type();
    let mut entries = Vec::new();
    let mut add = |label: String, supports: &dyn Fn(VtVersion) -> bool| {
        if !supports(version) {
            if let Some(availability) = compatibility::availability_description(supports) {
                entries.push(format!("{label} — {availability}"));
            }
        }
    };
    add(format!("{object_type:?} objects"), &|v| {
        compatibility::supported_object_type(object_type, v)
    });
    let descriptors = get_property_descriptors(object_type);
    for editor in property_editor_descriptors(object_type) {
        let EditorTarget::Property(property_key) = editor.property else {
            continue;
        };
        let property_name = property_key.as_str();
        let label = human_readable_property_name(property_name);
        add(label.clone(), &|v| {
            compatibility::supported_property(object_type, property_name, v)
        });
        let Ok(value) = get_object_property(object, property_key) else {
            continue;
        };
        match editor.editor {
            EditorKind::Enum => {
                if let Some(descriptor) = descriptors.iter().find(|item| item.name == property_key)
                {
                    for choice in &descriptor.enum_choices {
                        let choice_label = if property_key == PropertyKey::FontSize
                            && choice.get("Proportional").is_some()
                        {
                            "Proportional font sizes".to_owned()
                        } else {
                            format!("{label}: {}", enum_choice_label(choice))
                        };
                        add(choice_label, &|v| {
                            compatibility::supported_enum_value(
                                object_type,
                                property_key,
                                choice,
                                v,
                            )
                        });
                    }
                }
            }
            EditorKind::FlagSet => {
                if let Some(fields) = value.as_object() {
                    for (field, value) in fields {
                        if value.is_boolean()
                            && !(property_key == PropertyKey::FontStyle && field == "proportional")
                        {
                            add(
                                format!("{label}: {}", human_readable_property_name(field)),
                                &|v| {
                                    compatibility::supported_property_flag(
                                        object_type,
                                        property_key,
                                        field,
                                        v,
                                    )
                                },
                            );
                        }
                    }
                }
            }
            EditorKind::Justification => {
                add("Vertical alignment".to_owned(), &|v| {
                    compatibility::supported_vertical_alignment(
                        ag_iso_stack::object_pool::object_attributes::VerticalAlignment::Middle,
                        v,
                    )
                });
            }
            _ => {}
        }
        if let Some(descriptor) = descriptors.iter().find(|item| item.name == property_key) {
            if let PropertySemantic::ObjectReference { allowed_types } = &descriptor.semantic {
                for target_type in allowed_types {
                    add(format!("{label}: {target_type:?} references"), &|v| {
                        compatibility::supported_object_type(*target_type, v)
                    });
                }
            }
        }
    }
    if crate::operations::operation::macro_refs(object).is_some() {
        for event in crate::possible_events::get_possible_events(object) {
            add(format!("Macro event: {event:?}"), &|v| {
                compatibility::supported_event(object, event, v)
            });
        }
        add("Macro references with IDs above 255".to_owned(), &|v| {
            compatibility::supported_property(object_type, "extended_macro_reference", v)
        });
    }
    if object_type == ObjectType::InputBoolean {
        add("Boolean values above 1".to_owned(), &|v| {
            compatibility::supported_input_boolean_value(2, v)
        });
    }
    entries.sort();
    entries.dedup();
    entries
}

/// Only offer a new value when the normal transaction validator accepts it for
/// the current target VT. This keeps legacy values visible while allowing a
/// repair edit to move them back into the supported range.
fn property_choice_is_valid(
    design: &EditorProject,
    id: ObjectId,
    property: PropertyKey,
    value: serde_json::Value,
) -> bool {
    design.property_choice_is_valid(id, property, value)
}

/// Preserve the JSON number kind exposed by the property's Rust type. In
/// particular, integer properties must be committed as `42`, not `42.0`:
/// serde_json deliberately does not expose the latter through `as_u64()`.
fn number_after_edit(
    original: &serde_json::Value,
    edited: f64,
    property: PropertyKey,
) -> Option<serde_json::Number> {
    let serde_json::Value::Number(original) = original else {
        return None;
    };

    match property {
        PropertyKey::Offset => {
            if edited.is_finite()
                && edited.fract() == 0.0
                && (i32::MIN as f64..=i32::MAX as f64).contains(&edited)
            {
                return Some(serde_json::Number::from(edited as i32));
            }
            return None;
        }
        PropertyKey::ViewportX
        | PropertyKey::ViewportY
        | PropertyKey::GraphicsCursorX
        | PropertyKey::GraphicsCursorY => {
            if edited.is_finite()
                && edited.fract() == 0.0
                && (i16::MIN as f64..=i16::MAX as f64).contains(&edited)
            {
                return Some(serde_json::Number::from(edited as i16));
            }
            return None;
        }
        _ => {}
    }

    if original.is_u64() {
        if edited.is_finite() && edited.fract() == 0.0 && (0.0..=u64::MAX as f64).contains(&edited)
        {
            return Some(serde_json::Number::from(edited as u64));
        }
        return None;
    }

    if original.is_i64() {
        if edited.is_finite()
            && edited.fract() == 0.0
            && (i64::MIN as f64..=i64::MAX as f64).contains(&edited)
        {
            return Some(serde_json::Number::from(edited as i64));
        }
        return None;
    }

    serde_json::Number::from_f64(edited)
}

fn render_auto(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    match value {
        serde_json::Value::Bool(current) => {
            let mut edited = *current;
            if ui.checkbox(&mut edited, label).changed() {
                queue(design, id, property, edited.into());
            }
        }
        serde_json::Value::Number(_) => render_number(
            ui,
            label,
            value,
            numeric_range(property),
            id,
            property,
            design,
        ),
        serde_json::Value::String(current) => render_text(ui, label, current, id, property, design),
        _ => render_json(ui, label, value, id, property, design),
    }
}

fn render_text(
    ui: &mut egui::Ui,
    label: &str,
    current: &str,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    let key = ui.make_persistent_id((id.value(), property));
    touch_draft(ui.ctx(), key);
    let mut edited = pending_text(ui, key).unwrap_or_else(|| current.to_owned());
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.text_edit_singleline(&mut edited);
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new(("test_property_input_rect", id.value(), property)),
                response.rect,
            )
        });
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(key, PendingValue::Text(edited.clone())));
            update_draft(
                ui.ctx(),
                key,
                design,
                Some(property_operation(
                    design,
                    id,
                    property,
                    edited.clone().into(),
                )),
            );
        }
        let cancelled =
            response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape));
        if cancelled {
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        } else if (response.lost_focus()
            || response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
            && edited != current
        {
            queue(design, id, property, edited.into());
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        }
    });
}

fn render_number(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    range: std::ops::RangeInclusive<f64>,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    let Some(current) = value.as_f64() else {
        render_json(ui, label, value, id, property, design);
        return;
    };
    let key = ui.make_persistent_id((id.value(), property));
    touch_draft(ui.ctx(), key);
    let mut edited = ui
        .data(|data| match data.get_temp::<PendingValue>(key) {
            Some(PendingValue::Number(value)) => Some(value),
            _ => None,
        })
        .unwrap_or(current);
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.add(egui::DragValue::new(&mut edited).range(range).speed(1.0));
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(key, PendingValue::Number(edited)));
            let operation = number_after_edit(value, edited, property)
                .map(serde_json::Value::Number)
                .map(|value| property_operation(design, id, property, value));
            update_draft(ui.ctx(), key, design, operation);
        }
        let cancelled =
            response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape));
        if cancelled {
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        } else if drag_value_finished(&response)
            || response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
        {
            if edited != current {
                if let Some(number) = number_after_edit(value, edited, property) {
                    queue(design, id, property, number.into());
                }
            }
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        }
    });
}

/// Keep the editor's keyboard range aligned with the serialized Rust field
/// type. Offset is signed by the object-pool definition even when its current
/// value is zero, so it must never inherit the unsigned default range.
fn numeric_range(property: PropertyKey) -> std::ops::RangeInclusive<f64> {
    match property {
        PropertyKey::Offset => i32::MIN as f64..=i32::MAX as f64,
        PropertyKey::ViewportX
        | PropertyKey::ViewportY
        | PropertyKey::GraphicsCursorX
        | PropertyKey::GraphicsCursorY => i16::MIN as f64..=i16::MAX as f64,
        PropertyKey::Scale | PropertyKey::ViewportZoom => f32::MIN as f64..=f32::MAX as f64,
        _ => 0.0..=u64::MAX as f64,
    }
}

fn render_picture_dimensions(
    ui: &mut egui::Ui,
    picture: &ag_iso_stack::object_pool::object::PictureGraphic,
    id: ObjectId,
    design: &EditorProject,
) {
    let source_width = picture.actual_width as u32;
    let source_height = picture.actual_height as u32;
    ui.label(format!(
        "Source resolution: {source_width} × {source_height} px"
    ));
    if source_width == 0 || source_height == 0 {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "Display height is unavailable for an image with zero source dimensions.",
        );
        return;
    }
    let key = ui.make_persistent_id((id.value(), "picture_display_height"));
    touch_draft(ui.ctx(), key);
    let current_height = displayed_picture_height(picture.width, source_width, source_height);
    let mut edited = ui
        .data(|data| match data.get_temp::<PendingValue>(key) {
            Some(PendingValue::Number(value)) => Some(value as u16),
            _ => None,
        })
        .unwrap_or(current_height);
    ui.horizontal(|ui| {
        ui.label("Display height:");
        let response = ui.add(egui::DragValue::new(&mut edited).range(1..=u16::MAX));
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(key, PendingValue::Number(edited.into())));
            let width = picture_width_for_display_height(edited, source_width, source_height);
            let candidate = serde_json::json!(width);
            let operation = (edited != current_height)
                .then(|| property_operation(design, id, PropertyKey::Width, candidate));
            update_draft(ui.ctx(), key, design, operation);
        }
        let cancelled =
            response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape));
        if cancelled {
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        } else if drag_value_finished(&response)
            || response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
        {
            let width = picture_width_for_display_height(edited, source_width, source_height);
            let candidate = serde_json::json!(width);
            if edited != current_height {
                queue(design, id, PropertyKey::Width, candidate);
            }
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        }
        ui.label(format!(
            "Current: {} × {} px",
            picture.width, current_height
        ));
    });
}

fn picture_width_for_display_height(height: u16, source_width: u32, source_height: u32) -> u16 {
    if source_width == 0 || source_height == 0 {
        return 0;
    }
    let numerator = u64::from(height) * u64::from(source_width);
    let denominator = u64::from(source_height);
    let ceil_ideal_width = numerator.div_ceil(denominator);
    let mut candidates = vec![1_u16, u16::MAX];
    for candidate in [
        ceil_ideal_width.saturating_sub(1),
        ceil_ideal_width,
        ceil_ideal_width + 1,
    ] {
        candidates.push(candidate.clamp(1, u64::from(u16::MAX)) as u16);
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
        .into_iter()
        .min_by_key(|&width| {
            let rendered = displayed_picture_height(width, source_width, source_height);
            let rendered_error = u64::from(rendered.abs_diff(height));
            let aspect_error = (u64::from(width) * denominator).abs_diff(numerator);
            (rendered_error, aspect_error, width)
        })
        .unwrap_or(1)
}

fn displayed_picture_height(width: u16, source_width: u32, source_height: u32) -> u16 {
    if source_width == 0 {
        return 0;
    }
    (u64::from(source_height) * u64::from(width) / u64::from(source_width)).min(u64::from(u16::MAX))
        as u16
}

/// Enum-backed option bits keep their serialized enum representation when edited.
fn flag_choices(field: &str) -> Vec<serde_json::Value> {
    let variants: &[&str] = match field {
        "state" => &["Released", "Latched"],
        "color" => &["ForegroundBackground", "LineFontFill"],
        "deflection_direction" => &["AntiClockwise", "Clockwise"],
        "bar_graph_type" => &["Filled", "NotFilled"],
        "axis_orientation" => &["Vertical", "Horizontal"],
        "grow_direction" => &["GrowLeftDown", "GrowRightUp"],
        "data_code_type" => &["Raw", "RunLength"],
        "animation_sequence" => &["SingleShot", "Loop"],
        "disabled_behaviour" => &["Pause", "ResetToFirst", "DefaultObject", "Blank"],
        "function_type" => {
            use ag_iso_stack::object_pool::object_attributes::AuxiliaryFunctionType;
            return (0..=14) // IDs 0–14 are valid AuxiliaryFunctionType values
                .map(|value| serde_json::to_value(AuxiliaryFunctionType::from(value)).unwrap())
                .collect();
        }
        _ => &[],
    };
    variants
        .iter()
        .map(|value| serde_json::json!(value))
        .collect()
}

/// Independently worded editor guidance for the encodings in ISO 11783-6:2018,
/// J.4.6, Table J.5. Keep protocol facts exact; do not copy the standard’s prose.
fn auxiliary_function_type_help(
    function_type: ag_iso_stack::object_pool::object_attributes::AuxiliaryFunctionType,
) -> (String, String) {
    use ag_iso_stack::object_pool::object_attributes::AuxiliaryFunctionType::*;

    let (label, behavior, values) = match function_type {
        BooleanLatching => (
            "Boolean — latching",
            "Choose this for an on/off control that keeps its state after the operator lets go.",
            "Value 1: 0 = off (backward/down/left/not pressed); 1 = on (forward/up/right/pressed).\nValue 2: running total of activations after power-up.",
        ),
        AnalogueMaintains => (
            "Analogue — maintains position",
            "Choose this when releasing a variable control leaves its output unchanged.",
            "Value 1: 0–100%.\nValue 2: unused for this type; transmit 0xFFFF.",
        ),
        BooleanNonLatching => (
            "Boolean — momentary",
            "Choose this for an on/off control that is active only while operated, such as a push button.",
            "Value 1: 0 = off; 1 = momentary activation (forward/up/right/pressed); 2 = held.\nValue 2: running total of activations after power-up. Continuing to hold an active control adds nothing.",
        ),
        AnalogueReturnToCenter => (
            "Analogue — returns to 50%",
            "Choose this for a variable control with travel on both sides of neutral. Letting go restores a 50% output.",
            "Value 1: 0–100%.\nValue 2: unused for this type; transmit 0xFFFF.",
        ),
        AnalogueReturnToZero => (
            "Analogue — returns to 0%",
            "Choose this for a variable control whose output falls back to zero when the operator lets go.",
            "Value 1: 0–100%.\nValue 2: unused for this type; transmit 0xFFFF.",
        ),
        DualBooleanLatching => (
            "Dual Boolean — both directions latching",
            "Choose this when both directional states and the neutral state remain selected after release.",
            "Value 1: 0 = centre/off; 1 = forward/up/right; 4 = backward/down/left.\nValue 2: running total of activations after power-up.",
        ),
        DualBooleanNonLatching => (
            "Dual Boolean — both directions momentary",
            "Choose this for two opposing actions that stop on release, such as holding a rocker to raise or lower a tool.",
            "Value 1: 0 = off; 1 = momentary forward/up/right; 2 = held forward/up/right; 4 = momentary backward/down/left; 8 = held backward/down/left.\nValue 2: running total of activations after power-up. Continuing to hold an active control adds nothing.",
        ),
        DualBooleanLatchingUp => (
            "Dual Boolean — latching up, momentary down",
            "The forward/up/right action stays selected after release. The opposite action requires continued operation; releasing it selects off. Off is also a stable setting.",
            "Value 1: 0 = off; 1 = latched forward/up/right; 4 = momentary backward/down/left; 8 = held backward/down/left.\nValue 2: running total of activations after power-up. Continuing to hold an active control adds nothing.",
        ),
        DualBooleanLatchingDown => (
            "Dual Boolean — latching down, momentary up",
            "The backward/down/left action stays selected after release. The opposite action requires continued operation; releasing it selects off. Off is also a stable setting.",
            "Value 1: 0 = off; 1 = momentary forward/up/right; 2 = held forward/up/right; 4 = latched backward/down/left.\nValue 2: running total of activations after power-up. Continuing to hold an active control adds nothing.",
        ),
        CombinedAnalogueReturnWithLatch => (
            "Combined analogue — returns to 50%, latching endpoints",
            "Use this for variable movement around a 50% neutral output, plus a persistent directional state available at either end of travel.",
            "Value 1: 0–100%; 0xFB00 = latched forward; 0xFB01 = latched backward.\nValue 2: running total of latch engagements after power-up. An engagement is counted only when the previous state was not latched.",
        ),
        CombinedAnalogueMaintainsWithLatch => (
            "Combined analogue — maintains position, latching endpoints",
            "Use this when a variable setting remains after release and either end of travel can also select a persistent directional state.",
            "Value 1: 0–100%; 0xFB00 = latched forward; 0xFB01 = latched backward.\nValue 2: running total of latch engagements after power-up. An engagement is counted only when the previous state was not latched.",
        ),
        QuadratureBooleanNonLatching => (
            "Quadrature Boolean — momentary",
            "Use this for a two-axis directional control that becomes neutral on release. Each direction has its own two-bit state, allowing a change between held directions to be represented in one report.",
            "Value 1: bits 1–0 = forward/up; bits 3–2 = backward/down; bits 5–4 = right; bits 7–6 = left. Each pair: 00 = off; 01 = first activation; 10 = held; 11 = reserved.\nValue 2: running total of activations across both axes after power-up. Continuing to hold an active direction adds nothing.",
        ),
        QuadratureAnalogueMaintains => (
            "Quadrature analogue — maintains position",
            "Use this for two independent, perpendicular variable controls whose settings remain after release. A 50% reading marks the midpoint of either axis.",
            "Value 1: axis 1, 0% = backward/down; 100% = forward/up.\nValue 2: axis 2, 0% = left; 100% = right.",
        ),
        QuadratureAnalogueReturnToCenter => (
            "Quadrature analogue — returns to 50%",
            "Use this for a two-axis variable control that settles at a 50% reading on each axis when the operator lets go.",
            "Value 1: axis 1, 0% = backward/down; 100% = forward/up.\nValue 2: axis 2, 0% = left; 100% = right.",
        ),
        BidirectionalEncoder => (
            "Bidirectional encoder",
            "Use this for rotation measured in steps. Turning one way adds to the count; reversing direction subtracts from it.",
            "Value 1: current count, 0–0xFFFF, wrapping between 0xFFFF and 0.\nValue 2: fixed calibration, 1–0xFFFF encoder counts per revolution.",
        ),
        _ => (
            "Invalid",
            "This auxiliary function type is not supported to be used.",
            "No control behavior or status values are defined.",
        ),
    };
    let type_id = u8::from(function_type);
    let mut help = format!("{behavior}\n\nStatus report fields\n{values}");
    if matches!(type_id, 1 | 3 | 4 | 9 | 10 | 12 | 13) {
        help.push_str("\n\nAnalogue values encode 0–100% as 0–0xFAFF.");
        if !matches!(type_id, 12 | 13) {
            help.push_str(
                " 0% = backward/down/left/counter-clockwise; 100% = forward/up/right/clockwise.",
            );
        }
    }
    if matches!(type_id, 0 | 2 | 5..=11) {
        help.push_str(
            "\n\nAfter a transition counter reaches 0xFFFF, its next increment produces 0.",
        );
    }
    help.push_str("\n\nTechnical reference: ISO 11783-6:2018, J.4.6, Table J.5.");
    (format!("{type_id}: {label}"), help)
}

/// Keep the selected value and menu entries consistent, including type-specific help.
fn flag_choice_presentation(field: &str, value: &serde_json::Value) -> (String, Option<String>) {
    if field == "function_type" {
        if let Ok(function_type) = serde_json::from_value(value.clone()) {
            let (label, help) = auxiliary_function_type_help(function_type);
            return (label, Some(help));
        }
    }
    let label = value
        .as_str()
        .map(human_readable_property_name)
        .unwrap_or_else(|| value.to_string());
    (label, None)
}

fn render_flag_field(
    ui: &mut egui::Ui,
    field: &str,
    value: &serde_json::Value,
    fields: &serde_json::Map<String, serde_json::Value>,
    id: ObjectId,
    object_type: ObjectType,
    property: PropertyKey,
    design: &EditorProject,
) -> Option<serde_json::Value> {
    let label = human_readable_property_name(field);
    if let Some(mut enabled) = value.as_bool() {
        if !crate::vt_compatibility::supported_property_flag(
            object_type,
            property,
            field,
            design.vt_version,
        ) {
            if !enabled {
                return None;
            }
            repair_warning(ui, &label, design.vt_version, |version| {
                crate::vt_compatibility::supported_property_flag(
                    object_type,
                    property,
                    field,
                    version,
                )
            });
        }
        let mut updated = fields.clone();
        let candidate_enabled = !enabled;
        updated.insert(field.to_owned(), candidate_enabled.into());
        let version_allows = !candidate_enabled
            || crate::vt_compatibility::supported_property_flag(
                object_type,
                property,
                field,
                design.vt_version,
            );
        let response = ui.add_enabled(
            version_allows
                && property_choice_is_valid(
                    design,
                    id,
                    property,
                    serde_json::Value::Object(updated),
                ),
            egui::Checkbox::new(&mut enabled, label),
        );
        return response.changed().then_some(enabled.into());
    }
    let choices = flag_choices(field);
    let mut updated = None;
    let (selected_label, selected_help) = flag_choice_presentation(field, value);
    let response = ui
        .horizontal(|ui| {
            let label_response = ui.label(&label);
            egui::ComboBox::from_id_salt(&label)
                .icon(crate::ui_style::combo_box_icon)
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    for choice in choices {
                        let (label, help) = flag_choice_presentation(field, &choice);
                        let mut candidate_fields = fields.clone();
                        candidate_fields.insert(field.to_owned(), choice.clone());
                        let valid = value == &choice
                            || property_choice_is_valid(
                                design,
                                id,
                                property,
                                serde_json::Value::Object(candidate_fields),
                            );
                        let mut response = ui
                            .add_enabled(valid, egui::Button::selectable(value == &choice, label));
                        if let Some(help) = help {
                            response = response.on_hover_text(help);
                        }
                        if response.clicked() && value != &choice {
                            updated = Some(choice);
                        }
                    }
                })
                .response
                .labelled_by(label_response.id)
        })
        .inner;
    if let Some(help) = selected_help {
        response.on_hover_text(help);
    }
    if field == "function_type" {
        ui.small("Hover over a type for control behavior and status values.")
            .on_hover_text("For a Type 2 assignment, select the same type ID for the input and the function. This rule applies to both manual assignments and assignments made by a Working Set. ISO 15077 specifies the applicable requirements for the physical operator controls.");
    }
    updated
}

/// Independent bits and booleans use checkboxes; mutually exclusive enum values
/// use dropdowns. Both preserve the property's serialized representation.
fn render_flag_set(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    id: ObjectId,
    object_type: ObjectType,
    property: PropertyKey,
    design: &EditorProject,
) {
    if let Some(current) = value.as_u64() {
        let bits = if property == PropertyKey::LineArt {
            16
        } else {
            8
        };
        ui.label(label);
        ui.horizontal_wrapped(|ui| {
            for bit in (0..bits).rev() {
                let mask = 1_u64 << bit;
                let mut enabled = current & mask != 0;
                let candidate = if enabled {
                    current & !mask
                } else {
                    current | mask
                };
                let response = ui
                    .add_enabled(
                        property_choice_is_valid(design, id, property, candidate.into()),
                        egui::Checkbox::new(&mut enabled, format!("{bit}")),
                    )
                    .on_hover_text(format!("Bit {bit}"));
                if response.changed() {
                    let updated = if enabled {
                        current | mask
                    } else {
                        current & !mask
                    };
                    queue(design, id, property, updated.into());
                }
            }
        });
    } else if let Some(fields) = value.as_object() {
        ui.label(label);
        for (field, value) in fields {
            // The font-size selector already chooses fixed/proportional mode.
            if property == PropertyKey::FontStyle && field == "proportional" {
                continue;
            }
            let edited = ui
                .push_id(field, |ui| {
                    render_flag_field(ui, field, value, fields, id, object_type, property, design)
                })
                .inner;
            if let Some(edited) = edited {
                let mut updated = fields.clone();
                updated.insert(field.clone(), edited);
                let updated = serde_json::Value::Object(updated);
                queue(design, id, property, updated);
            }
        }
    } else {
        render_json(ui, label, value, id, property, design);
    }
}

fn render_reference(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    descriptor: &crate::object_properties::PropertyDescriptor,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    let PropertySemantic::ObjectReference { allowed_types } = &descriptor.semantic else {
        return;
    };
    let selected = value.as_u64();
    ui.horizontal(|ui| {
        ui.label(label);
        let selected_id = selected.and_then(|id| ObjectId::new(id as u16).ok());
        let referenced_object = selected_id.and_then(|id| design.get_pool().object_by_id(id));
        let label = selected_id.map_or_else(
            || "None".into(),
            |id| {
                referenced_object.map_or_else(
                    || format!("Missing object #{}", design.format_object_id(id.value())),
                    |object| object_selector_label(design, object),
                )
            },
        );
        let allow_none = design.get_pool().object_by_id(id).is_some_and(|object| {
            crate::object_properties::property_accepts_null(object, property)
        });
        if let Some(reference) = crate::object_picker::button(
            ui,
            (id.value(), property),
            &label,
            &format!(
                "Change {property} reference on {}",
                crate::object_picker::object_label(design, Some(id))
            ),
            design,
            selected_id,
            allow_none,
            |candidate| {
                (allowed_types.is_empty() || allowed_types.contains(&candidate.object_type()))
                    && crate::vt_compatibility::supported_object_type(
                        candidate.object_type(),
                        design.vt_version,
                    )
                    && validate_pointer_target(design, id, property, candidate.id()).is_ok()
            },
            |ui, reference| {
                crate::object_picker::impact::review_queued(ui, design, |candidate| {
                    queue(
                        candidate,
                        id,
                        property,
                        reference
                            .0
                            .map_or(serde_json::Value::Null, |id| serde_json::json!(id.value())),
                    );
                })
            },
        ) {
            queue(
                design,
                id,
                property,
                reference
                    .0
                    .map_or(serde_json::Value::Null, |id| serde_json::json!(id.value())),
            );
        }
        if let Some(object) = referenced_object {
            if ui
                .link("View")
                .on_hover_text("View referenced object")
                .clicked()
            {
                select_object(design, object.id());
            }
        }
    });
}

fn queue_allowed_references(design: &EditorProject, pointer: ObjectId, references: &[ObjectId]) {
    design.queue_operation(Operation::SetObjectPointerAllowedReferences {
        object: ObjectTarget::id(pointer.value()),
        references: references
            .iter()
            .map(|id| ObjectTarget::id(id.value()))
            .collect(),
    });
}

fn render_pointer_reference(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    descriptor: &crate::object_properties::PropertyDescriptor,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    let PropertySemantic::ObjectReference { allowed_types } = &descriptor.semantic else {
        return;
    };
    let selected_id = value.as_u64().and_then(|id| ObjectId::new(id as u16).ok());
    let references = design
        .get_pool()
        .object_by_id(id)
        .map(|object| design.get_object_info(object).allowed_references)
        .unwrap_or_default();

    if references.is_empty() {
        render_reference(ui, label, value, descriptor, id, property, design);
    } else {
        ui.horizontal(|ui| {
            ui.label(label);
            let selected_label = selected_id.map_or_else(
                || "None".to_owned(),
                |target| crate::object_picker::object_label(design, Some(target)),
            );
            egui::ComboBox::from_id_salt((id.value(), property, "allowed_reference"))
                .icon(crate::ui_style::combo_box_icon)
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(selected_id.is_none(), "None").clicked()
                        && selected_id.is_some()
                    {
                        queue(design, id, property, serde_json::Value::Null);
                    }
                    for target in &references {
                        let Some(candidate) = design.get_pool().object_by_id(*target) else {
                            ui.add_enabled(
                                false,
                                egui::Button::new(format!(
                                    "Missing object #{}",
                                    design.format_object_id(target.value())
                                )),
                            )
                            .on_disabled_hover_text("This allowed object no longer exists.");
                            continue;
                        };
                        if selected_id != Some(*target)
                            && !crate::vt_compatibility::supported_object_type(
                                candidate.object_type(),
                                design.vt_version,
                            )
                        {
                            continue;
                        }
                        let choice = crate::object_picker::object_label(design, Some(*target));
                        let reason = if !allowed_types.is_empty()
                            && !allowed_types.contains(&candidate.object_type())
                        {
                            Some("This object type is not valid for this reference.".to_owned())
                        } else if !crate::vt_compatibility::supported_object_type(
                            candidate.object_type(),
                            design.vt_version,
                        ) {
                            Some(format!(
                                "This object type is unavailable in VT{}.",
                                u8::from(design.vt_version)
                            ))
                        } else {
                            validate_pointer_target(design, id, property, *target).err()
                        };
                        let response = ui.add_enabled(
                            reason.is_none(),
                            egui::Button::selectable(selected_id == Some(*target), choice),
                        );
                        if let Some(reason) = reason {
                            response.on_disabled_hover_text(reason);
                        } else if response.clicked() && selected_id != Some(*target) {
                            queue(design, id, property, serde_json::json!(target.value()));
                        }
                    }
                });
            if let Some(object) =
                selected_id.and_then(|target| design.get_pool().object_by_id(target))
            {
                if ui
                    .link("View")
                    .on_hover_text("View referenced object")
                    .clicked()
                {
                    select_object(design, object.id());
                }
            }
        });
    }

    ui.horizontal(|ui| {
        ui.label("Allowed references:");
        if let Some(reference) = crate::object_picker::button(
            ui,
            (id.value(), property, "add_allowed_reference"),
            "Add",
            &format!(
                "Add an allowed reference for {}",
                crate::object_picker::object_label(design, Some(id))
            ),
            design,
            None,
            false,
            |candidate| {
                candidate.id() != id
                    && !references.contains(&candidate.id())
                    && crate::vt_compatibility::supported_object_type(
                        candidate.object_type(),
                        design.vt_version,
                    )
                    && (allowed_types.is_empty()
                        || allowed_types.contains(&candidate.object_type()))
                    && crate::pool_validation::would_create_circular_reference(
                        design.get_pool(),
                        id,
                        candidate.id(),
                    )
                    .is_ok()
            },
            |_, _| true,
        ) {
            if let Some(reference) = reference.0 {
                let mut updated = references.clone();
                updated.push(reference);
                queue_allowed_references(design, id, &updated);
            }
        }
    });
    for target in &references {
        ui.horizontal(|ui| {
            ui.label(crate::object_picker::object_label(design, Some(*target)));
            if ui.small_button("Remove").clicked() {
                let updated: Vec<_> = references
                    .iter()
                    .copied()
                    .filter(|reference| reference != target)
                    .collect();
                queue_allowed_references(design, id, &updated);
            }
        });
    }
}

fn validate_pointer_target(
    design: &EditorProject,
    pointer: ObjectId,
    property: PropertyKey,
    target: ObjectId,
) -> Result<(), String> {
    let mut transaction = OperationTransaction::new(None);
    transaction.add_operation(Operation::SetObjectReference {
        object: ObjectTarget::id(pointer.value()),
        property,
        reference: Some(ObjectTarget::id(target.value())),
    });
    design
        .validate_transaction(&transaction)
        .map_err(|diagnostics| design.format_transaction_diagnostics(&diagnostics))
}

fn render_justification(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    let Some(fields) = value.as_object() else {
        render_json(ui, label, value, id, property, design);
        return;
    };
    let horizontal = fields
        .get("horizontal")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Left");
    let vertical = fields
        .get("vertical")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Top");
    if design.vt_version < VtVersion::Version4 && vertical != "Top" {
        repair_warning(ui, "Vertical alignment", design.vt_version, |version| {
            crate::vt_compatibility::supported_vertical_alignment(
                ag_iso_stack::object_pool::object_attributes::VerticalAlignment::Middle,
                version,
            )
        });
    }
    ui.horizontal(|ui| {
        let horizontal_label = ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .icon(crate::ui_style::combo_box_icon)
            .selected_text(horizontal)
            .show_ui(ui, |ui| {
                for choice in ["Left", "Middle", "Right"] {
                    if ui.selectable_label(horizontal == choice, choice).clicked() {
                        let mut updated = fields.clone();
                        updated.insert("horizontal".to_owned(), choice.into());
                        queue(design, id, property, serde_json::Value::Object(updated));
                    }
                }
            })
            .response
            .labelled_by(horizontal_label.id);
        if design.vt_version < VtVersion::Version4 && vertical == "Top" {
            return;
        }
        let vertical_label = ui.label("Vertical");
        egui::ComboBox::from_id_salt("Vertical")
            .icon(crate::ui_style::combo_box_icon)
            .selected_text(vertical)
            .show_ui(ui, |ui| {
                for choice in ["Top", "Middle", "Bottom"] {
                    let alignment = match choice {
                        "Top" => {
                            ag_iso_stack::object_pool::object_attributes::VerticalAlignment::Top
                        }
                        "Middle" => {
                            ag_iso_stack::object_pool::object_attributes::VerticalAlignment::Middle
                        }
                        _ => {
                            ag_iso_stack::object_pool::object_attributes::VerticalAlignment::Bottom
                        }
                    };
                    let supported = crate::vt_compatibility::supported_vertical_alignment(
                        alignment,
                        design.vt_version,
                    );
                    let response = ui.add_enabled(
                        supported,
                        egui::Button::selectable(vertical == choice, choice),
                    );
                    if !supported && response.contains_pointer() {
                        if let Some(availability) =
                            crate::vt_compatibility::availability_description(|version| {
                                crate::vt_compatibility::supported_vertical_alignment(
                                    alignment, version,
                                )
                            })
                        {
                            response.show_tooltip_text(availability);
                        }
                    }
                    if response.clicked() && choice != vertical {
                        let mut updated = fields.clone();
                        updated.insert("vertical".to_owned(), choice.into());
                        let updated = serde_json::Value::Object(updated);
                        if property_choice_is_valid(design, id, property, updated.clone()) {
                            queue(design, id, property, updated);
                        }
                    }
                }
            })
            .response
            .labelled_by(vertical_label.id);
    });
}

fn render_json(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    id: ObjectId,
    property: PropertyKey,
    design: &EditorProject,
) {
    let key = ui.make_persistent_id((id.value(), property));
    touch_draft(ui.ctx(), key);
    let original = serde_json::to_string(value).unwrap_or_default();
    let mut edited = pending_text(ui, key).unwrap_or_else(|| original.clone());
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::multiline(&mut edited)
            .desired_rows(2)
            .code_editor(),
    );
    if response.changed() {
        ui.data_mut(|data| data.insert_temp(key, PendingValue::Text(edited.clone())));
        let operation = serde_json::from_str(&edited)
            .ok()
            .map(|value| property_operation(design, id, property, value));
        update_draft(ui.ctx(), key, design, operation);
    }
    let cancelled = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape));
    if cancelled {
        ui.data_mut(|data| data.remove::<PendingValue>(key));
        clear_draft(ui.ctx(), key);
    } else if response.lost_focus()
        || response.has_focus()
            && ui.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::Enter))
    {
        if edited == original {
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
            return;
        }
        if let Ok(parsed) = serde_json::from_str(&edited) {
            queue(design, id, property, parsed);
            ui.data_mut(|data| data.remove::<PendingValue>(key));
            clear_draft(ui.ctx(), key);
        } else {
            crate::ui_style::error_text(ui, "Invalid JSON");
        }
    }
}

fn pending_text(ui: &egui::Ui, key: egui::Id) -> Option<String> {
    ui.data(|data| match data.get_temp::<PendingValue>(key) {
        Some(PendingValue::Text(value)) => Some(value),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_object, OperationTransaction};
    use ag_iso_stack::object_pool::ObjectPool;

    #[test]
    fn signed_number_offsets_accept_negative_values_from_zero() {
        let range = numeric_range(PropertyKey::Offset);
        assert!(*range.start() < 0.0);
        assert_eq!(*range.end(), i32::MAX as f64);
        let negative = number_after_edit(&serde_json::json!(0), -1.0, PropertyKey::Offset)
            .expect("zero-valued signed offsets accept negative edits");
        assert_eq!(i32::try_from(negative.as_i64().unwrap()).unwrap(), -1);
        let negative_position =
            number_after_edit(&serde_json::json!(0), -1.0, PropertyKey::ViewportX)
                .expect("zero-valued signed positions accept negative edits");
        assert_eq!(
            i16::try_from(negative_position.as_i64().unwrap()).unwrap(),
            -1
        );
        assert!(number_after_edit(
            &serde_json::json!(0),
            i32::MAX as f64 + 1.0,
            PropertyKey::Offset
        )
        .is_none());
    }

    #[test]
    fn picture_height_maps_to_nearest_aspect_ratio_width_and_guards_zero_source() {
        assert_eq!(picture_width_for_display_height(30, 40, 20), 60);
        let width = picture_width_for_display_height(2, 100, 60);
        assert_eq!(width, 4);
        assert_eq!(displayed_picture_height(width, 100, 60), 2);
        let unreachable = picture_width_for_display_height(u16::MAX, 2, 1);
        assert_eq!(unreachable, u16::MAX);
        assert_eq!(displayed_picture_height(unreachable, 2, 1), 32767);
        assert_eq!(picture_width_for_display_height(1, 1, 3), 1);
        assert_eq!(picture_width_for_display_height(20, 0, 10), 0);
        assert_eq!(picture_width_for_display_height(20, 10, 0), 0);
    }

    #[test]
    fn selection_change_finishes_text_draft_for_original_object_as_one_undo_step() {
        let first = object(ObjectType::InputAttributes, 10);
        let second = object(ObjectType::InputAttributes, 11);
        let mut pool = ObjectPool::default();
        pool.add(first);
        pool.add(second);
        let mut project = EditorProject::from(pool);
        let first_id = ObjectId::new(10).unwrap();
        let second_id = ObjectId::new(11).unwrap();
        let ctx = egui::Context::default();

        editor_frame_for(&ctx, &project, first_id, vec![]);
        let input_rect = ctx
            .data(|data| {
                data.get_temp::<egui::Rect>(egui::Id::new((
                    "test_property_input_rect",
                    first_id.value(),
                    PropertyKey::ValidationString,
                )))
            })
            .expect("validation string input was rendered");
        let pos = input_rect.center();
        editor_frame_for(
            &ctx,
            &project,
            first_id,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        editor_frame_for(
            &ctx,
            &project,
            first_id,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        editor_frame_for(
            &ctx,
            &project,
            first_id,
            vec![egui::Event::Text("draft".to_owned())],
        );
        assert!(!project.commit_queued("premature draft"));

        editor_frame_for(&ctx, &project, second_id, vec![]);
        assert!(project.commit_queued("Finish previous property edit"));
        assert_eq!(
            get_object_property(
                project.get_pool().object_by_id(first_id).unwrap(),
                PropertyKey::ValidationString,
            )
            .unwrap(),
            serde_json::json!("draft")
        );
        assert_eq!(
            get_object_property(
                project.get_pool().object_by_id(second_id).unwrap(),
                PropertyKey::ValidationString,
            )
            .unwrap(),
            serde_json::json!("")
        );
        assert!(project.undo_operation());
        assert_eq!(
            get_object_property(
                project.get_pool().object_by_id(first_id).unwrap(),
                PropertyKey::ValidationString,
            )
            .unwrap(),
            serde_json::json!("")
        );
    }

    fn object(object_type: ObjectType, id: u16) -> Object {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        // Structural-edit tests use Buttons as parents. Give them a usable
        // content area so placement behavior is independent of zero defaults.
        if let Object::Button(button) = &mut object {
            button.width = 100;
            button.height = 100;
        }
        object
    }

    fn editor_frame(
        ctx: &egui::Context,
        project: &EditorProject,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect)> {
        let object_id = project.get_pool().objects().first().unwrap().id();
        editor_frame_for(ctx, project, object_id, events)
    }

    fn editor_frame_for(
        ctx: &egui::Context,
        project: &EditorProject,
        object_id: ObjectId,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect)> {
        editor_frame_for_time(ctx, project, object_id, events, None)
    }

    fn editor_frame_for_time(
        ctx: &egui::Context,
        project: &EditorProject,
        object_id: ObjectId,
        events: Vec<egui::Event>,
        time: Option<f64>,
    ) -> Vec<(String, egui::Rect)> {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 1800.0),
                )),
                time,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let object = project.get_pool().object_by_id(object_id).unwrap();
                    render_property_editor(ui, object, project);
                });
                finish_property_edits(ctx, project);
            },
        );
        output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                )),
                _ => None,
            })
            .collect()
    }

    fn hover_editor_item(
        ctx: &egui::Context,
        project: &EditorProject,
        object_id: ObjectId,
        pos: egui::Pos2,
    ) -> Vec<(String, egui::Rect)> {
        let time = ctx.input(|input| input.time);
        editor_frame_for_time(
            ctx,
            project,
            object_id,
            vec![egui::Event::PointerMoved(pos)],
            Some(time + 0.01),
        );
        editor_frame_for_time(ctx, project, object_id, vec![], Some(time + 1.0))
    }

    fn click_editor_text(
        ctx: &egui::Context,
        project: &EditorProject,
        label: &str,
    ) -> Vec<(String, egui::Rect)> {
        let text = editor_frame(ctx, project, vec![]);
        let pos = text
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("Missing {label}: {text:?}"))
            .1
            .center();
        click_editor_at(ctx, project, pos)
    }

    fn click_editor_at(
        ctx: &egui::Context,
        project: &EditorProject,
        pos: egui::Pos2,
    ) -> Vec<(String, egui::Rect)> {
        editor_frame(
            ctx,
            project,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        editor_frame(
            ctx,
            project,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        editor_frame(ctx, project, vec![])
    }

    fn project_at_version(object: Object, version: u8) -> EditorProject {
        let mut pool = ObjectPool::default();
        pool.add(object);
        let project = EditorProject::from(pool);
        load_at_version(&project, version)
    }

    fn load_at_version(project: &EditorProject, version: u8) -> EditorProject {
        let mut saved: serde_json::Value =
            serde_json::from_slice(&project.save_project().unwrap()).unwrap();
        saved["settings"]["vt_version"] = version.into();
        EditorProject::load_project(serde_json::to_vec(&saved).unwrap()).unwrap()
    }

    #[test]
    fn identity_controls_show_assignment_and_override_checkbox() {
        let project = project_at_version(object(ObjectType::Button, 6000), 6);
        let frame = editor_frame(&egui::Context::default(), &project, vec![]);
        for label in ["Assigned ID", "Automatic", "Override ID"] {
            assert!(
                frame.iter().any(|(text, _)| text == label),
                "missing {label}"
            );
        }
        assert!(!frame.iter().any(|(text, _)| text == "Use automatic ID"));
    }

    fn edit_identity_id(
        ctx: &egui::Context,
        project: &EditorProject,
        input: &str,
    ) -> Vec<(String, egui::Rect)> {
        let current = project.get_pool().objects().first().unwrap().id().value();
        let frame = editor_frame(ctx, project, vec![]);
        // The assigned-ID label precedes the editable field with the same text.
        let pos = frame
            .iter()
            .rev()
            .find(|(text, _)| text == &project.format_object_id(current))
            .unwrap()
            .1
            .center();
        click_editor_at(ctx, project, pos);
        editor_frame(
            ctx,
            project,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text(input.to_owned()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        editor_frame(ctx, project, vec![])
    }

    #[test]
    fn identity_id_is_editable_only_with_override_enabled() {
        let mut project = project_at_version(object(ObjectType::Button, 6000), 6);
        let ctx = egui::Context::default();
        edit_identity_id(&ctx, &project, "6100");
        assert!(!project.commit_queued("Disabled ID edit"));

        click_editor_text(&ctx, &project, "Override ID");
        assert!(project.commit_queued("Enable override"));
        assert!(project.object_id_is_overridden(ObjectId::new(6000).unwrap()));
        edit_identity_id(&ctx, &project, "6100");
        assert!(project.commit_queued("Edit override"));
        assert!(project
            .get_pool()
            .object_by_id(ObjectId::new(6100).unwrap())
            .is_some());
        assert!(project.object_id_is_overridden(ObjectId::new(6100).unwrap()));

        click_editor_text(&ctx, &project, "Override ID");
        assert!(project.commit_queued("Return to automatic"));
        let automatic = project.get_pool().objects().first().unwrap().id();
        assert_eq!(automatic.value(), 6000);
        assert!(!project.object_id_is_overridden(automatic));
        edit_identity_id(&ctx, &project, "6200");
        assert!(!project.commit_queued("Disabled automatic ID edit"));
        assert!(project.undo_operation());
        assert!(project.object_id_is_overridden(ObjectId::new(6100).unwrap()));
        assert!(project.redo_operation());
        assert!(!project.object_id_is_overridden(automatic));
    }

    #[test]
    fn identity_override_rejects_duplicate_and_legacy_macro_ids() {
        for (object_type, version, input, error) in [
            (ObjectType::Button, 6, "6001", "ID already in use"),
            (ObjectType::Macro, 4, "256", "VT2–4 macro IDs must be 0–255"),
        ] {
            let mut pool = ObjectPool::default();
            pool.add(object(object_type, 10));
            pool.add(object(ObjectType::Button, 6001));
            let mut project = load_at_version(&EditorProject::from(pool), version);
            let ctx = egui::Context::default();
            click_editor_text(&ctx, &project, "Override ID");
            assert!(project.commit_queued("Enable override"));
            let frame = edit_identity_id(&ctx, &project, input);
            assert!(frame.iter().any(|(text, _)| text == error), "{frame:?}");
            assert!(!project.commit_queued("Invalid override"));
            assert!(project
                .get_pool()
                .object_by_id(ObjectId::new(10).unwrap())
                .is_some());
        }
    }

    #[test]
    fn identity_override_drag_applies_only_on_release() {
        let mut project = project_at_version(object(ObjectType::Button, 6000), 6);
        let ctx = egui::Context::default();
        click_editor_text(&ctx, &project, "Override ID");
        assert!(project.commit_queued("Enable override"));
        let frame = editor_frame(&ctx, &project, vec![]);
        let start = frame
            .iter()
            .rev()
            .find(|(text, _)| text == "6000")
            .unwrap()
            .1
            .center();
        editor_frame(
            &ctx,
            &project,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let end = start + egui::vec2(30.0, 0.0);
        editor_frame(&ctx, &project, vec![egui::Event::PointerMoved(end)]);
        assert!(!project.commit_queued("Unfinished ID drag"));
        editor_frame(
            &ctx,
            &project,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(project.commit_queued("Finished ID drag"));
        assert_ne!(
            project.get_pool().objects().first().unwrap().id().value(),
            6000
        );
    }

    #[test]
    fn identity_override_remains_enabled_when_automatic_range_is_full() {
        let mut project = project_at_version(object(ObjectType::Button, 6000), 6);
        let mut ranges = project.object_id_ranges().clone();
        ranges.set_range(
            ObjectType::Button,
            crate::ObjectIdRange {
                start: 6000,
                end: 6000,
            },
        );
        project.set_object_id_ranges(ranges).unwrap();
        let ctx = egui::Context::default();
        click_editor_text(&ctx, &project, "Override ID");
        assert!(project.commit_queued("Enable override"));
        let error = project
            .allocate_object_id(ObjectType::Button)
            .unwrap_err()
            .to_string();
        let frame = click_editor_text(&ctx, &project, "Override ID");
        assert!(!project.commit_queued("Failed automatic assignment"));
        assert!(project.object_id_is_overridden(ObjectId::new(6000).unwrap()));
        assert!(frame.iter().any(|(text, _)| text == &error), "{frame:?}");
        edit_identity_id(&ctx, &project, "6100");
        assert!(project.commit_queued("Edit retained override"));
        let frame = editor_frame(&ctx, &project, vec![]);
        assert!(!frame.iter().any(|(text, _)| text == &error));
    }

    #[test]
    fn pointer_menu_hides_unsupported_targets_but_preserves_management() {
        let mut pool = ObjectPool::default();
        pool.add(object(ObjectType::ObjectPointer, 1));
        pool.add(object(ObjectType::OutputList, 2));
        pool.add(object(ObjectType::Button, 3));
        let mut project = EditorProject::from(pool);
        apply(
            &mut project,
            Operation::SetObjectPointerAllowedReferences {
                object: ObjectTarget::id(1),
                references: vec![ObjectTarget::id(2), ObjectTarget::id(3)],
            },
        );
        let project = load_at_version(&project, 3);
        let unsupported =
            crate::object_picker::object_label(&project, Some(ObjectId::new(2).unwrap()));
        let supported =
            crate::object_picker::object_label(&project, Some(ObjectId::new(3).unwrap()));
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.animation_time = 0.0);
        let text = click_editor_text(&ctx, &project, "None");
        // One label belongs to management; a second would be a dropdown choice.
        assert_eq!(
            text.iter().filter(|(text, _)| text == &unsupported).count(),
            1
        );
        assert_eq!(
            text.iter().filter(|(text, _)| text == &supported).count(),
            2
        );
    }

    #[test]
    fn vt_controls_hide_and_availability_note_expands() {
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.animation_time = 0.0);
        let mut project = project_at_version(object(ObjectType::Button, 1), 3);
        let text = editor_frame(&ctx, &project, vec![]);
        assert!(!text
            .iter()
            .any(|(text, _)| text == "Disabled" || text.contains("Available from VT4")));
        assert!(text.iter().any(|(text, _)| text == "Unavailable in VT3"));
        let text = click_editor_text(&ctx, &project, "Unavailable in VT3");
        assert!(text
            .iter()
            .any(|(text, _)| text == "Options: Disabled — Available from VT4"));
        project.set_vt_version(VtVersion::Version4).unwrap();
        assert!(editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Disabled"));
        project.undo();
        assert!(!editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Disabled"));
        project.redo();
        assert!(editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Disabled"));
    }

    #[test]
    fn imported_unsupported_flag_remains_repairable_then_hides() {
        let mut button = object(ObjectType::Button, 1);
        let Object::Button(value) = &mut button else {
            unreachable!()
        };
        value.options.disabled = true;
        let mut project = project_at_version(button, 3);
        let original = project.save_project().unwrap();
        let ctx = egui::Context::default();
        let text = editor_frame(&ctx, &project, vec![]);
        assert!(text.iter().any(|(text, _)| text == "Disabled"));
        assert!(text
            .iter()
            .any(|(text, _)| text.contains("Disabled: incompatible with VT3. Available from VT4")));
        assert_eq!(original, project.save_project().unwrap());
        click_editor_text(&ctx, &project, "Disabled");
        assert!(project.commit_queued("Repair disabled flag"));
        assert!(!editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Disabled"));
        assert!(!project
            .compatibility_diagnostics()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("VT_")));
        project.undo();
        assert!(editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Disabled"));
    }

    #[test]
    fn legacy_vertical_selector_hides_unless_needed_for_repair() {
        let string = object(ObjectType::OutputString, 1);
        let ctx = egui::Context::default();
        let project = project_at_version(string.clone(), 3);
        assert!(!editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Vertical"));
        let project = project_at_version(string.clone(), 4);
        assert!(editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Vertical"));
        let mut string = string;
        let Object::OutputString(value) = &mut string else {
            unreachable!()
        };
        value.justification.vertical =
            ag_iso_stack::object_pool::object_attributes::VerticalAlignment::Middle;
        let mut project = project_at_version(string, 3);
        let text = editor_frame(&ctx, &project, vec![]);
        assert!(text.iter().any(|(text, _)| text == "Vertical"));
        assert!(text
            .iter()
            .any(|(text, _)| text.contains("Vertical alignment: incompatible with VT3")));
        let text = click_editor_text(&ctx, &project, "Middle");
        let bottom = text
            .iter()
            .find(|(text, _)| text == "Bottom")
            .expect("VT4-only vertical choices remain visible")
            .1
            .center();
        let hover_text = hover_editor_item(&ctx, &project, ObjectId::new(1).unwrap(), bottom);
        assert!(
            hover_text
                .iter()
                .any(|(text, _)| text == "Available from VT4"),
            "{hover_text:?}"
        );
        let after_disabled_click = click_editor_at(&ctx, &project, bottom);
        assert!(!project.commit_queued("Select unavailable vertical alignment"));
        if !after_disabled_click.iter().any(|(text, _)| text == "Top") {
            click_editor_text(&ctx, &project, "Middle");
        }
        click_editor_text(&ctx, &project, "Top");
        assert!(project.commit_queued("Repair vertical alignment"));
        assert!(!editor_frame(&ctx, &project, vec![])
            .iter()
            .any(|(text, _)| text == "Vertical"));
    }

    #[test]
    fn font_and_macro_notes_follow_version_boundaries() {
        let font = object(ObjectType::FontAttributes, 1);
        let features = unavailable_features(&font, VtVersion::Version3);
        assert_eq!(
            features
                .iter()
                .filter(|entry| entry.contains("Proportional font sizes"))
                .count(),
            1
        );
        assert!(features
            .iter()
            .any(|entry| entry.contains("Font Type: Greek — Available from VT4")));
        assert_eq!(
            unavailable_features(&font, VtVersion::Version4),
            vec!["Macro references with IDs above 255 — Available from VT5",]
        );
        let mask = object(ObjectType::DataMask, 1);
        let features = unavailable_features(&mask, VtVersion::Version4);
        assert!(features
            .iter()
            .any(|entry| entry == "Macro event: OnPointingEventPress — Available from VT5"));
        assert!(features
            .iter()
            .any(|entry| entry == "Macro references with IDs above 255 — Available from VT5"));
        assert!(unavailable_features(&mask, VtVersion::Version5).is_empty());
    }

    #[test]
    fn font_dropdown_disables_incompatible_choices_and_keeps_them_repairable() {
        use ag_iso_stack::object_pool::object_attributes::FontType;
        let descriptor = get_property_descriptors(ObjectType::FontAttributes)
            .into_iter()
            .find(|descriptor| descriptor.name == PropertyKey::FontType)
            .unwrap();
        for (version, greek_supported) in [(3, false), (4, true)] {
            let ctx = egui::Context::default();
            let font = object(ObjectType::FontAttributes, 1);
            let mut project = project_at_version(font, version);
            let text = click_editor_text(&ctx, &project, "Latin1");
            assert!(text.iter().any(|(text, _)| text == "Greek"), "{text:?}");
            if !greek_supported {
                let current = get_object_property(
                    project.get_pool().objects().first().unwrap(),
                    PropertyKey::FontType,
                )
                .unwrap();
                let greek_position = text
                    .iter()
                    .find(|(text, _)| text == "Greek")
                    .unwrap()
                    .1
                    .center();
                let hover_text =
                    hover_editor_item(&ctx, &project, ObjectId::new(1).unwrap(), greek_position);
                assert!(
                    hover_text
                        .iter()
                        .any(|(text, _)| text == "Available from VT4"),
                    "{hover_text:?}"
                );
                let after_disabled_click = click_editor_at(&ctx, &project, greek_position);
                assert!(!project.commit_queued("Select unavailable font encoding"));

                let compatible_choice = descriptor
                    .enum_choices
                    .iter()
                    .find(|choice| {
                        *choice != &current
                            && crate::vt_compatibility::supported_enum_value(
                                ObjectType::FontAttributes,
                                PropertyKey::FontType,
                                choice,
                                VtVersion::Version3,
                            )
                    })
                    .unwrap();
                let compatible_label = enum_choice_label(compatible_choice);
                if !after_disabled_click
                    .iter()
                    .any(|(text, _)| text == &compatible_label)
                {
                    click_editor_text(&ctx, &project, &enum_choice_label(&current));
                }
                click_editor_text(&ctx, &project, &compatible_label);
                assert!(project.commit_queued("Select compatible font encoding"));
            }
        }
        let mut font = object(ObjectType::FontAttributes, 1);
        let Object::FontAttributes(value) = &mut font else {
            unreachable!()
        };
        value.font_type = FontType::Greek;
        let mut project = project_at_version(font, 3);
        let ctx = egui::Context::default();
        let text = editor_frame(&ctx, &project, vec![]);
        assert!(text.iter().any(|(text, _)| {
            text.contains("Font Type: incompatible with VT3. Available from VT4")
        }));
        let text = click_editor_text(&ctx, &project, "Greek");
        assert!(text.iter().any(|(text, _)| text == "Greek"));
        click_editor_text(&ctx, &project, "Latin1");
        assert!(project.commit_queued("Repair font encoding"));
        let text = editor_frame(&ctx, &project, vec![]);
        assert!(!text
            .iter()
            .any(|(text, _)| text == "Greek" || text.contains("incompatible with VT3")));
    }

    fn apply(project: &mut EditorProject, operation: Operation) {
        let mut transaction = OperationTransaction::new(Some("Structural edit".to_owned()));
        transaction.add_operation(operation);
        project.execute_transaction(transaction).unwrap();
    }

    fn placements(refs: &[ObjectRef]) -> Vec<ChildPlacement> {
        refs.iter()
            .map(|reference| ChildPlacement {
                child: ObjectTarget::id(reference.id.value()),
                x: reference.offset.x,
                y: reference.offset.y,
            })
            .collect()
    }

    fn macro_targets(refs: &[MacroRef]) -> Vec<MacroReferenceTarget> {
        refs.iter()
            .map(|reference| MacroReferenceTarget {
                event_id: reference.event_id,
                macro_object: ObjectTarget::id(reference.macro_id),
            })
            .collect()
    }

    #[test]
    fn reference_view_navigates_without_editing_and_preserves_history() {
        for (source_type, property, target_type) in [
            (
                ObjectType::ObjectPointer,
                PropertyKey::Value,
                ObjectType::ObjectPointer,
            ),
            (
                ObjectType::OutputNumber,
                PropertyKey::VariableReference,
                ObjectType::NumberVariable,
            ),
            (
                ObjectType::OutputString,
                PropertyKey::FontAttributes,
                ObjectType::FontAttributes,
            ),
        ] {
            let mut pool = ObjectPool::default();
            pool.add(object(source_type, 1));
            pool.add(object(target_type, 2));
            let mut project = EditorProject::from(pool);
            let source = ObjectId::new(1).unwrap();
            let target = ObjectId::new(2).unwrap();
            select_object(&project, source);
            project.update_selected();
            // Object labels lazily initialize default metadata during rendering.
            for object in project.get_pool().objects() {
                project.get_object_info(object);
            }
            let before = project.save_project().unwrap();
            let descriptor = get_property_descriptors(source_type)
                .into_iter()
                .find(|descriptor| descriptor.name == property)
                .unwrap();
            let ctx = egui::Context::default();
            let mut position = None;
            // Resolve the link position from rendered text, then click it.
            for pressed in [None, Some(true), Some(false)] {
                let events = pressed.map_or_else(Vec::new, |pressed| {
                    let pos = position.unwrap();
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                });
                let output = ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render_reference(
                                ui,
                                property.as_str(),
                                &serde_json::json!(2),
                                &descriptor,
                                source,
                                property,
                                &project,
                            );
                        });
                    },
                );
                if pressed.is_none() {
                    position = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "View" => {
                            Some(text.pos + text.galley.size() * 0.5)
                        }
                        _ => None,
                    });
                    assert!(position.is_some());
                }
            }
            assert!(project.update_selected());
            assert_eq!(project.get_selected().0, Some(target));
            assert!(!project.commit_queued("Navigation"));
            project.set_previous_selected();
            assert_eq!(project.get_selected().0, Some(source));
            // Saving includes the last selection; compare at the original selection.
            assert_eq!(project.save_project().unwrap(), before);
            project.set_next_selected();
            assert_eq!(project.get_selected().0, Some(target));
        }
    }

    #[test]
    fn null_and_missing_references_have_no_view_link() {
        let mut pool = ObjectPool::default();
        pool.add(object(ObjectType::ObjectPointer, 1));
        let project = EditorProject::from(pool);
        let descriptor = get_property_descriptors(ObjectType::ObjectPointer)
            .into_iter()
            .find(|descriptor| descriptor.name == PropertyKey::Value)
            .unwrap();
        for (value, expected) in [
            (serde_json::Value::Null, "None"),
            (serde_json::json!(42), "Missing object #42"),
        ] {
            let ctx = egui::Context::default();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_reference(
                        ui,
                        "Value",
                        &value,
                        &descriptor,
                        ObjectId::new(1).unwrap(),
                        PropertyKey::Value,
                        &project,
                    );
                });
            });
            let text: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect();
            assert!(text.contains(&expected), "{text:?}");
            assert!(!text.contains(&"View"));
        }
    }

    #[test]
    fn every_enum_flag_has_dropdown_choices_that_validate() {
        for object_type in (0..=u8::MAX).filter_map(|value| ObjectType::try_from(value).ok()) {
            let object = object(object_type, 1);
            let id = object.id();
            let mut pool = ObjectPool::default();
            pool.add(object.clone());
            for editor in property_editor_descriptors(object_type) {
                if !matches!(editor.editor, EditorKind::FlagSet) {
                    continue;
                }
                let EditorTarget::Property(property_key) = editor.property else {
                    continue;
                };
                let value = get_object_property(&object, property_key).unwrap();
                if value.is_u64() {
                    continue;
                }
                let fields = value
                    .as_object()
                    .expect("flags must be a bit mask or object");
                for (field, current) in fields {
                    if current.is_boolean() {
                        continue;
                    }
                    let choices = flag_choices(field);
                    assert!(
                        choices.contains(current),
                        "{object_type:?}.{}.{field}: {current}",
                        property_key
                    );
                    for choice in choices {
                        let mut updated = fields.clone();
                        updated.insert(field.clone(), choice);
                        crate::object_properties::validate_property(
                            &pool,
                            id,
                            property_key,
                            &serde_json::Value::Object(updated),
                        )
                        .unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn numeric_edits_preserve_integer_and_float_json_kinds() {
        let integer = number_after_edit(
            &serde_json::json!(0_u8),
            42.0,
            PropertyKey::BackgroundColour,
        )
        .unwrap();
        assert_eq!(integer.as_u64(), Some(42));
        assert!(!integer.is_f64());

        let mut pool = ObjectPool::default();
        let button = object(ObjectType::Button, 1002);
        let button_id = button.id();
        pool.add(button);
        assert!(crate::object_properties::validate_property(
            &pool,
            button_id,
            PropertyKey::BackgroundColour,
            &serde_json::Value::Number(integer),
        )
        .is_ok());

        let float =
            number_after_edit(&serde_json::json!(0.0_f64), 42.0, PropertyKey::Value).unwrap();
        assert!(float.is_f64());
        assert_eq!(float.as_f64(), Some(42.0));
    }

    #[test]
    fn positioned_children_reorder_remove_and_cycle_validation_are_reversible() {
        let parent = object(ObjectType::Button, 10);
        let first = object(ObjectType::PictureGraphic, 11);
        let second = object(ObjectType::PictureGraphic, 12);
        let mut pool = ObjectPool::default();
        for object in [parent, first, second] {
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        let initial = vec![
            ObjectRef {
                id: ObjectId::new(11).unwrap(),
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 1, y: 2 },
            },
            ObjectRef {
                id: ObjectId::new(12).unwrap(),
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 3, y: 4 },
            },
        ];
        apply(
            &mut project,
            Operation::SetChildren {
                parent: ObjectTarget::id(10),
                children: placements(&initial),
            },
        );
        let changed = vec![initial[1], initial[0]];
        apply(
            &mut project,
            Operation::SetChildren {
                parent: ObjectTarget::id(10),
                children: placements(&changed),
            },
        );
        assert_eq!(
            crate::operations::operation::object_refs(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(10).unwrap())
                    .unwrap()
            ),
            Some(changed.as_slice())
        );
        assert!(project.undo_operation());
        assert_eq!(
            crate::operations::operation::object_refs(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(10).unwrap())
                    .unwrap()
            ),
            Some(initial.as_slice())
        );
        assert!(project.redo_operation());
        assert_eq!(
            crate::operations::operation::object_refs(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(10).unwrap())
                    .unwrap()
            ),
            Some(changed.as_slice())
        );

        apply(
            &mut project,
            Operation::SetChildren {
                parent: ObjectTarget::id(10),
                children: placements(&[changed[0]]),
            },
        );
        assert!(project.undo_operation());
        assert_eq!(
            crate::operations::operation::object_refs(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(10).unwrap())
                    .unwrap()
            ),
            Some(changed.as_slice())
        );

        let mut cyclic_child = object(ObjectType::Container, 20);
        if let Object::Container(container) = &mut cyclic_child {
            container.object_refs.push(ObjectRef {
                id: ObjectId::new(10).unwrap(),
                offset: Default::default(),
            });
        }
        let mut cycle_pool = project.get_pool().clone();
        cycle_pool.add(cyclic_child);
        let mut cycle_project = EditorProject::from(cycle_pool);
        let mut transaction = OperationTransaction::new(None);
        transaction.add_operation(Operation::SetChildren {
            parent: ObjectTarget::id(10),
            children: vec![ChildPlacement {
                child: ObjectTarget::id(20),
                x: 0,
                y: 0,
            }],
        });
        assert!(cycle_project.execute_transaction(transaction).is_err());
    }

    #[test]
    fn positioned_child_edit_updates_the_exact_repeated_occurrence() {
        let parent = object(ObjectType::Button, 10);
        let child = object(ObjectType::PictureGraphic, 11);
        let mut pool = ObjectPool::default();
        pool.add(parent);
        pool.add(child);
        let mut project = EditorProject::from(pool);
        let child_id = ObjectId::new(11).unwrap();
        let initial = vec![
            ObjectRef {
                id: child_id,
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 12, y: 7 },
            },
            ObjectRef {
                id: child_id,
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 30, y: 42 },
            },
        ];
        apply(
            &mut project,
            Operation::SetChildren {
                parent: ObjectTarget::id(10),
                children: placements(&initial),
            },
        );

        let edited = positioned_children_with_position(&initial, 1, 80, 80);
        apply(
            &mut project,
            Operation::SetChildren {
                parent: ObjectTarget::id(10),
                children: edited,
            },
        );
        let refs = crate::operations::operation::object_refs(
            project
                .get_pool()
                .object_by_id(ObjectId::new(10).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(refs[0].offset.x, 12);
        assert_eq!(refs[0].offset.y, 7);
        assert_eq!(refs[1].offset.x, 80);
        assert_eq!(refs[1].offset.y, 80);
    }

    #[test]
    fn existing_signed_offsets_can_be_repaired_without_worsening_overflow() {
        let parent = object(ObjectType::Button, 10);
        let child = object(ObjectType::PictureGraphic, 11);
        let mut pool = ObjectPool::default();
        pool.add(parent);
        pool.add(child);
        let child_id = ObjectId::new(11).unwrap();
        let legacy = vec![
            ObjectRef {
                id: child_id,
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: -12, y: -7 },
            },
            ObjectRef {
                id: child_id,
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 30, y: 42 },
            },
        ];
        if let Some(Object::Button(button)) = pool.object_mut_by_id(ObjectId::new(10).unwrap()) {
            button.object_refs = legacy.clone();
        }
        let mut project = EditorProject::from(pool);

        let repaired = positioned_children_with_position(&legacy, 0, 0, 0);
        apply(
            &mut project,
            Operation::SetChildren {
                parent: ObjectTarget::id(10),
                children: repaired,
            },
        );
        let refs = crate::operations::operation::object_refs(
            project
                .get_pool()
                .object_by_id(ObjectId::new(10).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!((refs[0].offset.x, refs[0].offset.y), (0, 0));
        assert_eq!((refs[1].offset.x, refs[1].offset.y), (30, 42));
    }

    #[test]
    fn required_and_nullable_object_lists_are_reversible() {
        let required = object(ObjectType::SoftKeyMask, 10);
        let nullable = object(ObjectType::InputList, 20);
        let first = object(ObjectType::Key, 11);
        let second = object(ObjectType::Key, 12);
        let input_item = object(ObjectType::OutputString, 13);
        let mut pool = ObjectPool::default();
        for object in [required, nullable, first, second, input_item] {
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        let first_id = ObjectId::new(11).unwrap();
        let second_id = ObjectId::new(12).unwrap();
        apply(
            &mut project,
            Operation::SetObjectList {
                object: ObjectTarget::id(10),
                objects: ObjectTargetList::Required(vec![
                    ObjectTarget::id(first_id.value()),
                    ObjectTarget::id(second_id.value()),
                ]),
            },
        );
        apply(
            &mut project,
            Operation::SetObjectList {
                object: ObjectTarget::id(10),
                objects: ObjectTargetList::Required(vec![ObjectTarget::id(second_id.value())]),
            },
        );
        assert!(project.undo_operation());
        assert_eq!(
            crate::operations::operation::object_list(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(10).unwrap())
                    .unwrap()
            ),
            Some(ObjectReferenceList::Required(vec![first_id, second_id]))
        );
        assert!(project.redo_operation());

        apply(
            &mut project,
            Operation::SetObjectList {
                object: ObjectTarget::id(20),
                objects: ObjectTargetList::Nullable(vec![Some(ObjectTarget::id(13)), None]),
            },
        );
        assert!(project.undo_operation());
        assert_eq!(
            crate::operations::operation::object_list(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(20).unwrap())
                    .unwrap()
            ),
            Some(ObjectReferenceList::Nullable(vec![]))
        );
        assert!(project.redo_operation());
    }

    #[test]
    fn macro_reference_add_reorder_and_remove_are_reversible() {
        let owner = object(ObjectType::Button, 10);
        let first = object(ObjectType::Macro, 20);
        let second = object(ObjectType::Macro, 21);
        let mut pool = ObjectPool::default();
        for object in [owner, first, second] {
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        let refs = vec![
            MacroRef {
                event_id: Event::OnEnable,
                macro_id: 20,
            },
            MacroRef {
                event_id: Event::OnDisable,
                macro_id: 21,
            },
        ];
        apply(
            &mut project,
            Operation::SetMacroReferences {
                object: ObjectTarget::id(10),
                macro_refs: macro_targets(&refs),
            },
        );
        let reordered = vec![refs[1].clone(), refs[0].clone()];
        apply(
            &mut project,
            Operation::SetMacroReferences {
                object: ObjectTarget::id(10),
                macro_refs: macro_targets(&reordered),
            },
        );
        assert!(project.undo_operation());
        assert_eq!(
            crate::operations::operation::macro_refs(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(10).unwrap())
                    .unwrap()
            ),
            Some(refs.as_slice())
        );
        assert!(project.redo_operation());
        apply(
            &mut project,
            Operation::SetMacroReferences {
                object: ObjectTarget::id(10),
                macro_refs: macro_targets(&[reordered[0].clone()]),
            },
        );
        assert!(project.undo_operation());
        assert_eq!(
            crate::operations::operation::macro_refs(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(10).unwrap())
                    .unwrap()
            ),
            Some(reordered.as_slice())
        );
    }
}
