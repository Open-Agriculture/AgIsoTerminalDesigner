//! Local application of Macro commands that modify pool objects.

use ag_iso_stack::object_pool::{
    object::Object,
    object_attributes::{Event, FontSize, FontStyle, FontType, LineDirection},
    vt_version::VtVersion,
    ObjectId, ObjectPool, ObjectType,
};
use serde_json::Value;

/// Apply one decoded visual command to temporary preview state.
///
/// The pool is cloned first so a bad operand or target leaves the caller's
/// state untouched. Returned object/event pairs are queued by the preview
/// session after this command finishes.
pub(crate) fn apply(
    pool: &mut ObjectPool,
    instruction: &crate::macro_authoring::MacroInstruction,
    version: VtVersion,
) -> Result<Vec<(u16, Event)>, String> {
    let mut staged = pool.clone();
    let events = apply_staged(&mut staged, instruction, version)?;
    *pool = staged;
    Ok(events)
}

fn apply_staged(
    pool: &mut ObjectPool,
    instruction: &crate::macro_authoring::MacroInstruction,
    version: VtVersion,
) -> Result<Vec<(u16, Event)>, String> {
    let args = &instruction.arguments;
    let name = instruction.name.as_str();
    match name {
        "change-attribute" => change_attribute(pool, args, version),
        "change-font-attributes" => change_font_attributes(pool, args),
        "change-line-attributes" => change_line_attributes(pool, args),
        "change-fill-attributes" => change_fill_attributes(pool, args),
        "change-end-point" => change_end_point(pool, args),
        "change-priority" => change_priority(pool, args),
        "change-list-item" => change_list_item(pool, args),
        "change-polygon-point" => change_polygon_point(pool, args),
        "change-polygon-scale" => change_polygon_scale(pool, args),
        "change-object-label" => change_object_label(pool, args),
        "select-colour-map-or-palette" => select_colour_map_or_palette(pool, args, version),
        _ => Err(format!(
            "Macro command '{name}' is not handled by the local object command applier"
        )),
    }
}

fn change_font_attributes(
    pool: &mut ObjectPool,
    args: &[String],
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 5, "change-font-attributes")?;
    let id = object_id(&args[0])?;
    require_type(pool, id, ObjectType::FontAttributes)?;
    let font_colour = number(&args[1], "font colour")?;
    let font_size = FontSize::from(number(&args[2], "font size")?);
    let font_type = FontType::from(number(&args[3], "font type")?);
    let font_style = FontStyle::from(number(&args[4], "font style")?);
    set(pool, id, "font_colour", Value::from(font_colour))?;
    set(
        pool,
        id,
        "font_size",
        serde_json::to_value(font_size).map_err(err)?,
    )?;
    set(
        pool,
        id,
        "font_type",
        serde_json::to_value(font_type).map_err(err)?,
    )?;
    set(
        pool,
        id,
        "font_style",
        serde_json::to_value(font_style).map_err(err)?,
    )?;
    Ok(vec![(id.value(), Event::OnChangeFontAttributes)])
}

fn change_line_attributes(
    pool: &mut ObjectPool,
    args: &[String],
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 4, "change-line-attributes")?;
    let id = object_id(&args[0])?;
    require_type(pool, id, ObjectType::LineAttributes)?;
    set(
        pool,
        id,
        "line_colour",
        Value::from(number(&args[1], "line colour")?),
    )?;
    set(
        pool,
        id,
        "line_width",
        Value::from(number(&args[2], "line width")?),
    )?;
    set(
        pool,
        id,
        "line_art",
        Value::from(u16::try_from(integer(&args[3], "line art")?).map_err(err)?),
    )?;
    Ok(vec![(id.value(), Event::OnChangeLineAttributes)])
}

fn change_fill_attributes(
    pool: &mut ObjectPool,
    args: &[String],
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 4, "change-fill-attributes")?;
    let id = object_id(&args[0])?;
    require_type(pool, id, ObjectType::FillAttributes)?;
    set(
        pool,
        id,
        "fill_type",
        Value::from(number(&args[1], "fill type")?),
    )?;
    set(
        pool,
        id,
        "fill_colour",
        Value::from(number(&args[2], "fill colour")?),
    )?;
    set_nullable_reference(pool, id, "fill_pattern", &args[3])?;
    Ok(vec![(id.value(), Event::OnChangeFillAttributes)])
}

fn change_end_point(pool: &mut ObjectPool, args: &[String]) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 4, "change-end-point")?;
    let id = object_id(&args[0])?;
    require_type(pool, id, ObjectType::OutputLine)?;
    let width = integer(&args[1], "width")?;
    let height = integer(&args[2], "height")?;
    let direction = match number(&args[3], "line direction")? {
        0 => LineDirection::TopLeftToBottomRight,
        1 => LineDirection::BottomLeftToTopRight,
        other => return Err(format!("Invalid line direction {other}")),
    };
    set(pool, id, "width", Value::from(width))?;
    set(pool, id, "height", Value::from(height))?;
    set(
        pool,
        id,
        "line_direction",
        serde_json::to_value(direction).map_err(err)?,
    )?;
    Ok(vec![(id.value(), Event::OnChangeEndPoint)])
}

fn change_priority(pool: &mut ObjectPool, args: &[String]) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 2, "change-priority")?;
    let id = object_id(&args[0])?;
    set(
        pool,
        id,
        "priority",
        Value::from(number(&args[1], "priority")?),
    )?;
    Ok(vec![(id.value(), Event::OnChangePriority)])
}

fn change_list_item(pool: &mut ObjectPool, args: &[String]) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 3, "change-list-item")?;
    let id = object_id(&args[0])?;
    let index = usize::from(number(&args[1], "list index")?);
    let value = nullable_object_id(&args[2])?;
    let object = pool.object_mut_by_id(id).ok_or_else(|| missing(id))?;
    match object {
        Object::InputList(list) => {
            replace_nullable_list_item(&mut list.list_items, index, value, id)?
        }
        Object::OutputList(list) => {
            replace_nullable_list_item(&mut list.list_items, index, value, id)?
        }
        Object::Animation(animation) => {
            let item = animation
                .object_refs
                .get_mut(index)
                .ok_or_else(|| index_error(id, index))?;
            item.id =
                value.ok_or_else(|| "Animation object references cannot be NULL".to_owned())?;
        }
        Object::ExternalObjectDefinition(definition) => {
            replace_nullable_list_item(&mut definition.objects, index, value, id)?
        }
        other => {
            return Err(format!(
                "Object {} is {:?}; Change List Item requires a list-bearing object",
                id.value(),
                other.object_type()
            ));
        }
    }
    let event = if matches!(
        pool.object_by_id(id),
        Some(Object::ExternalObjectDefinition(_))
    ) {
        Event::OnChangeAttribute
    } else {
        Event::OnChangeValue
    };
    Ok(vec![(id.value(), event)])
}

fn change_polygon_point(
    pool: &mut ObjectPool,
    args: &[String],
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 4, "change-polygon-point")?;
    let id = object_id(&args[0])?;
    let index = usize::from(number(&args[1], "point index")?);
    let x = small_integer(&args[2], "point X")?;
    let y = small_integer(&args[3], "point Y")?;
    let Some(Object::OutputPolygon(polygon)) = pool.object_mut_by_id(id) else {
        return Err(format!("Object {} is not an Output Polygon", id.value()));
    };
    let point = polygon
        .points
        .get_mut(index)
        .ok_or_else(|| index_error(id, index))?;
    point.x = x;
    point.y = y;
    Ok(Vec::new())
}

fn change_polygon_scale(
    pool: &mut ObjectPool,
    args: &[String],
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 3, "change-polygon-scale")?;
    let id = object_id(&args[0])?;
    let width = small_integer(&args[1], "width")?;
    let height = small_integer(&args[2], "height")?;
    let Some(Object::OutputPolygon(polygon)) = pool.object_mut_by_id(id) else {
        return Err(format!("Object {} is not an Output Polygon", id.value()));
    };
    let (old_width, old_height) = (polygon.width, polygon.height);
    if old_width == 0 || old_height == 0 {
        return Err("Cannot scale an Output Polygon with zero width or height".into());
    }
    for point in &mut polygon.points {
        point.x = ((u32::from(point.x) * u32::from(width) + u32::from(old_width) / 2)
            / u32::from(old_width)) as u16;
        point.y = ((u32::from(point.y) * u32::from(height) + u32::from(old_height) / 2)
            / u32::from(old_height)) as u16;
    }
    polygon.width = width;
    polygon.height = height;
    Ok(Vec::new())
}

fn select_colour_map_or_palette(
    pool: &mut ObjectPool,
    args: &[String],
    version: VtVersion,
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 1, "select-colour-map-or-palette")?;
    let selected = nullable_object_id(&args[0])?;
    let selected_type = selected.and_then(|id| {
        pool.object_by_id(id)
            .map(|object| (id, object.object_type()))
    });
    if let Some((id, object_type)) = selected_type {
        match object_type {
            ObjectType::ColourMap if version >= VtVersion::Version4 => {}
            ObjectType::ColourPalette if version >= VtVersion::Version6 => {}
            ObjectType::ColourMap => {
                return Err("Colour Map selection requires VT version 4 or later".into());
            }
            ObjectType::ColourPalette => {
                return Err("Colour Palette selection requires VT version 6 or later".into());
            }
            _ => {
                return Err(format!(
                    "Object {} is not a Colour Map or Colour Palette",
                    id.value()
                ));
            }
        }
    } else if let Some(id) = selected {
        return Err(missing(id));
    }

    let controls_id = pool
        .objects()
        .iter()
        .filter_map(|object| match object {
            Object::WorkingSetSpecialControls(controls) => Some(controls.id),
            _ => None,
        })
        .min_by_key(ObjectId::value);
    let controls_id = if let Some(id) = controls_id {
        id
    } else {
        let id = (0..=u16::MAX - 1)
            .find(|candidate| {
                ObjectId::new(*candidate)
                    .ok()
                    .is_some_and(|id| pool.object_by_id(id).is_none())
            })
            .ok_or_else(|| {
                "No free object ID is available for preview colour controls".to_owned()
            })?;
        let mut controls =
            crate::object_defaults::default_object(ObjectType::WorkingSetSpecialControls);
        let Object::WorkingSetSpecialControls(controls) = &mut controls else {
            unreachable!()
        };
        controls.id =
            ObjectId::new(id).map_err(|_| "Selected preview object ID is invalid".to_owned())?;
        pool.add(Object::WorkingSetSpecialControls(controls.clone()));
        controls.id
    };
    let Object::WorkingSetSpecialControls(controls) = pool.object_mut_by_id(controls_id).unwrap()
    else {
        unreachable!()
    };
    match selected_type.map(|(_, object_type)| object_type) {
        Some(ObjectType::ColourMap) => controls.id_of_colour_map = nullable_id(selected),
        Some(ObjectType::ColourPalette) => controls.id_of_colour_palette = nullable_id(selected),
        None => {
            controls.id_of_colour_map = ag_iso_stack::object_pool::NullableObjectId::NULL;
            controls.id_of_colour_palette = ag_iso_stack::object_pool::NullableObjectId::NULL;
        }
        _ => unreachable!(),
    }
    Ok(Vec::new())
}

fn change_object_label(
    pool: &mut ObjectPool,
    args: &[String],
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 4, "change-object-label")?;
    let object_id = object_id(&args[0])?;
    let string_id = nullable_object_id(&args[1])?;
    let font_type = number(&args[2], "font type")?;
    let graphic_id = nullable_object_id(&args[3])?;
    let mut found = false;
    for object in pool.objects_mut() {
        if let Object::ObjectLabelReferenceList(labels) = object {
            if let Some(label) = labels
                .object_labels
                .iter_mut()
                .find(|label| label.id == object_id)
            {
                label.string_variable_reference = nullable_id(string_id);
                label.font_type = font_type;
                label.graphic_representation = nullable_id(graphic_id);
                found = true;
            }
        }
    }
    if !found {
        return Err(format!(
            "Object {} has no entry in the Object Label Reference List",
            object_id.value()
        ));
    }
    Ok(Vec::new())
}

fn change_attribute(
    pool: &mut ObjectPool,
    args: &[String],
    version: VtVersion,
) -> Result<Vec<(u16, Event)>, String> {
    require_args(args, 3, "change-attribute")?;
    let id = object_id(&args[0])?;
    let aid = number(&args[1], "attribute ID")?;
    let raw = integer(&args[2], "attribute value")?;
    let object = pool.object_by_id(id).ok_or_else(|| missing(id))?;
    let key = attribute_key(object.object_type(), aid).ok_or_else(|| {
        format!(
            "Attribute ID {aid} is not handled for {:?}",
            object.object_type()
        )
    })?;
    let value = attribute_value(object.object_type(), key, aid, raw, version)?;
    set(pool, id, key.as_str(), value)?;
    Ok(vec![(id.value(), Event::OnChangeAttribute)])
}

pub(crate) fn attribute_key(
    object_type: ObjectType,
    aid: u8,
) -> Option<crate::object_properties::PropertyKey> {
    use crate::object_properties::PropertyKey as K;
    use ObjectType as T;
    let keys: &[K] = match object_type {
        T::DataMask => &[K::BackgroundColour, K::SoftKeyMask],
        T::AlarmMask => &[
            K::BackgroundColour,
            K::SoftKeyMask,
            K::Priority,
            K::AcousticSignal,
        ],
        T::SoftKeyMask => &[K::BackgroundColour],
        T::Key => &[K::BackgroundColour, K::KeyCode],
        T::Button => &[
            K::Width,
            K::Height,
            K::BackgroundColour,
            K::BorderColour,
            K::KeyCode,
            K::Options,
        ],
        T::InputBoolean => &[
            K::BackgroundColour,
            K::Width,
            K::ForegroundColour,
            K::VariableReference,
        ],
        T::InputString => &[
            K::Width,
            K::Height,
            K::BackgroundColour,
            K::FontAttributes,
            K::InputAttributes,
            K::Options,
            K::VariableReference,
            K::Value,
        ],
        T::InputNumber => &[
            K::Width,
            K::Height,
            K::BackgroundColour,
            K::FontAttributes,
            K::Options,
            K::VariableReference,
            K::Value,
            K::MinValue,
            K::MaxValue,
            K::Offset,
            K::Scale,
            K::NrOfDecimals,
            K::Format,
        ],
        T::InputList => &[K::Width, K::Height, K::VariableReference],
        T::OutputString => &[
            K::Width,
            K::Height,
            K::BackgroundColour,
            K::FontAttributes,
            K::Options,
            K::VariableReference,
            K::Value,
        ],
        T::OutputNumber => &[
            K::Width,
            K::Height,
            K::BackgroundColour,
            K::FontAttributes,
            K::Options,
            K::VariableReference,
            K::Value,
            K::Offset,
            K::Scale,
            K::NrOfDecimals,
            K::Format,
        ],
        T::OutputList => &[K::Width, K::Height, K::VariableReference],
        T::OutputLine => &[K::LineAttributes, K::Width, K::Height, K::LineDirection],
        T::OutputRectangle => &[
            K::LineAttributes,
            K::Width,
            K::Height,
            K::LineSuppression,
            K::FillAttributes,
        ],
        T::OutputEllipse => &[
            K::LineAttributes,
            K::Width,
            K::Height,
            K::EllipseType,
            K::StartAngle,
            K::EndAngle,
            K::FillAttributes,
        ],
        T::OutputPolygon => &[
            K::Width,
            K::Height,
            K::LineAttributes,
            K::FillAttributes,
            K::PolygonType,
        ],
        T::OutputMeter => &[
            K::Width,
            K::NeedleColour,
            K::BorderColour,
            K::ArcAndTickColour,
            K::Options,
            K::NrOfTicks,
            K::StartAngle,
            K::EndAngle,
            K::MinValue,
            K::MaxValue,
            K::VariableReference,
        ],
        T::OutputLinearBarGraph => &[
            K::Width,
            K::Height,
            K::Colour,
            K::TargetLineColour,
            K::Options,
            K::NrOfTicks,
            K::MinValue,
            K::MaxValue,
            K::VariableReference,
            K::Value,
            K::TargetValueVariableReference,
        ],
        T::OutputArchedBarGraph => &[
            K::Width,
            K::Height,
            K::Colour,
            K::TargetLineColour,
            K::Options,
            K::StartAngle,
            K::EndAngle,
            K::BarGraphWidth,
            K::MinValue,
            K::MaxValue,
            K::VariableReference,
            K::Value,
            K::TargetValueVariableReference,
        ],
        T::LineAttributes => &[K::LineColour, K::LineWidth, K::LineArt],
        T::FillAttributes => &[K::FillType, K::FillColour, K::FillPattern],
        T::FontAttributes => &[K::FontColour, K::FontSize, K::FontType, K::FontStyle],
        T::PictureGraphic => &[K::Width, K::ActualWidth, K::ActualHeight],
        T::WindowMask => &[K::CellFormat, K::WindowType, K::BackgroundColour],
        T::GraphicsContext => &[
            K::ViewportWidth,
            K::ViewportHeight,
            K::ViewportX,
            K::ViewportY,
            K::CanvasWidth,
            K::CanvasHeight,
            K::ViewportZoom,
            K::GraphicsCursorX,
            K::GraphicsCursorY,
            K::ForegroundColour,
            K::BackgroundColour,
            K::FontAttributesObject,
            K::LineAttributesObject,
            K::FillAttributesObject,
        ],
        T::Animation => &[
            K::Width,
            K::Height,
            K::RefreshInterval,
            K::Value,
            K::Enabled,
            K::FirstChildIndex,
            K::LastChildIndex,
            K::DefaultChildIndex,
            K::Options,
        ],
        T::ScaledGraphic => &[K::Width, K::Height, K::ScaleType, K::Options, K::Value],
        _ => return None,
    };
    keys.get(usize::from(aid.checked_sub(1)?)).copied()
}

fn attribute_value(
    object_type: ObjectType,
    key: crate::object_properties::PropertyKey,
    aid: u8,
    raw: u32,
    version: VtVersion,
) -> Result<Value, String> {
    use crate::macro_id_remap::{macro_attribute_value_kind, MacroAttributeValueKind};
    use crate::object_properties::{PropertyKey as K, PropertySemantic};
    use ag_iso_stack::object_pool::object_attributes::{
        Alignment, FormatType, HorizontalAlignment, VerticalAlignment,
    };
    let kind = macro_attribute_value_kind(object_type, Some(aid), version);
    if kind == MacroAttributeValueKind::Unknown {
        return Err(format!(
            "Attribute ID {aid} is unavailable or read-only for {object_type:?}"
        ));
    }
    if let MacroAttributeValueKind::LocalObjectId16 { value_offset } = kind {
        if value_offset != 0 {
            return Err("Attribute reference uses an unsupported byte offset".into());
        }
        return Ok(Value::from(raw as u16));
    }
    let signed = raw as i32;
    let number = Value::from(if key == K::Offset {
        signed as i64
    } else {
        i64::from(raw)
    });
    Ok(match key {
        K::Scale => {
            let value = f32::from_bits(raw);
            if !value.is_finite() {
                return Err("Scale attribute must be finite".into());
            }
            serde_json::Number::from_f64(f64::from(value))
                .map(Value::Number)
                .ok_or_else(|| "Scale attribute is invalid".to_owned())?
        }
        K::Options => bitflag_value(object_type, raw as u8)?,
        K::Options2 => bitflag_value(object_type, raw as u8)?,
        K::FontStyle => serde_json::to_value(FontStyle::from(raw as u8)).map_err(err)?,
        K::FontSize => serde_json::to_value(FontSize::from(raw as u8)).map_err(err)?,
        K::FontType => serde_json::to_value(FontType::from(raw as u8)).map_err(err)?,
        K::LineDirection => serde_json::to_value(match raw as u8 {
            0 => ag_iso_stack::object_pool::object_attributes::LineDirection::TopLeftToBottomRight,
            1 => ag_iso_stack::object_pool::object_attributes::LineDirection::BottomLeftToTopRight,
            _ => return Err("Line direction must be 0 or 1".into()),
        })
        .map_err(err)?,
        K::Format => serde_json::to_value(FormatType::from(raw & 1 != 0)).map_err(err)?,
        K::Justification => serde_json::to_value(Alignment {
            horizontal: HorizontalAlignment::from([raw & 0x80 != 0, raw & 0x40 != 0]),
            vertical: VerticalAlignment::from([raw & 0x20 != 0, raw & 0x10 != 0]),
        })
        .map_err(err)?,
        _ => {
            let descriptor = crate::object_properties::get_property_descriptors(object_type)
                .into_iter()
                .find(|descriptor| descriptor.name == key)
                .ok_or_else(|| format!("Property {key} is not available for {object_type:?}"))?;
            if matches!(descriptor.semantic, PropertySemantic::Bitflags) {
                bitflag_value(object_type, raw as u8)?
            } else if matches!(object_type, ObjectType::OutputEllipse) && key == K::EllipseType {
                serde_json::to_value(raw as u8).map_err(err)?
            } else {
                number
            }
        }
    })
}

fn bitflag_value(object_type: ObjectType, raw: u8) -> Result<Value, String> {
    use ag_iso_stack::object_pool::object_attributes::*;
    let value = match object_type {
        ObjectType::Button => serde_json::to_value(ButtonOptions::from(raw)),
        ObjectType::InputString => serde_json::to_value(InputStringOptions::from(raw)),
        ObjectType::OutputString => serde_json::to_value(OutputStringOptions::from(raw)),
        ObjectType::InputNumber | ObjectType::OutputNumber => {
            serde_json::to_value(NumberOptions::from(raw))
        }
        ObjectType::InputList => serde_json::to_value(InputListOptions::from(raw)),
        ObjectType::OutputRectangle => serde_json::to_value(raw),
        _ => {
            return Err(format!(
                "Bitfield options are not supported for {object_type:?}"
            ));
        }
    };
    value.map_err(err)
}

fn set(pool: &mut ObjectPool, id: ObjectId, property: &str, value: Value) -> Result<(), String> {
    let key = property.parse().map_err(err)?;
    crate::object_properties::set_property(pool, id, key, value)
        .map(|_| ())
        .map_err(err)
}

fn require_type(pool: &ObjectPool, id: ObjectId, expected: ObjectType) -> Result<(), String> {
    match pool.object_by_id(id) {
        Some(object) if object.object_type() == expected => Ok(()),
        Some(object) => Err(format!(
            "Object {} is {:?}; expected {expected:?}",
            id.value(),
            object.object_type()
        )),
        None => Err(missing(id)),
    }
}

fn set_nullable_reference(
    pool: &mut ObjectPool,
    id: ObjectId,
    property: &str,
    value: &str,
) -> Result<(), String> {
    let value = nullable_object_id(value)?;
    set(
        pool,
        id,
        property,
        serde_json::to_value(value.map(|id| id.value())).map_err(err)?,
    )
}

fn replace_nullable_list_item(
    items: &mut [ag_iso_stack::object_pool::NullableObjectId],
    index: usize,
    value: Option<ObjectId>,
    id: ObjectId,
) -> Result<(), String> {
    let item = items.get_mut(index).ok_or_else(|| index_error(id, index))?;
    *item = nullable_id(value);
    Ok(())
}

fn require_args(args: &[String], count: usize, name: &str) -> Result<(), String> {
    if args.len() == count {
        Ok(())
    } else {
        Err(format!(
            "{name} requires {count} arguments, received {}",
            args.len()
        ))
    }
}
fn integer(value: &str, label: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("{label} must be an unsigned 32-bit integer"))
}
fn number(value: &str, label: &str) -> Result<u8, String> {
    value
        .parse()
        .map_err(|_| format!("{label} must be an unsigned byte"))
}
fn object_id(value: &str) -> Result<ObjectId, String> {
    let value = integer(value, "object ID")?;
    ObjectId::new(u16::try_from(value).map_err(err)?)
        .map_err(|_| format!("Object ID {value} is reserved for NULL"))
}
fn nullable_object_id(value: &str) -> Result<Option<ObjectId>, String> {
    if value == "null" {
        Ok(None)
    } else {
        object_id(value).map(Some)
    }
}
fn missing(id: ObjectId) -> String {
    format!("Object {} does not exist", id.value())
}
fn index_error(id: ObjectId, index: usize) -> String {
    format!("Object {} has no item at index {index}", id.value())
}
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn small_integer(value: &str, label: &str) -> Result<u16, String> {
    u16::try_from(integer(value, label)?).map_err(err)
}
fn nullable_id(value: Option<ObjectId>) -> ag_iso_stack::object_pool::NullableObjectId {
    value.map_or(
        ag_iso_stack::object_pool::NullableObjectId::NULL,
        Into::into,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macro_authoring::MacroInstruction;
    use ag_iso_stack::object_pool::{object::Object, Colour};

    fn instruction(name: &str, args: &[&str]) -> MacroInstruction {
        MacroInstruction {
            name: name.into(),
            arguments: args.iter().map(|arg| (*arg).into()).collect(),
        }
    }

    #[test]
    fn polygon_scale_resizes_points_and_failed_command_is_atomic() {
        let mut pool = ObjectPool::new();
        let mut polygon = crate::object_defaults::default_object(ObjectType::OutputPolygon);
        let Object::OutputPolygon(polygon) = &mut polygon else {
            unreachable!()
        };
        polygon.id = ObjectId::new(7).unwrap();
        polygon.width = 10;
        polygon.height = 20;
        polygon.points[0].x = 5;
        polygon.points[0].y = 10;
        pool.add(Object::OutputPolygon(polygon.clone()));

        apply(
            &mut pool,
            &instruction("change-polygon-scale", &["7", "20", "10"]),
            VtVersion::Version6,
        )
        .unwrap();
        let Some(Object::OutputPolygon(polygon)) = pool.object_by_id(ObjectId::new(7).unwrap())
        else {
            panic!()
        };
        assert_eq!(
            (
                polygon.width,
                polygon.height,
                polygon.points[0].x,
                polygon.points[0].y
            ),
            (20, 10, 10, 5)
        );
        let before = pool.clone();
        assert!(apply(
            &mut pool,
            &instruction("change-polygon-point", &["7", "99", "1", "2"]),
            VtVersion::Version6
        )
        .is_err());
        assert_eq!(
            pool.object_by_id(ObjectId::new(7).unwrap()),
            before.object_by_id(ObjectId::new(7).unwrap())
        );
    }

    #[test]
    fn colour_selection_updates_temporary_controls_and_null_restores_defaults() {
        let mut pool = ObjectPool::new();
        let map_id = ObjectId::new(8).unwrap();
        pool.add(Object::ColourMap(
            ag_iso_stack::object_pool::object::ColourMap {
                id: map_id,
                colour_map: vec![0],
            },
        ));
        let palette_id = ObjectId::new(9).unwrap();
        pool.add(Object::ColourPalette(
            ag_iso_stack::object_pool::object::ColourPalette {
                id: palette_id,
                options: ag_iso_stack::object_pool::object_attributes::ColourPaletteOptions {},
                colours: vec![Colour::COLOUR_PALETTE[2]],
            },
        ));
        apply(
            &mut pool,
            &instruction("select-colour-map-or-palette", &["8"]),
            VtVersion::Version6,
        )
        .unwrap();
        apply(
            &mut pool,
            &instruction("select-colour-map-or-palette", &["9"]),
            VtVersion::Version6,
        )
        .unwrap();
        assert_eq!(
            crate::effective_colour::colour(&pool, 0),
            Colour::COLOUR_PALETTE[2]
        );
        apply(
            &mut pool,
            &instruction("select-colour-map-or-palette", &["null"]),
            VtVersion::Version6,
        )
        .unwrap();
        assert_eq!(
            crate::effective_colour::colour(&pool, 0),
            Colour::COLOUR_PALETTE[0]
        );
    }
}
