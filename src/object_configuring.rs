//! Descriptor-driven object configuration.
//!
//! This module is deliberately generic: object types expose properties through
//! `PropertyAccess`, and this renderer is the sole configuration UI.

use crate::object_properties::{
    get_object_property, get_property_descriptors, property_editor_descriptors, EditorKind,
    PropertySemantic,
};
use crate::operations::ObjectReferenceList;
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
    for editor in property_editor_descriptors(object_type) {
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
            EditorKind::Colour => {
                render_number(ui, &label, &value, 0.0..=255.0, id, editor.property, design)
            }
            EditorKind::ObjectReference => {
                render_reference(ui, &label, &value, property, id, editor.property, design)
            }
            EditorKind::Json => render_json(ui, &label, &value, id, editor.property, design),
            EditorKind::FlagSet => render_flag_set(ui, &label, &value, id, editor.property, design),
            EditorKind::Justification => {
                render_justification(ui, &label, &value, id, editor.property, design)
            }
            EditorKind::Auto => render_auto(ui, &label, &value, id, editor.property, design),
            _ => render_auto(ui, &label, &value, id, editor.property, design),
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
    allowed_types: &[ObjectType],
) -> Option<ObjectId> {
    let selected_text = design.get_pool().object_by_id(selected).map_or_else(
        || format!("{}: Missing object", selected.value()),
        |object| object_selector_label(design, object),
    );
    let mut replacement = None;
    egui::ComboBox::from_id_salt(salt)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for candidate in design.get_pool().objects_by_types(allowed_types) {
                let valid = candidate_is_valid(design, parent, candidate.id());
                ui.add_enabled_ui(valid, |ui| {
                    if ui
                        .selectable_label(
                            candidate.id() == selected,
                            object_selector_label(design, candidate),
                        )
                        .clicked()
                    {
                        replacement = Some(candidate.id());
                    }
                });
            }
        });
    replacement.filter(|replacement| *replacement != selected)
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
        let mut updated = children.to_vec();
        updated[index].id = child_id;
        design.queue_operation(Operation::SetChildren {
            parent_id: parent_id.value(),
            children: updated,
        });
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
            parent_id: parent_id.value(),
            children: updated,
        });
    }

    ui.horizontal(|ui| {
        ui.label("Add object:");
        egui::ComboBox::from_id_salt(("add_positioned_child", parent_id.value()))
            .selected_text("Select existing object")
            .show_ui(ui, |ui| {
                for candidate in design.get_pool().objects_by_types(&allowed) {
                    let valid = candidate_is_valid(design, parent_id, candidate.id());
                    ui.add_enabled_ui(valid, |ui| {
                        if ui
                            .selectable_label(false, object_label(design, candidate))
                            .clicked()
                        {
                            design.queue_operation(Operation::AddChild {
                                parent_id: parent_id.value(),
                                child_id: candidate.id().value(),
                                x: 0,
                                y: 0,
                            });
                        }
                    });
                }
            });
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
                let mut updated = objects.clone();
                updated[index] = object_id;
                queue_object_list(design, parent_id, ObjectReferenceList::Required(updated));
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
                        egui::ComboBox::from_id_salt(("nullable_object", index))
                            .selected_text(object_id.0.map_or_else(
                                || "None".to_owned(),
                                |id| {
                                    design.get_pool().object_by_id(id).map_or_else(
                                        || format!("{}: Missing object", id.value()),
                                        |object| object_label(design, object),
                                    )
                                },
                            ))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut selected, NullableObjectId::NULL, "None");
                                for candidate in design.get_pool().objects_by_types(&allowed) {
                                    let valid =
                                        candidate_is_valid(design, parent_id, candidate.id());
                                    ui.add_enabled_ui(valid, |ui| {
                                        ui.selectable_value(
                                            &mut selected,
                                            NullableObjectId(Some(candidate.id())),
                                            object_label(design, candidate),
                                        );
                                    });
                                }
                            });
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
                let mut updated = objects.clone();
                updated[index] = object_id;
                queue_object_list(design, parent_id, ObjectReferenceList::Nullable(updated));
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
        egui::ComboBox::from_id_salt(("add_object_list_item", parent_id.value()))
            .selected_text("Select existing object")
            .show_ui(ui, |ui| {
                if allow_none && ui.selectable_label(false, "None").clicked() {
                    add(None);
                }
                for candidate in design.get_pool().objects_by_types(allowed) {
                    let valid = candidate_is_valid(design, parent_id, candidate.id());
                    ui.add_enabled_ui(valid, |ui| {
                        if ui
                            .selectable_label(false, object_label(design, candidate))
                            .clicked()
                        {
                            add(Some(candidate.id()));
                        }
                    });
                }
            });
    });
}

fn queue_object_list(design: &EditorProject, id: ObjectId, objects: ObjectReferenceList) {
    design.queue_operation(Operation::SetObjectList {
        object_id: id.value(),
        objects,
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
        object_id: id.value(),
        macro_refs,
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
            egui::ComboBox::from_id_salt(("new_macro_object", id.value()))
                .selected_text("Select macro")
                .show_ui(ui, |ui| {
                    for candidate in design.get_pool().objects_by_type(ObjectType::Macro) {
                        if candidate.id().value() <= u8::MAX.into()
                            && ui
                                .selectable_label(false, object_label(design, candidate))
                                .clicked()
                        {
                            let mut updated = references.to_vec();
                            updated.push(MacroRef {
                                event_id,
                                macro_id: candidate.id().value() as u8,
                            });
                            queue_macro_references(design, id, updated);
                            ui.data_mut(|data| data.remove::<Event>(event_key));
                        }
                    }
                });
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
                egui::ComboBox::from_id_salt(("macro_object", index))
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for candidate in design.get_pool().objects_by_type(ObjectType::Macro) {
                            if candidate.id().value() <= u8::MAX.into() {
                                ui.selectable_value(
                                    &mut edited.macro_id,
                                    candidate.id().value() as u8,
                                    object_label(design, candidate),
                                );
                            }
                        }
                    });
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
    design.queue_operation(Operation::SetProperty {
        object_id: id.value(),
        property: property.to_owned(),
        value,
    });
}

/// Preserve the JSON number kind exposed by the property's Rust type. In
/// particular, integer properties must be committed as `42`, not `42.0`:
/// serde_json deliberately does not expose the latter through `as_u64()`.
fn number_after_edit(
    original: &serde_json::Value,
    edited: f64,
) -> Option<serde_json::Number> {
    let serde_json::Value::Number(original) = original else {
        return None;
    };

    if original.is_u64() {
        if edited.is_finite()
            && edited.fract() == 0.0
            && (0.0..=u64::MAX as f64).contains(&edited)
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

/// Render bit-mask and object-style options as individual checkboxes rather
/// than leaking their JSON encoding into the UI.
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
            let Some(mut enabled) = value.as_bool() else {
                render_json(ui, label, value, id, property, design);
                return;
            };
            if ui.checkbox(&mut enabled, field).changed() {
                let mut updated = fields.clone();
                updated.insert(field.clone(), enabled.into());
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
    egui::ComboBox::from_label(label)
        .selected_text(selected.map_or_else(|| "None".to_owned(), |value| value.to_string()))
        .show_ui(ui, |ui| {
            if selected.is_some() && ui.selectable_label(false, "None").clicked() {
                queue(design, id, property, serde_json::Value::Null);
            }
            for candidate in design.get_pool().objects_by_types(allowed_types) {
                if ui
                    .selectable_label(
                        selected == Some(u64::from(candidate.id().value())),
                        format!("{}: {:?}", candidate.id().value(), candidate.object_type()),
                    )
                    .clicked()
                {
                    queue(
                        design,
                        id,
                        property,
                        serde_json::Value::from(candidate.id().value()),
                    );
                }
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
        egui::ComboBox::from_label(label)
            .selected_text(horizontal)
            .show_ui(ui, |ui| {
                for choice in ["Left", "Middle", "Right"] {
                    if ui.selectable_label(horizontal == choice, choice).clicked() {
                        let mut updated = fields.clone();
                        updated.insert("horizontal".to_owned(), choice.into());
                        queue(design, id, property, serde_json::Value::Object(updated));
                    }
                }
            });
        egui::ComboBox::from_label("Vertical")
            .selected_text(vertical)
            .show_ui(ui, |ui| {
                for choice in ["Top", "Middle", "Bottom"] {
                    if ui.selectable_label(vertical == choice, choice).clicked() {
                        let mut updated = fields.clone();
                        updated.insert("vertical".to_owned(), choice.into());
                        queue(design, id, property, serde_json::Value::Object(updated));
                    }
                }
            });
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
                parent_id: 10,
                children: initial.clone(),
            },
        );
        let changed = vec![initial[1], initial[0]];
        apply(
            &mut project,
            Operation::SetChildren {
                parent_id: 10,
                children: changed.clone(),
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
                parent_id: 10,
                children: vec![changed[0]],
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
            parent_id: 10,
            children: vec![ObjectRef {
                id: ObjectId::new(20).unwrap(),
                offset: Default::default(),
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
        let mut pool = ObjectPool::default();
        for object in [required, nullable, first, second] {
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        let first_id = ObjectId::new(11).unwrap();
        let second_id = ObjectId::new(12).unwrap();
        apply(
            &mut project,
            Operation::SetObjectList {
                object_id: 10,
                objects: ObjectReferenceList::Required(vec![first_id, second_id]),
            },
        );
        apply(
            &mut project,
            Operation::SetObjectList {
                object_id: 10,
                objects: ObjectReferenceList::Required(vec![second_id]),
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
                object_id: 20,
                objects: ObjectReferenceList::Nullable(vec![
                    NullableObjectId(Some(first_id)),
                    NullableObjectId::NULL,
                ]),
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
                object_id: 10,
                macro_refs: refs.clone(),
            },
        );
        let reordered = vec![refs[1].clone(), refs[0].clone()];
        apply(
            &mut project,
            Operation::SetMacroReferences {
                object_id: 10,
                macro_refs: reordered.clone(),
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
                object_id: 10,
                macro_refs: vec![reordered[0].clone()],
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
