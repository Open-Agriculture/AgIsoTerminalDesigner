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

use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool, ObjectType};
use serde_json::Value;
use std::collections::HashMap;

/// Property metadata for UI and validation
#[derive(Debug, Clone)]
pub struct PropertyDescriptor {
    pub name: &'static str,
    pub writable: bool,
    pub semantic: PropertySemantic,
    pub valid_range: Option<(Value, Value)>,
    /// Serialized choices derived from the field type, shared by all editors.
    pub enum_choices: Vec<Value>,
}

/// Presentation metadata for the generic property editor.
///
/// This intentionally complements `PropertyDescriptor`: validation semantics
/// stay available to all callers, while UI-only labels and widget choices are
/// declared only for object types that use the generic renderer.
#[derive(Debug, Clone, Copy)]
pub struct PropertyEditorDescriptor {
    pub property: &'static str,
    pub editor: EditorKind,
}

#[derive(Debug, Clone, Copy)]
pub enum EditorKind {
    Auto,
    Colour,
    ObjectReference,
    Enum,
    /// A bit-mask or JSON object of boolean options.
    FlagSet,
    Justification,
    MacroReferences,
}

/// UI schemas for every writable property of an object type.
///
/// This is deliberately derived from `PropertyAccess`, so configuration cannot
/// silently fall back to an object-specific widget when a new property is
/// added.
pub fn property_editor_descriptors(object_type: ObjectType) -> Vec<PropertyEditorDescriptor> {
    let mut descriptors: Vec<_> = get_property_descriptors(object_type)
        .into_iter()
        .filter(|descriptor| descriptor.writable)
        .map(|descriptor| PropertyEditorDescriptor {
            property: descriptor.name,
            editor: editor_override(object_type, descriptor.name)
                .unwrap_or_else(|| default_editor(&descriptor.semantic)),
        })
        .collect();
    if supports_macro_references(object_type) {
        descriptors.push(PropertyEditorDescriptor {
            property: "macro_refs",
            editor: EditorKind::MacroReferences,
        });
    }
    descriptors
}

/// Map property semantics to the generic renderer's default widgets.
fn default_editor(semantic: &PropertySemantic) -> EditorKind {
    match semantic {
        PropertySemantic::Colour => EditorKind::Colour,
        PropertySemantic::ObjectReference { .. } => EditorKind::ObjectReference,
        PropertySemantic::Enum => EditorKind::Enum,
        PropertySemantic::Bitflags => EditorKind::FlagSet,
        PropertySemantic::Normal => EditorKind::Auto,
    }
}

/// Widgets whose presentation cannot be determined from their semantic alone.
fn editor_override(object_type: ObjectType, property: &str) -> Option<EditorKind> {
    match (object_type, property) {
        // These upstream fields are raw bytes, not Rust enums. Keep their numeric
        // editor explicit until the stack exposes typed variants.
        (ObjectType::AuxiliaryFunctionType1 | ObjectType::AuxiliaryInputType1, "function_type")
        | (ObjectType::AuxiliaryControlDesignatorType2, "pointer_type")
        | (ObjectType::OutputEllipse, "ellipse_type")
        | (ObjectType::OutputPolygon, "polygon_type")
        | (ObjectType::FillAttributes, "fill_type")
        | (ObjectType::GraphicData, "format")
        | (ObjectType::ScaledGraphic, "scale_type") => Some(EditorKind::Auto),
        (_, "justification") => Some(EditorKind::Justification),
        _ => None,
    }
}

fn supports_macro_references(object_type: ObjectType) -> bool {
    matches!(
        object_type,
        ObjectType::WorkingSet
            | ObjectType::DataMask
            | ObjectType::AlarmMask
            | ObjectType::Container
            | ObjectType::SoftKeyMask
            | ObjectType::Key
            | ObjectType::Button
            | ObjectType::InputBoolean
            | ObjectType::InputString
            | ObjectType::InputNumber
            | ObjectType::InputList
            | ObjectType::OutputString
            | ObjectType::OutputNumber
            | ObjectType::OutputList
            | ObjectType::OutputLine
            | ObjectType::OutputRectangle
            | ObjectType::OutputEllipse
            | ObjectType::OutputPolygon
            | ObjectType::OutputMeter
            | ObjectType::OutputLinearBarGraph
            | ObjectType::OutputArchedBarGraph
            | ObjectType::PictureGraphic
            | ObjectType::FontAttributes
            | ObjectType::LineAttributes
            | ObjectType::FillAttributes
            | ObjectType::InputAttributes
            | ObjectType::WindowMask
            | ObjectType::KeyGroup
            | ObjectType::Animation
            | ObjectType::ScaledGraphic
    )
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

impl std::fmt::Display for PropertyAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PropertyNotFound(property) => {
                write!(f, "Property {property} does not exist on this object.")
            }
            Self::UnsupportedObjectType => write!(
                f,
                "Editing properties of this object type is not supported."
            ),
            Self::InvalidValue { reason, .. } => write!(f, "Invalid value: {reason}"),
        }
    }
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

impl std::fmt::Display for PropertyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ObjectNotFound(id) => write!(f, "Object {} could not be found.", id.value()),
            Self::PropertyNotFound { property, .. } => write!(
                f,
                "The {} property does not exist on this object.",
                property.replace('_', " ")
            ),
            Self::UnsupportedObjectType { object_type, .. } => write!(
                f,
                "Editing properties of {object_type} objects is not supported."
            ),
            Self::InvalidValue {
                property, reason, ..
            } => {
                if property == "active_mask" && reason == "invalid type: null, expected u16" {
                    write!(
                        f,
                        "An active mask is required. Select a mask instead of clearing this value."
                    )
                } else if reason == "invalid type: null, expected u16" {
                    write!(f, "A value is required. Enter a whole number between 0 and 65535, or select an object if this is a reference.")
                } else {
                    write!(f, "Invalid value: {reason}")
                }
            }
            Self::OutOfRange {
                value, min, max, ..
            } => write!(f, "The value {value} must be between {min} and {max}."),
            Self::InvalidObjectReference { reference_id, .. } => write!(
                f,
                "Object {} is not a valid reference for this property. Select a compatible object.",
                reference_id.value()
            ),
        }
    }
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

// Special handling is limited to enums with payloads; unit variants are read
// directly from Serde metadata, so adding a Rust variant updates the UI too.
macro_rules! property_enum_choices {
    (font_size, $owner:ty) => {{
        use ag_iso_stack::object_pool::object_attributes::{FontSize, NonProportionalFontSize};
        let mut choices: Vec<Value> = enum_unit_choices::<NonProportionalFontSize>()
            .into_iter()
            .map(|value| {
                serde_json::to_value(FontSize::NonProportional(
                    serde_json::from_value(value).unwrap(),
                ))
                .unwrap()
            })
            .collect();
        choices.extend(
            // The preview supports every fixed font through 128×192, hence N=192.
            (8..=192).map(|height| serde_json::to_value(FontSize::Proportional(height)).unwrap()),
        );
        choices
    }};
    (font_type, $owner:ty) => {{
        use ag_iso_stack::object_pool::object_attributes::FontType;
        let mut choices = enum_unit_choices::<FontType>();
        choices.extend(
            (240..=255).map(|code| serde_json::to_value(FontType::Proprietary(code)).unwrap()),
        );
        choices
    }};
    ($field:ident, $owner:ty) => {
        field_enum_choices(|object: &$owner| &object.$field)
    };
}

fn field_enum_choices<O, T: serde::de::DeserializeOwned>(_: fn(&O) -> &T) -> Vec<Value> {
    enum_unit_choices::<T>()
}

/// Ask the derived deserializer for its variant names without constructing an
/// object or depending on the spelling of Serde error messages. Payload variants
/// are excluded because they need an explicit domain-specific choice provider.
fn enum_unit_choices<T: serde::de::DeserializeOwned>() -> Vec<Value> {
    struct Variants<'a>(&'a mut &'static [&'static str]);
    impl<'de> serde::Deserializer<'de> for Variants<'_> {
        type Error = serde::de::value::Error;

        fn deserialize_any<V: serde::de::Visitor<'de>>(
            self,
            _: V,
        ) -> Result<V::Value, Self::Error> {
            Err(serde::de::Error::custom("metadata only"))
        }

        fn deserialize_enum<V: serde::de::Visitor<'de>>(
            self,
            _: &'static str,
            variants: &'static [&'static str],
            _: V,
        ) -> Result<V::Value, Self::Error> {
            *self.0 = variants;
            Err(serde::de::Error::custom("metadata only"))
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
            byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct
            map struct identifier ignored_any
        }
    }
    let mut names = &[][..];
    let _ = T::deserialize(Variants(&mut names));
    names
        .iter()
        .filter_map(|name| {
            let value = Value::String((*name).to_owned());
            serde_json::from_value::<T>(value.clone())
                .ok()
                .map(|_| value)
        })
        .collect()
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
                        {
                            let enum_choices = property_enum_choices!($field, Self);
                            let semantic = property_semantic!($($semantic)?);
                            let semantic = if semantic == PropertySemantic::Normal && !enum_choices.is_empty() {
                                PropertySemantic::Enum
                            } else {
                                semantic
                            };
                            PropertyDescriptor {
                                name: stringify!($field),
                                writable: true,
                                semantic,
                                valid_range: None,
                                enum_choices,
                            }
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
        match $obj {
            $(
                ag_iso_stack::object_pool::object::Object::$variant(v) => {
                    v.$method$args
                }
            )*
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

/// Get one declared property from an object without requiring a pool lookup.
pub fn get_object_property(object: &Object, property: &str) -> Result<Value, PropertyAccessError> {
    dispatch_property!(object, get_property, property)
}

/// Query the field's existing deserializer without changing the object.
pub(crate) fn property_accepts_null(object: &Object, property: &str) -> bool {
    dispatch_property!(&mut object.clone(), set_property, property, Value::Null).is_ok()
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

/// Rewrite property-backed references after an object's identity changes.
/// Structural reference collections are handled by the operations layer.
pub(crate) fn replace_object_reference(
    object: &mut Object,
    old_id: ObjectId,
    new_id: ObjectId,
) -> Result<bool, PropertyAccessError> {
    let mut changed = false;
    for descriptor in get_property_descriptors(object.object_type()) {
        if !matches!(
            &descriptor.semantic,
            PropertySemantic::ObjectReference { .. }
        ) {
            continue;
        }

        let value = dispatch_property!(object, get_property, descriptor.name)?;
        if value.is_null() {
            continue;
        }
        let reference_id = serde_json::from_value::<ObjectId>(value).map_err(|error| {
            PropertyAccessError::InvalidValue {
                property: descriptor.name,
                reason: error.to_string(),
            }
        })?;
        if reference_id == old_id {
            dispatch_property!(
                object,
                set_property,
                descriptor.name,
                serde_json::to_value(new_id).map_err(|error| {
                    PropertyAccessError::InvalidValue {
                        property: descriptor.name,
                        reason: error.to_string(),
                    }
                })?
            )?;
            changed = true;
        }
    }
    Ok(changed)
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
    fn typed_enum_fields_are_discovered_without_manual_semantics_or_choices() {
        #[derive(serde::Serialize, serde::Deserialize)]
        enum Mode {
            First,
            NewlyAdded,
        }
        struct Example {
            mode: Mode,
        }
        impl_properties! { Example { mode } }
        let descriptor = Example::property_descriptors().remove(0);
        assert_eq!(descriptor.semantic, PropertySemantic::Enum);
        assert!(matches!(
            default_editor(&descriptor.semantic),
            EditorKind::Enum
        ));
        assert_eq!(
            descriptor.enum_choices,
            vec![json!("First"), json!("NewlyAdded")]
        );
    }

    #[test]
    fn every_enum_property_has_a_deliberate_editor_and_valid_choices() {
        for object_type in supported_object_types() {
            let object = default_object(object_type);
            let editors = property_editor_descriptors(object_type);
            for descriptor in get_property_descriptors(object_type) {
                if descriptor.semantic != PropertySemantic::Enum {
                    continue;
                }
                let editor = editors
                    .iter()
                    .find(|editor| editor.property == descriptor.name)
                    .unwrap();
                let current = get_object_property(&object, descriptor.name).unwrap();
                if matches!(editor.editor, EditorKind::Auto) {
                    // Only explicitly overridden upstream byte fields may use numbers.
                    assert!(editor_override(object_type, descriptor.name).is_some());
                    assert!(current.is_u64());
                    assert!(descriptor.enum_choices.is_empty());
                    continue;
                }
                assert!(matches!(editor.editor, EditorKind::Enum));
                assert!(
                    descriptor.enum_choices.contains(&current),
                    "{object_type:?}.{}: missing {current}",
                    descriptor.name
                );
                for choice in descriptor.enum_choices {
                    let mut pool = ObjectPool::default();
                    pool.add(object.clone());
                    set_property(&mut pool, object.id(), descriptor.name, choice.clone()).unwrap();
                    assert_eq!(
                        get_property(&pool, object.id(), descriptor.name).unwrap(),
                        choice
                    );
                }
            }
        }
    }

    #[test]
    fn font_choices_cover_sizes_encodings_and_proprietary_types() {
        let descriptors = get_property_descriptors(ObjectType::FontAttributes);
        let sizes = &descriptors
            .iter()
            .find(|d| d.name == "font_size")
            .unwrap()
            .enum_choices;
        assert_eq!(sizes.len(), 15 + 185);
        assert!(sizes.contains(&json!({"NonProportional": "Px6x8"})));
        assert!(sizes.contains(&json!({"NonProportional": "Px128x192"})));
        assert!(sizes.contains(&json!({"Proportional": 8})));
        assert!(sizes.contains(&json!({"Proportional": 192})));
        let fonts = &descriptors
            .iter()
            .find(|d| d.name == "font_type")
            .unwrap()
            .enum_choices;
        assert_eq!(fonts.len(), 6 + 16);
        for font in ["Latin1", "Latin9", "Latin2", "Latin4", "Cyrillic", "Greek"] {
            assert!(fonts.contains(&json!(font)));
        }
        assert!(fonts.contains(&json!({"Proprietary": 240})));
        assert!(fonts.contains(&json!({"Proprietary": 255})));
        assert!(!fonts.iter().any(|font| font.get("Reserved").is_some()));
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

    #[test]
    fn every_writable_property_has_an_editor_descriptor() {
        for object_type in supported_object_types() {
            let properties = get_property_descriptors(object_type);
            let editors = property_editor_descriptors(object_type);
            assert_eq!(
                editors.len(),
                properties
                    .iter()
                    .filter(|property| property.writable)
                    .count()
                    + usize::from(supports_macro_references(object_type))
            );
            for property in properties.into_iter().filter(|property| property.writable) {
                assert!(editors
                    .iter()
                    .any(|editor| editor.property == property.name));
            }
        }
    }
}
