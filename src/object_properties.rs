//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//!
//! Typed property access layer for fine-grained object editing.
//!
//! This module defines the PropertyAccess trait and uses a macro to generate
//! implementations for each object type. This avoids repetitive boilerplate
//! while maintaining type safety and property documentation.
//!
//! For whole-object operations (CreateObject, DeleteObject, copy/paste),
//! use Object::serialize/deserialize directly via Serde.

use ag_iso_stack::object_pool::{ObjectId, ObjectPool, ObjectType};
use serde_json::Value;
use std::collections::HashMap;

/// Property metadata for UI and validation
#[derive(Debug, Clone)]
pub struct PropertyDescriptor {
    pub name: &'static str,
    pub writable: bool,
    pub semantic: PropertySemantic,
    pub valid_range: Option<(Value, Value)>,
}

/// Meaning that cannot be recovered reliably from a property's JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertySemantic {
    Normal,
    Colour,
    ObjectReference { allowed_types: Vec<ObjectType> },
    Enum,
    Bitflags,
}

/// Error from a single object's property operation
/// Pool-level dispatcher converts this to PropertyError with object_id
#[derive(Debug, Clone)]
pub enum PropertyAccessError {
    PropertyNotFound(String),
    UnsupportedObjectType,
    InvalidValue {
        property: &'static str,
        reason: String,
    },
}

/// Error from pool-level property access (includes object context)
#[derive(Debug, Clone)]
pub enum PropertyError {
    ObjectNotFound(ObjectId),
    PropertyNotFound {
        object_id: ObjectId,
        property: String,
    },
    UnsupportedObjectType {
        object_id: ObjectId,
        object_type: String,
    },
    InvalidValue {
        object_id: ObjectId,
        property: String,
        reason: String,
    },
    OutOfRange {
        property: String,
        value: String,
        min: String,
        max: String,
    },
    InvalidObjectReference {
        object_id: ObjectId,
        reference_id: ObjectId,
    },
}

/// Trait for property access on individual object types
/// Implementations are generated via impl_properties! macro
pub trait PropertyAccess {
    /// Get a property value as JSON
    fn get_property(&self, property: &str) -> Result<Value, PropertyAccessError>;

    /// Set a property to a new value, returning the old value
    fn set_property(&mut self, property: &str, value: Value) -> Result<Value, PropertyAccessError>;

    /// Get all property descriptors for this object
    fn property_descriptors() -> Vec<PropertyDescriptor>;
}

/// Macro to generate PropertyAccess implementations for object types.
/// Takes field names plus optional semantic metadata; Serde handles Rust types.
///
/// Example:
/// ```ignore
/// impl_properties! {
///     Button {
///         width,
///         height,
///         background_colour => PropertySemantic::Colour,
///         border_colour => PropertySemantic::Colour,
///         key_code,
///     }
/// }
/// ```
macro_rules! property_semantic {
    () => {
        PropertySemantic::Normal
    };
    ($semantic:expr) => {
        $semantic
    };
}

macro_rules! object_reference {
    ($($object_type:ident),* $(,)?) => {
        PropertySemantic::ObjectReference {
            allowed_types: vec![$(ObjectType::$object_type),*],
        }
    };
}

macro_rules! impl_properties {
    (
        $type:ty {
            $( $field:ident $(=> $semantic:expr)? ),* $(,)?
        }
    ) => {
        impl PropertyAccess for $type {
            fn get_property(&self, property: &str) -> Result<Value, PropertyAccessError> {
                match property {
                    $(
                        stringify!($field) => {
                            serde_json::to_value(&self.$field)
                                .map_err(|e| PropertyAccessError::InvalidValue {
                                    property: stringify!($field),
                                    reason: e.to_string(),
                                })
                        }
                    )*
                    _ => Err(PropertyAccessError::PropertyNotFound(property.to_string())),
                }
            }

            #[allow(unused_variables)]
            fn set_property(&mut self, property: &str, value: Value) -> Result<Value, PropertyAccessError> {
                match property {
                    $(
                        stringify!($field) => {
                            let old = serde_json::to_value(&self.$field)
                                .map_err(|e| PropertyAccessError::InvalidValue {
                                    property: stringify!($field),
                                    reason: e.to_string(),
                                })?;

                            self.$field = serde_json::from_value(value)
                                .map_err(|e| PropertyAccessError::InvalidValue {
                                    property: stringify!($field),
                                    reason: e.to_string(),
                                })?;

                            Ok(old)
                        }
                    )*
                    _ => Err(PropertyAccessError::PropertyNotFound(property.to_string())),
                }
            }

            fn property_descriptors() -> Vec<PropertyDescriptor> {
                vec![
                    $(
                        PropertyDescriptor {
                            name: stringify!($field),
                            writable: true,
                            semantic: property_semantic!($($semantic)?),
                            valid_range: None,
                        },
                    )*
                ]
            }
        }
    };
}

// Identity, child placement/reference collections, and macro references are
// deliberately managed by their dedicated operations rather than SetProperty.
impl_properties! {
    ag_iso_stack::object_pool::object::WorkingSet {
        background_colour => PropertySemantic::Colour,
        selectable,
        active_mask => object_reference!(DataMask, AlarmMask),
        language_codes
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::DataMask {
        background_colour => PropertySemantic::Colour,
        soft_key_mask => object_reference!(SoftKeyMask)
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::AlarmMask {
        background_colour => PropertySemantic::Colour,
        soft_key_mask => object_reference!(SoftKeyMask),
        priority,
        acoustic_signal
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::Container {
        width,
        height,
        hidden
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::SoftKeyMask {
        background_colour => PropertySemantic::Colour
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::Key {
        background_colour => PropertySemantic::Colour,
        key_code
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::Button {
        width,
        height,
        background_colour => PropertySemantic::Colour,
        border_colour => PropertySemantic::Colour,
        key_code,
        options => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::InputBoolean {
        background_colour => PropertySemantic::Colour,
        width,
        foreground_colour => object_reference!(FontAttributes),
        variable_reference => object_reference!(NumberVariable),
        value,
        enabled
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::InputString {
        width,
        height,
        background_colour => PropertySemantic::Colour,
        font_attributes => object_reference!(FontAttributes),
        input_attributes => object_reference!(InputAttributes),
        options => PropertySemantic::Bitflags,
        variable_reference => object_reference!(StringVariable),
        value,
        justification,
        enabled
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::InputNumber {
        width,
        height,
        background_colour => PropertySemantic::Colour,
        font_attributes => object_reference!(FontAttributes),
        options => PropertySemantic::Bitflags,
        variable_reference => object_reference!(NumberVariable),
        value,
        min_value,
        max_value,
        offset,
        scale,
        nr_of_decimals,
        format => PropertySemantic::Enum,
        justification,
        options2 => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::InputList {
        width,
        height,
        variable_reference => object_reference!(NumberVariable),
        value,
        options => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputString {
        width,
        height,
        background_colour => PropertySemantic::Colour,
        font_attributes => object_reference!(FontAttributes),
        options => PropertySemantic::Bitflags,
        variable_reference => object_reference!(StringVariable),
        justification,
        value
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputNumber {
        width,
        height,
        background_colour => PropertySemantic::Colour,
        font_attributes => object_reference!(FontAttributes),
        options => PropertySemantic::Bitflags,
        variable_reference => object_reference!(NumberVariable),
        value,
        offset,
        scale,
        nr_of_decimals,
        format => PropertySemantic::Enum,
        justification
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputList {
        width,
        height,
        variable_reference => object_reference!(NumberVariable),
        value
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputLine {
        line_attributes => object_reference!(LineAttributes),
        width,
        height,
        line_direction => PropertySemantic::Enum
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputRectangle {
        line_attributes => object_reference!(LineAttributes),
        width,
        height,
        line_suppression => PropertySemantic::Bitflags,
        fill_attributes => object_reference!(FillAttributes)
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputEllipse {
        line_attributes => object_reference!(LineAttributes),
        width,
        height,
        ellipse_type => PropertySemantic::Enum,
        start_angle,
        end_angle,
        fill_attributes => object_reference!(FillAttributes)
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputPolygon {
        width,
        height,
        line_attributes => object_reference!(LineAttributes),
        fill_attributes => object_reference!(FillAttributes),
        polygon_type => PropertySemantic::Enum,
        points
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputMeter {
        width,
        needle_colour => PropertySemantic::Colour,
        border_colour => PropertySemantic::Colour,
        arc_and_tick_colour => PropertySemantic::Colour,
        options => PropertySemantic::Bitflags,
        nr_of_ticks,
        start_angle,
        end_angle,
        min_value,
        max_value,
        variable_reference => object_reference!(NumberVariable),
        value
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputLinearBarGraph {
        width,
        height,
        colour => PropertySemantic::Colour,
        target_line_colour => PropertySemantic::Colour,
        options => PropertySemantic::Bitflags,
        nr_of_ticks,
        min_value,
        max_value,
        variable_reference => object_reference!(NumberVariable),
        value,
        target_value_variable_reference => object_reference!(NumberVariable),
        target_value
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::OutputArchedBarGraph {
        width,
        height,
        colour => PropertySemantic::Colour,
        target_line_colour => PropertySemantic::Colour,
        options => PropertySemantic::Bitflags,
        start_angle,
        end_angle,
        bar_graph_width,
        min_value,
        max_value,
        variable_reference => object_reference!(NumberVariable),
        value,
        target_value_variable_reference => object_reference!(NumberVariable),
        target_value
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::PictureGraphic {
        width,
        actual_width,
        actual_height,
        format => PropertySemantic::Enum,
        options => PropertySemantic::Bitflags,
        transparency_colour => PropertySemantic::Colour,
        data
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::NumberVariable { value }
}

impl_properties! {
    ag_iso_stack::object_pool::object::StringVariable { value }
}

impl_properties! {
    ag_iso_stack::object_pool::object::FontAttributes {
        font_colour => PropertySemantic::Colour,
        font_size => PropertySemantic::Enum,
        font_type => PropertySemantic::Enum,
        font_style => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::LineAttributes {
        line_colour => PropertySemantic::Colour,
        line_width,
        line_art => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::FillAttributes {
        fill_type => PropertySemantic::Enum,
        fill_colour => PropertySemantic::Colour,
        fill_pattern => object_reference!(PictureGraphic)
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::InputAttributes {
        validation_type => PropertySemantic::Enum,
        validation_string
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ObjectPointer {
        value => object_reference!()
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::Macro { commands }
}

impl_properties! {
    ag_iso_stack::object_pool::object::AuxiliaryFunctionType1 {
        background_colour => PropertySemantic::Colour,
        function_type => PropertySemantic::Enum
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::AuxiliaryInputType1 {
        background_colour => PropertySemantic::Colour,
        function_type => PropertySemantic::Enum,
        input_id
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::AuxiliaryFunctionType2 {
        background_colour => PropertySemantic::Colour,
        function_attributes => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::AuxiliaryInputType2 {
        background_colour => PropertySemantic::Colour,
        function_attributes => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::AuxiliaryControlDesignatorType2 {
        pointer_type => PropertySemantic::Enum,
        auxiliary_object_id => object_reference!(AuxiliaryFunctionType2, AuxiliaryInputType2)
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::WindowMask {
        cell_format => PropertySemantic::Enum,
        window_type => PropertySemantic::Enum,
        background_colour => PropertySemantic::Colour,
        options => PropertySemantic::Bitflags,
        name => object_reference!(OutputString),
        window_title => object_reference!(OutputString),
        window_icon => object_reference!(PictureGraphic)
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::KeyGroup {
        options => PropertySemantic::Bitflags,
        name => object_reference!(OutputString),
        key_group_icon => object_reference!(PictureGraphic)
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::GraphicsContext {
        viewport_width,
        viewport_height,
        viewport_x,
        viewport_y,
        canvas_width,
        canvas_height,
        viewport_zoom,
        graphics_cursor_x,
        graphics_cursor_y,
        foreground_colour => PropertySemantic::Colour,
        background_colour => PropertySemantic::Colour,
        font_attributes_object => object_reference!(FontAttributes),
        line_attributes_object => object_reference!(LineAttributes),
        fill_attributes_object => object_reference!(FillAttributes),
        format => PropertySemantic::Enum,
        options => PropertySemantic::Bitflags,
        transparency_colour => PropertySemantic::Colour
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ExtendedInputAttributes {
        validation_type => PropertySemantic::Enum,
        code_planes
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ColourMap { colour_map }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ObjectLabelReferenceList {}
}

impl_properties! {
    ag_iso_stack::object_pool::object::ExternalObjectDefinition {
        options => PropertySemantic::Bitflags,
        name
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ExternalReferenceName {
        options => PropertySemantic::Bitflags,
        name
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ExternalObjectPointer {
        default_object_id => object_reference!(),
        external_reference_name_id => object_reference!(ExternalReferenceName),
        external_object_id => object_reference!()
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::Animation {
        width,
        height,
        refresh_interval,
        value,
        enabled,
        first_child_index,
        last_child_index,
        default_child_index,
        options => PropertySemantic::Bitflags
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ColourPalette {
        options => PropertySemantic::Bitflags,
        colours
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::GraphicData {
        format => PropertySemantic::Enum,
        data
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::WorkingSetSpecialControls {
        id_of_colour_map => object_reference!(ColourMap),
        id_of_colour_palette => object_reference!(ColourPalette),
        language_pairs
    }
}

impl_properties! {
    ag_iso_stack::object_pool::object::ScaledGraphic {
        width,
        height,
        scale_type => PropertySemantic::Enum,
        options => PropertySemantic::Bitflags,
        value => object_reference!(PictureGraphic, GraphicData)
    }
}

// Master list used to generate every match over property-enabled objects.
macro_rules! supported_property_objects {
    ($macro:ident, $($args:tt)*) => {
        $macro! {
            $($args)*;
            WorkingSet,
            DataMask,
            AlarmMask,
            Container,
            SoftKeyMask,
            Key,
            Button,
            InputBoolean,
            InputString,
            InputNumber,
            InputList,
            OutputString,
            OutputNumber,
            OutputList,
            OutputLine,
            OutputRectangle,
            OutputEllipse,
            OutputPolygon,
            OutputMeter,
            OutputLinearBarGraph,
            OutputArchedBarGraph,
            PictureGraphic,
            NumberVariable,
            StringVariable,
            FontAttributes,
            LineAttributes,
            FillAttributes,
            InputAttributes,
            ObjectPointer,
            Macro,
            AuxiliaryFunctionType1,
            AuxiliaryInputType1,
            AuxiliaryFunctionType2,
            AuxiliaryInputType2,
            AuxiliaryControlDesignatorType2,
            WindowMask,
            KeyGroup,
            GraphicsContext,
            ExtendedInputAttributes,
            ColourMap,
            ObjectLabelReferenceList,
            ExternalObjectDefinition,
            ExternalReferenceName,
            ExternalObjectPointer,
            Animation,
            ColourPalette,
            GraphicData,
            WorkingSetSpecialControls,
            ScaledGraphic,
        }
    };
}

macro_rules! dispatch_property_impl {
    ($obj:expr, $method:ident, $args:tt; $($variant:ident),* $(,)?) => {
        #[allow(unreachable_patterns)]
        match $obj {
            $(
                ag_iso_stack::object_pool::object::Object::$variant(v) => {
                    v.$method$args
                }
            )*
            _ => Err(PropertyAccessError::UnsupportedObjectType),
        }
    };
}

/// Call a PropertyAccess method for any supported object variant.
macro_rules! dispatch_property {
    ($obj:expr, $method:ident $(, $arg:expr)*) => {
        supported_property_objects!(dispatch_property_impl, $obj, $method, ($($arg),*))
    };
}

macro_rules! property_descriptors_impl {
    ($object_type:expr; $($variant:ident),* $(,)?) => {
        #[allow(unreachable_patterns)]
        match $object_type {
            $(
                ObjectType::$variant =>
                    <ag_iso_stack::object_pool::object::$variant as PropertyAccess>::property_descriptors(),
            )*
            _ => Vec::new(),
        }
    };
}

macro_rules! property_access_supported_impl {
    ($object_type:expr; $($variant:ident),* $(,)?) => {
        #[allow(unreachable_patterns)]
        match $object_type {
            $(ObjectType::$variant => true,)*
            _ => false,
        }
    };
}

#[cfg(test)]
macro_rules! supported_object_types_impl {
    (; $($variant:ident),* $(,)?) => {
        vec![$(ObjectType::$variant),*]
    };
}

/// Get a property from an object in the pool
pub fn get_property(
    pool: &ObjectPool,
    object_id: ObjectId,
    property: &str,
) -> Result<Value, PropertyError> {
    let obj = pool
        .object_by_id(object_id)
        .ok_or(PropertyError::ObjectNotFound(object_id))?;

    dispatch_property!(obj, get_property, property).map_err(|e| match e {
        PropertyAccessError::PropertyNotFound(p) => PropertyError::PropertyNotFound {
            object_id,
            property: p,
        },
        PropertyAccessError::UnsupportedObjectType => PropertyError::UnsupportedObjectType {
            object_id,
            object_type: format!("{:?}", obj.object_type()),
        },
        PropertyAccessError::InvalidValue { property, reason } => PropertyError::InvalidValue {
            object_id,
            property: property.to_string(),
            reason,
        },
    })
}

/// Set a property on an object in the pool
pub fn set_property(
    pool: &mut ObjectPool,
    object_id: ObjectId,
    property: &str,
    value: Value,
) -> Result<Value, PropertyError> {
    let obj = pool
        .object_mut_by_id(object_id)
        .ok_or(PropertyError::ObjectNotFound(object_id))?;

    dispatch_property!(obj, set_property, property, value).map_err(|e| match e {
        PropertyAccessError::PropertyNotFound(p) => PropertyError::PropertyNotFound {
            object_id,
            property: p,
        },
        PropertyAccessError::UnsupportedObjectType => PropertyError::UnsupportedObjectType {
            object_id,
            object_type: format!("{:?}", obj.object_type()),
        },
        PropertyAccessError::InvalidValue { property, reason } => PropertyError::InvalidValue {
            object_id,
            property: property.to_string(),
            reason,
        },
    })
}

pub fn get_property_descriptors(object_type: ObjectType) -> Vec<PropertyDescriptor> {
    supported_property_objects!(property_descriptors_impl, object_type)
}

/// Get every non-identity, non-placement property exposed by an object.
pub fn get_properties(
    pool: &ObjectPool,
    object_id: ObjectId,
) -> Result<HashMap<String, Value>, PropertyError> {
    let object = pool
        .object_by_id(object_id)
        .ok_or(PropertyError::ObjectNotFound(object_id))?;
    get_property_descriptors(object.object_type())
        .into_iter()
        .map(|descriptor| {
            get_property(pool, object_id, descriptor.name)
                .map(|value| (descriptor.name.to_owned(), value))
        })
        .collect()
}

fn property_access_supported(object_type: ObjectType) -> bool {
    supported_property_objects!(property_access_supported_impl, object_type)
}

pub fn validate_property(
    pool: &ObjectPool,
    object_id: ObjectId,
    property: &str,
    value: &Value,
) -> Result<(), PropertyError> {
    let object = pool
        .object_by_id(object_id)
        .ok_or(PropertyError::ObjectNotFound(object_id))?;
    let descriptors = get_property_descriptors(object.object_type());
    if !property_access_supported(object.object_type()) {
        return Err(PropertyError::UnsupportedObjectType {
            object_id,
            object_type: format!("{:?}", object.object_type()),
        });
    }

    let descriptor = descriptors
        .into_iter()
        .find(|descriptor| descriptor.name == property)
        .ok_or_else(|| PropertyError::PropertyNotFound {
            object_id,
            property: property.to_string(),
        })?;

    if !descriptor.writable {
        return Err(PropertyError::InvalidValue {
            object_id,
            property: property.to_string(),
            reason: "property is read-only".to_string(),
        });
    }

    if let Some((min, max)) = descriptor.valid_range {
        let number = value.as_f64();
        let minimum = min.as_f64();
        let maximum = max.as_f64();
        if !matches!((number, minimum, maximum), (Some(value), Some(min), Some(max)) if value >= min && value <= max)
        {
            return Err(PropertyError::OutOfRange {
                property: property.to_string(),
                value: value.to_string(),
                min: min.to_string(),
                max: max.to_string(),
            });
        }
    }

    match descriptor.semantic {
        PropertySemantic::Colour => {
            if !matches!(value.as_u64(), Some(0..=255)) {
                return Err(PropertyError::InvalidValue {
                    object_id,
                    property: property.to_string(),
                    reason: "colour must be an integer from 0 through 255".to_string(),
                });
            }
        }
        PropertySemantic::ObjectReference { allowed_types } => {
            // Null is valid metadata-wise for NullableObjectId. Required ObjectId
            // fields are still rejected by their Serde deserializer in set_property.
            if value.is_null() {
                return Ok(());
            }
            let reference_id =
                serde_json::from_value::<ObjectId>(value.clone()).map_err(|error| {
                    PropertyError::InvalidValue {
                        object_id,
                        property: property.to_string(),
                        reason: error.to_string(),
                    }
                })?;
            let valid_reference = pool.object_by_id(reference_id).is_some_and(|referenced| {
                allowed_types.is_empty() || allowed_types.contains(&referenced.object_type())
            });
            if !valid_reference {
                return Err(PropertyError::InvalidObjectReference {
                    object_id,
                    reference_id,
                });
            }
        }
        PropertySemantic::Normal | PropertySemantic::Enum | PropertySemantic::Bitflags => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_object;
    use serde_json::json;

    fn supported_object_types() -> Vec<ObjectType> {
        supported_property_objects!(supported_object_types_impl,)
    }

    #[test]
    fn descriptors_contain_only_semantics_not_primitive_types() {
        let descriptors = get_property_descriptors(ObjectType::Button);

        assert!(descriptors.iter().any(|descriptor| {
            descriptor.name == "width" && descriptor.semantic == PropertySemantic::Normal
        }));
        assert!(descriptors.iter().any(|descriptor| {
            descriptor.name == "background_colour"
                && descriptor.semantic == PropertySemantic::Colour
        }));
    }

    #[test]
    fn invalid_property_value_does_not_mutate_the_object() {
        let mut pool = ObjectPool::default();
        let button = default_object(ObjectType::Button);
        let button_id = button.id();
        pool.add(button);

        let result = set_property(&mut pool, button_id, "width", json!("not a number"));

        assert!(matches!(result, Err(PropertyError::InvalidValue { .. })));
        assert!(matches!(
            get_property(&pool, button_id, "width"),
            Ok(value) if value == json!(0)
        ));
    }

    #[test]
    fn semantic_validation_rejects_an_invalid_colour() {
        let mut pool = ObjectPool::default();
        let button = default_object(ObjectType::Button);
        let button_id = button.id();
        pool.add(button);

        let result = validate_property(&pool, button_id, "background_colour", &json!(256));

        assert!(matches!(result, Err(PropertyError::InvalidValue { .. })));
    }

    #[test]
    fn reference_validation_checks_nullability_and_allowed_types() {
        let mut pool = ObjectPool::default();

        let mut input = default_object(ObjectType::InputString);
        *input.mut_id() = ObjectId::new(1).unwrap();
        let input_id = input.id();
        pool.add(input);

        let mut font = default_object(ObjectType::FontAttributes);
        *font.mut_id() = ObjectId::new(2).unwrap();
        pool.add(font);

        let mut number = default_object(ObjectType::NumberVariable);
        *number.mut_id() = ObjectId::new(3).unwrap();
        pool.add(number);

        assert!(validate_property(&pool, input_id, "font_attributes", &json!(2)).is_ok());
        assert!(matches!(
            validate_property(&pool, input_id, "font_attributes", &json!(3)),
            Err(PropertyError::InvalidObjectReference { .. })
        ));
        assert!(validate_property(&pool, input_id, "variable_reference", &Value::Null).is_ok());
    }

    #[test]
    fn every_object_variant_dispatches_and_round_trips_its_properties() {
        let mut object_types = supported_object_types();
        object_types.sort_by_key(|object_type| u8::from(*object_type));
        let all_object_types: Vec<_> = (0..=u8::MAX)
            .filter_map(|value| ObjectType::try_from(value).ok())
            .collect();
        assert_eq!(object_types, all_object_types);

        for object_type in object_types {
            let mut pool = ObjectPool::default();
            let object = default_object(object_type);
            let object_id = object.id();
            pool.add(object);

            assert!(matches!(
                get_property(&pool, object_id, "__unknown_property__"),
                Err(PropertyError::PropertyNotFound { .. })
            ));

            for descriptor in get_property_descriptors(object_type) {
                let value =
                    get_property(&pool, object_id, descriptor.name).unwrap_or_else(|error| {
                        panic!("{object_type:?}.{}: {error:?}", descriptor.name)
                    });
                let old_value = set_property(&mut pool, object_id, descriptor.name, value.clone())
                    .unwrap_or_else(|error| {
                        panic!("{object_type:?}.{}: {error:?}", descriptor.name)
                    });
                assert_eq!(old_value, value, "{object_type:?}.{}", descriptor.name);
            }
        }
    }
}
