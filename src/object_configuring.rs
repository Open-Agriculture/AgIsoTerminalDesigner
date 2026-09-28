//! Descriptor-driven object configuration.
//!
//! This module is deliberately generic: object types expose properties through
//! `PropertyAccess`, and this renderer is the sole configuration UI.

use crate::object_properties::{
    get_object_property, get_property_descriptors, property_editor_descriptors, EditorKind,
    PropertySemantic,
};
use crate::operations::{
    ChildPlacement, MacroReferenceTarget, ObjectReferenceList, ObjectTarget, ObjectTargetList,
};
use crate::{EditorProject, Operation};
use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef};
use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::{NullableObjectId, ObjectId, ObjectRef, ObjectType};
use eframe::egui;

#[derive(Clone)]
enum PendingValue {
    Number(f64),
    Text(String),
}

/// A drag value changes on every pointer-move frame. Keep displaying those
/// intermediate values, but only turn the edit into an operation when the
/// pointer is released (or a keyboard edit loses focus).
fn drag_value_finished(response: &egui::Response) -> bool {
    response.drag_stopped() || response.lost_focus()
}

/// Render every writable property declared for `object`.
pub fn render_property_editor(ui: &mut egui::Ui, object: &Object, design: &EditorProject) {
    ui.style_mut().spacing.combo_width = 0.0;
    let id = object.id();
    let object_type = object.object_type();
    let properties = get_property_descriptors(object_type);

    render_identity(ui, id, object_type, design);
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
                    for property in ["format", "options", "data"] {
                        queue(design, id, property, value[property].clone());
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
                "options",
                serde_json::to_value(options).unwrap(),
            );
            queue(design, id, "transparency_colour", serde_json::json!(colour));
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
        if object_type == ObjectType::PictureGraphic
            && matches!(
                editor.property,
                "format" | "options" | "transparency_colour"
            )
        {
            continue;
        }

        if object_type == ObjectType::PictureGraphic
            && matches!(editor.property, "data" | "actual_width" | "actual_height")
        {
            continue;
        }
        // Structural collections are rendered below and never use SetProperty.
        if matches!(editor.editor, EditorKind::MacroReferences) {
            continue;
        }
        let Some(property) = properties.iter().find(|item| item.name == editor.property) else {
            continue;
        };
        let Ok(value) = get_object_property(object, editor.property) else {
            continue;
        };
        let label = human_readable_property_name(editor.property);
        ui.push_id(editor.property, |ui| match editor.editor {
            EditorKind::Colour => render_colour(ui, &label, &value, id, editor.property, design),
            EditorKind::ObjectReference => {
                render_reference(ui, &label, &value, property, id, editor.property, design)
            }
            EditorKind::Enum => {
                ui.horizontal(|ui| {
                    let label_response = ui.label(&label);
                    egui::ComboBox::from_id_salt(editor.property)
                        .selected_text(enum_choice_label(&value))
                        .show_ui(ui, |ui| {
                            for choice in &property.enum_choices {
                                if ui
                                    .selectable_label(choice == &value, enum_choice_label(choice))
                                    .clicked()
                                    && choice != &value
                                {
                                    queue(design, id, editor.property, choice.clone());
                                }
                            }
                        })
                        .response
                        .labelled_by(label_response.id);
                });
            }
            EditorKind::FlagSet => render_flag_set(ui, &label, &value, id, editor.property, design),
            EditorKind::Justification => {
                render_justification(ui, &label, &value, id, editor.property, design)
            }
            EditorKind::Auto => render_auto(ui, &label, &value, id, editor.property, design),
            EditorKind::MacroReferences => unreachable!("macro references are rendered separately"),
        });
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
    property: &str,
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
    format!(
        "{}: {:?} - {}",
        object.id().value(),
        object.object_type(),
        design.get_object_info(object).get_name(object)
    )
}

fn object_selector_label(design: &EditorProject, object: &Object) -> String {
    let info = design.get_object_info(object);
    info.name.map_or_else(
        || object.id().value().to_string(),
        |name| format!("{} - {name}", object.id().value()),
    )
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

fn parent_dimensions(object: &Object, design: &EditorProject) -> (u16, u16) {
    object
        .as_sized_object()
        .map(|sized| (sized.width(), sized.height()))
        .unwrap_or((design.mask_size, design.mask_size))
}

fn child_position_limits(
    parent: &Object,
    child: Option<&Object>,
    design: &EditorProject,
) -> (i16, i16) {
    let (width, height) = parent_dimensions(parent, design);
    let (child_width, child_height) = child
        .and_then(Object::as_sized_object)
        .map(|sized| (sized.width(), sized.height()))
        .unwrap_or_default();
    (
        width.saturating_sub(child_width).min(i16::MAX as u16) as i16,
        height.saturating_sub(child_height).min(i16::MAX as u16) as i16,
    )
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
        VtVersion::Version6,
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
                    ui.colored_label(egui::Color32::RED, "Missing object");
                }
                let (max_x, max_y) = child_position_limits(parent, child, design);
                let mut x = child_ref.offset.x.clamp(0, max_x);
                let mut y = child_ref.offset.y.clamp(0, max_y);
                let x_changed = ui
                    .add(
                        egui::Slider::new(&mut x, 0..=max_x)
                            .show_value(true)
                            .drag_value_speed(1.0),
                    )
                    .changed();
                let y_changed = ui
                    .add(
                        egui::Slider::new(&mut y, 0..=max_y)
                            .show_value(true)
                            .drag_value_speed(1.0),
                    )
                    .changed();
                if x_changed || y_changed {
                    position = Some((child_ref.id, x, y));
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
    } else if let Some((child_id, x, y)) = position {
        design.queue_operation(Operation::SetChildPosition {
            parent_id: parent_id.value(),
            child_id: child_id.value(),
            x,
            y,
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
        VtVersion::Version6,
    );
    match objects {
        ObjectReferenceList::Required(objects) => {
            let mut replacement = None;
            let mut action = None;
            egui::Grid::new(("required_object_list", parent_id.value()))
                .striped(true)
                .show(ui, |ui| {
                    for (index, object_id) in objects.iter().copied().enumerate() {
                        ui.label(object_id.value().to_string());
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
                        ui.label(
                            object_id
                                .0
                                .map_or_else(|| "None".to_owned(), |id| id.value().to_string()),
                        );
                        let mut selected = object_id;
                        let label = object_id.0.map_or_else(
                            || "None".to_owned(),
                            |id| {
                                design.get_pool().object_by_id(id).map_or_else(
                                    || format!("#{}: Missing object", id.value()),
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
        if ui
            .link(design.get_object_info(object).get_name(object))
            .clicked()
        {
            select_object(design, object_id);
        }
    } else {
        ui.colored_label(egui::Color32::RED, "Missing object");
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

fn macro_object(design: &EditorProject, macro_id: u8) -> Option<&Object> {
    ObjectId::new(macro_id.into())
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
                macro_object: ObjectTarget::id(reference.macro_id.into()),
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
        if let Some(event_id) = selected_event {
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
                        && candidate.id().value() <= u8::MAX.into()
                },
                |ui, choice| {
                    crate::object_picker::impact::review_queued(ui, design, |candidate| {
                        if let Some(choice) = choice.0 {
                            let mut updated = references.to_vec();
                            updated.push(MacroRef {
                                event_id,
                                macro_id: choice.value() as u8,
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
                    macro_id: candidate.value() as u8,
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
    let events = crate::possible_events::get_possible_events(object);
    let mut action = None;
    egui::Grid::new(("macro_grid", id.value()))
        .striped(true)
        .min_col_width(0.0)
        .show(ui, |ui| {
            for (index, reference) in references.iter().enumerate() {
                let mut edited = reference.clone();
                egui::ComboBox::from_id_salt(("macro_event", index))
                    .selected_text(format!("{:?}", reference.event_id))
                    .show_ui(ui, |ui| {
                        for event in &events {
                            ui.selectable_value(&mut edited.event_id, *event, format!("{event:?}"));
                        }
                    });

                let macro_object = macro_object(design, reference.macro_id);
                let selected_text = macro_object.map_or_else(
                    || format!("{} - Missing macro", reference.macro_id),
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
                            && candidate.id().value() <= u8::MAX.into()
                    },
                    |ui, choice| {
                        crate::object_picker::impact::review_queued(ui, design, |candidate| {
                            if let Some(choice) = choice.0 {
                                let mut updated = references.to_vec();
                                updated[index] = edited.clone();
                                updated[index].macro_id = choice.value() as u8;
                                queue_macro_references(candidate, id, updated);
                            }
                        })
                    },
                )
                .map(|id| id.0)
                {
                    edited.macro_id = candidate.value() as u8;
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
    let mut value = ui
        .data(|data| match data.get_temp::<PendingValue>(key) {
            Some(PendingValue::Number(value)) => Some(value as u16),
            _ => None,
        })
        .unwrap_or_else(|| id.value());
    ui.horizontal(|ui| {
        ui.label("Object ID");
        let response = ui.add(egui::DragValue::new(&mut value).range(0..=65534));
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(key, PendingValue::Number(value.into())));
        }

        let conflict = value != id.value()
            && ObjectId::new(value)
                .ok()
                .is_some_and(|new_id| design.get_pool().object_by_id(new_id).is_some());
        if conflict {
            ui.colored_label(egui::Color32::RED, "ID already in use");
        }

        if drag_value_finished(&response) {
            if value != id.value() {
                if let Ok(new_id) = ObjectId::new(value) {
                    if design.get_pool().object_by_id(new_id).is_none() {
                        design.queue_operation(Operation::ChangeObjectId {
                            old_id: id.value(),
                            new_id: value,
                        });
                        design
                            .get_mut_selected()
                            .set(NullableObjectId(Some(new_id)));
                    }
                }
            }
            ui.data_mut(|data| data.remove::<PendingValue>(key));
        }
        ui.label(format!("Type: {object_type:?}"));
    });
}

fn queue(design: &EditorProject, id: ObjectId, property: &str, value: serde_json::Value) {
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
        design.queue_operation(Operation::SetObjectReference {
            object: ObjectTarget::id(id.value()),
            property: property.to_owned(),
            reference,
        });
    } else {
        design.queue_operation(Operation::SetProperty {
            object: ObjectTarget::id(id.value()),
            property: property.to_owned(),
            value,
        });
    }
}

/// Preserve the JSON number kind exposed by the property's Rust type. In
/// particular, integer properties must be committed as `42`, not `42.0`:
/// serde_json deliberately does not expose the latter through `as_u64()`.
fn number_after_edit(original: &serde_json::Value, edited: f64) -> Option<serde_json::Number> {
    let serde_json::Value::Number(original) = original else {
        return None;
    };

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
    property: &str,
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
            0.0..=u64::MAX as f64,
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
    property: &str,
    design: &EditorProject,
) {
    let key = ui.make_persistent_id((id.value(), property));
    let mut edited = pending_text(ui, key).unwrap_or_else(|| current.to_owned());
    ui.horizontal(|ui| {
        ui.label(label);
        let response = ui.text_edit_singleline(&mut edited);
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(key, PendingValue::Text(edited.clone())));
        }
        if response.lost_focus() && edited != current {
            queue(design, id, property, edited.into());
            ui.data_mut(|data| data.remove::<PendingValue>(key));
        }
    });
}

fn render_number(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    range: std::ops::RangeInclusive<f64>,
    id: ObjectId,
    property: &str,
    design: &EditorProject,
) {
    let Some(current) = value.as_f64() else {
        render_json(ui, label, value, id, property, design);
        return;
    };
    let key = ui.make_persistent_id((id.value(), property));
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
        }
        if drag_value_finished(&response) {
            if edited != current {
                if let Some(number) = number_after_edit(value, edited) {
                    queue(design, id, property, number.into());
                }
            }
            ui.data_mut(|data| data.remove::<PendingValue>(key));
        }
    });
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
) -> Option<serde_json::Value> {
    let label = human_readable_property_name(field);
    if let Some(mut enabled) = value.as_bool() {
        return ui
            .checkbox(&mut enabled, label)
            .changed()
            .then_some(enabled.into());
    }
    let choices = flag_choices(field);
    let mut updated = None;
    let (selected_label, selected_help) = flag_choice_presentation(field, value);
    let response = ui
        .horizontal(|ui| {
            let label_response = ui.label(&label);
            egui::ComboBox::from_id_salt(&label)
                .selected_text(selected_label)
                .show_ui(ui, |ui| {
                    for choice in choices {
                        let (label, help) = flag_choice_presentation(field, &choice);
                        let mut response = ui.selectable_label(value == &choice, label);
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
    property: &str,
    design: &EditorProject,
) {
    if let Some(current) = value.as_u64() {
        let bits = if property == "line_art" { 16 } else { 8 };
        ui.label(label);
        ui.horizontal_wrapped(|ui| {
            for bit in (0..bits).rev() {
                let mask = 1_u64 << bit;
                let mut enabled = current & mask != 0;
                if ui
                    .checkbox(&mut enabled, format!("{bit}"))
                    .on_hover_text(format!("Bit {bit}"))
                    .changed()
                {
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
            let edited = ui
                .push_id(field, |ui| render_flag_field(ui, field, value))
                .inner;
            if let Some(edited) = edited {
                let mut updated = fields.clone();
                updated.insert(field.clone(), edited);
                queue(design, id, property, serde_json::Value::Object(updated));
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
    property: &str,
    design: &EditorProject,
) {
    let PropertySemantic::ObjectReference { allowed_types } = &descriptor.semantic else {
        return;
    };
    let selected = value.as_u64();
    ui.horizontal(|ui| {
        ui.label(label);
        let selected_id = selected.and_then(|id| ObjectId::new(id as u16).ok());
        let label = selected_id
            .and_then(|id| design.get_pool().object_by_id(id))
            .map(|object| object_selector_label(design, object))
            .unwrap_or_else(|| "None".into());
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
                    && crate::pool_validation::would_create_circular_reference(
                        design.get_pool(),
                        id,
                        candidate.id(),
                    )
                    .is_ok()
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
    });
}

fn render_justification(
    ui: &mut egui::Ui,
    label: &str,
    value: &serde_json::Value,
    id: ObjectId,
    property: &str,
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
    ui.horizontal(|ui| {
        let horizontal_label = ui.label(label);
        egui::ComboBox::from_id_salt(label)
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
        let vertical_label = ui.label("Vertical");
        egui::ComboBox::from_id_salt("Vertical")
            .selected_text(vertical)
            .show_ui(ui, |ui| {
                for choice in ["Top", "Middle", "Bottom"] {
                    if ui.selectable_label(vertical == choice, choice).clicked() {
                        let mut updated = fields.clone();
                        updated.insert("vertical".to_owned(), choice.into());
                        queue(design, id, property, serde_json::Value::Object(updated));
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
    property: &str,
    design: &EditorProject,
) {
    let key = ui.make_persistent_id((id.value(), property));
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
    }
    if response.lost_focus() && edited != original {
        if let Ok(parsed) = serde_json::from_str(&edited) {
            queue(design, id, property, parsed);
            ui.data_mut(|data| data.remove::<PendingValue>(key));
        } else {
            ui.colored_label(egui::Color32::RED, "Invalid JSON");
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

    fn object(object_type: ObjectType, id: u16) -> Object {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        object
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
                macro_object: ObjectTarget::id(reference.macro_id.into()),
            })
            .collect()
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
                let value = get_object_property(&object, editor.property).unwrap();
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
                        editor.property
                    );
                    for choice in choices {
                        let mut updated = fields.clone();
                        updated.insert(field.clone(), choice);
                        crate::object_properties::validate_property(
                            &pool,
                            id,
                            editor.property,
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
        let integer = number_after_edit(&serde_json::json!(0_u8), 42.0).unwrap();
        assert_eq!(integer.as_u64(), Some(42));
        assert!(!integer.is_f64());

        let mut pool = ObjectPool::default();
        let button = object(ObjectType::Button, 1002);
        let button_id = button.id();
        pool.add(button);
        assert!(crate::object_properties::validate_property(
            &pool,
            button_id,
            "background_colour",
            &serde_json::Value::Number(integer),
        )
        .is_ok());

        let float = number_after_edit(&serde_json::json!(0.0_f64), 42.0).unwrap();
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
