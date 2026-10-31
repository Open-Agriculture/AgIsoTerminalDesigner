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

use ag_iso_stack::object_pool::{
    object::Object,
    object_attributes::{FontType, ObjectLabel},
    vt_version::VtVersion,
    ObjectId, ObjectPool, ObjectType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A recognized property name shared by editing, validation, and serialization.
///
/// This single registry defines the enum, stable JSON spellings, parser, and
/// lexical ordering used by property maps and pagination.
macro_rules! property_keys {
    ($( $variant:ident => $spelling:literal ),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum PropertyKey {
            $(#[serde(rename = $spelling)] $variant,)*
        }

        impl PropertyKey {
            /// Return the stable JSON spelling of this property.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $spelling,)* }
            }

            /// Parse an existing serialized property spelling.
            pub fn parse(value: &str) -> Option<Self> {
                match value { $($spelling => Some(Self::$variant),)* _ => None }
            }
        }

        impl std::fmt::Display for PropertyKey {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for PropertyKey {
            type Err = UnknownPropertyKey;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value).ok_or_else(|| UnknownPropertyKey(value.to_owned()))
            }
        }

        impl PartialOrd for PropertyKey {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for PropertyKey {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.as_str().cmp(other.as_str())
            }
        }
    };
}

property_keys! {
    AcousticSignal => "acoustic_signal",
    ActiveMask => "active_mask",
    ActualHeight => "actual_height",
    ActualWidth => "actual_width",
    ArcAndTickColour => "arc_and_tick_colour",
    AuxiliaryObjectId => "auxiliary_object_id",
    BackgroundColour => "background_colour",
    BarGraphWidth => "bar_graph_width",
    BorderColour => "border_colour",
    CanvasHeight => "canvas_height",
    CanvasWidth => "canvas_width",
    CellFormat => "cell_format",
    CodePlanes => "code_planes",
    Colour => "colour",
    ColourMap => "colour_map",
    Colours => "colours",
    Commands => "commands",
    Data => "data",
    DefaultChildIndex => "default_child_index",
    DefaultObjectId => "default_object_id",
    EllipseType => "ellipse_type",
    Enabled => "enabled",
    EndAngle => "end_angle",
    ExternalObjectId => "external_object_id",
    ExternalReferenceNameId => "external_reference_name_id",
    FillAttributes => "fill_attributes",
    FillAttributesObject => "fill_attributes_object",
    FillColour => "fill_colour",
    FillPattern => "fill_pattern",
    FillType => "fill_type",
    FirstChildIndex => "first_child_index",
    FontAttributes => "font_attributes",
    FontAttributesObject => "font_attributes_object",
    FontColour => "font_colour",
    FontSize => "font_size",
    FontStyle => "font_style",
    FontType => "font_type",
    ForegroundColour => "foreground_colour",
    Format => "format",
    FunctionAttributes => "function_attributes",
    FunctionType => "function_type",
    GraphicsCursorX => "graphics_cursor_x",
    GraphicsCursorY => "graphics_cursor_y",
    Height => "height",
    Hidden => "hidden",
    IdOfColourMap => "id_of_colour_map",
    IdOfColourPalette => "id_of_colour_palette",
    InputAttributes => "input_attributes",
    InputId => "input_id",
    Justification => "justification",
    KeyCode => "key_code",
    KeyGroupIcon => "key_group_icon",
    LanguageCodes => "language_codes",
    LanguagePairs => "language_pairs",
    LastChildIndex => "last_child_index",
    LineArt => "line_art",
    LineAttributes => "line_attributes",
    LineAttributesObject => "line_attributes_object",
    LineColour => "line_colour",
    LineDirection => "line_direction",
    LineSuppression => "line_suppression",
    LineWidth => "line_width",
    MaxValue => "max_value",
    MinValue => "min_value",
    Mode => "mode",
    Name => "name",
    NeedleColour => "needle_colour",
    NrOfDecimals => "nr_of_decimals",
    NrOfTicks => "nr_of_ticks",
    Offset => "offset",
    Options => "options",
    Options2 => "options2",
    PointerType => "pointer_type",
    Points => "points",
    PolygonType => "polygon_type",
    Priority => "priority",
    RefreshInterval => "refresh_interval",
    Scale => "scale",
    ScaleType => "scale_type",
    Selectable => "selectable",
    SoftKeyMask => "soft_key_mask",
    StartAngle => "start_angle",
    TargetLineColour => "target_line_colour",
    TargetValue => "target_value",
    TargetValueVariableReference => "target_value_variable_reference",
    TransparencyColour => "transparency_colour",
    ValidationString => "validation_string",
    ValidationType => "validation_type",
    Value => "value",
    VariableReference => "variable_reference",
    ViewportHeight => "viewport_height",
    ViewportWidth => "viewport_width",
    ViewportX => "viewport_x",
    ViewportY => "viewport_y",
    ViewportZoom => "viewport_zoom",
    Width => "width",
    WindowIcon => "window_icon",
    WindowTitle => "window_title",
    WindowType => "window_type",
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPropertyKey(pub String);

impl std::fmt::Display for UnknownPropertyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unknown property key: {}", self.0)
    }
}

impl std::error::Error for UnknownPropertyKey {}

/// An editor control target. Structural controls are not ordinary properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTarget {
    Property(PropertyKey),
    MacroRefs,
}

impl EditorTarget {
    pub const fn property(self) -> Option<PropertyKey> {
        match self {
            Self::Property(key) => Some(key),
            Self::MacroRefs => None,
        }
    }
}

/// Property metadata for UI and validation
#[derive(Debug, Clone, Serialize)]
pub struct PropertyDescriptor {
    pub name: PropertyKey,
    pub writable: bool,
    pub semantic: PropertySemantic,
    pub valid_range: Option<(Value, Value)>,
    /// Serialized choices derived from the field type or ISO-defined byte domain.
    /// Contains all versions; callers apply project-version availability separately.
    pub enum_choices: Vec<Value>,
}

/// Presentation metadata for the generic property editor.
///
/// This intentionally complements `PropertyDescriptor`: validation semantics
/// stay available to all callers, while UI-only labels and widget choices are
/// declared only for object types that use the generic renderer.
#[derive(Debug, Clone, Copy)]
pub struct PropertyEditorDescriptor {
    pub property: EditorTarget,
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
    PolygonPoints,
    MacroReferences,
    /// ISO 639-1 language codes, picked from a searchable list.
    LanguageCodes,
    /// ISO 639-1 language and ISO 3166-1 country code pairs.
    LanguagePairs,
    /// A 64-bit ISO 11783-5 NAME, edited by field.
    Name,
    /// The colours of a Colour Palette, as swatches.
    PaletteColours,
    /// The entries of a Colour Map, as palette colours.
    ColourMap,
    /// Code planes and character ranges of Extended Input Attributes.
    CodePlanes,
    /// The command stream of a Macro, as readable instructions.
    MacroCommands,
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
            property: EditorTarget::Property(descriptor.name),
            editor: editor_override(object_type, descriptor.name)
                .unwrap_or_else(|| default_editor(&descriptor.semantic)),
        })
        .collect();
    if supports_macro_references(object_type) {
        descriptors.push(PropertyEditorDescriptor {
            property: EditorTarget::MacroRefs,
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
fn editor_override(object_type: ObjectType, property: PropertyKey) -> Option<EditorKind> {
    match (object_type, property) {
        (ObjectType::OutputPolygon, PropertyKey::Points) => Some(EditorKind::PolygonPoints),
        (ObjectType::WorkingSet, PropertyKey::LanguageCodes) => Some(EditorKind::LanguageCodes),
        (ObjectType::WorkingSetSpecialControls, PropertyKey::LanguagePairs) => {
            Some(EditorKind::LanguagePairs)
        }
        (
            ObjectType::ExternalObjectDefinition | ObjectType::ExternalReferenceName,
            PropertyKey::Name,
        ) => Some(EditorKind::Name),
        (ObjectType::ColourPalette, PropertyKey::Colours) => Some(EditorKind::PaletteColours),
        (ObjectType::ColourMap, PropertyKey::ColourMap) => Some(EditorKind::ColourMap),
        (ObjectType::ExtendedInputAttributes, PropertyKey::CodePlanes) => {
            Some(EditorKind::CodePlanes)
        }
        (ObjectType::Macro, PropertyKey::Commands) => Some(EditorKind::MacroCommands),
        (_, PropertyKey::Justification) => Some(EditorKind::Justification),
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    PropertyNotFound(PropertyKey),
    UnsupportedObjectType,
    InvalidValue {
        property: PropertyKey,
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
        property: PropertyKey,
    },
    UnsupportedObjectType {
        object_id: ObjectId,
        object_type: String,
    },
    InvalidValue {
        object_id: ObjectId,
        property: PropertyKey,
        reason: String,
    },
    OutOfRange {
        property: PropertyKey,
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
                property.as_str().replace('_', " ")
            ),
            Self::UnsupportedObjectType { object_type, .. } => write!(
                f,
                "Editing properties of {object_type} objects is not supported."
            ),
            Self::InvalidValue {
                property, reason, ..
            } => {
                if *property == PropertyKey::ActiveMask
                    && reason == "invalid type: null, expected u16"
                {
                    write!(
                        f,
                        "An active mask is required. Select a mask instead of clearing this value."
                    )
                } else if reason == "invalid type: null, expected u16" {
                    write!(
                        f,
                        "A value is required. Enter a whole number between 0 and 65535, or select an object if this is a reference."
                    )
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
    fn get_property(&self, property: PropertyKey) -> Result<Value, PropertyAccessError>;

    /// Set a property to a new value, returning the old value
    fn set_property(
        &mut self,
        property: PropertyKey,
        value: Value,
    ) -> Result<Value, PropertyAccessError>;

    /// Get all property descriptors for this object
    fn property_descriptors() -> Vec<PropertyDescriptor>;
}

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

macro_rules! object_label_graphic_reference {
    () => {
        PropertySemantic::ObjectReference {
            allowed_types: <ObjectLabel as crate::allowed_object_relationships::AllowedChildRefs>::get_allowed_child_refs(VtVersion::Version6),
        }
    };
}

// Unit variants come from Serde metadata. Payload enums and ISO-defined raw
// byte fields supply explicit choices.
macro_rules! property_enum_choices {
    ($field:ident, $owner:ty, $choices:expr) => {
        $choices
    };
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
    (font_type, $owner:ty) => {
        // Font Type support is monotonic: VT6 includes the complete defined domain.
        (0..=u8::MAX)
            .map(FontType::from)
            .filter(|font| crate::vt_compatibility::supported_font_type(*font, VtVersion::Version6))
            .map(|font| serde_json::to_value(font).unwrap())
            .collect()
    };
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

/// Macro to generate PropertyAccess implementations for object types.
/// Maps typed keys to upstream fields with optional semantic metadata; Serde handles Rust types.
///
/// Example:
/// ```ignore
/// impl_properties! {
///     Button {
///         Width: width,
///         Height: height,
///         BackgroundColour: background_colour => PropertySemantic::Colour,
///         BorderColour: border_colour => PropertySemantic::Colour,
///         KeyCode: key_code,
///     }
/// }
/// ```
macro_rules! impl_properties {
    (
        $type:ty {
            $( $key:ident : $field:ident $(=> $semantic:expr)? $(; choices $choices:expr)? ),* $(,)?
        }
    ) => {
        impl PropertyAccess for $type {
            fn get_property(&self, property: PropertyKey) -> Result<Value, PropertyAccessError> {
                match property {
                    $(
                        PropertyKey::$key => {
                            serde_json::to_value(&self.$field)
                                .map_err(|e| PropertyAccessError::InvalidValue {
                                    property: PropertyKey::$key,
                                    reason: e.to_string(),
                                })
                        }
                    )*
                    _ => Err(PropertyAccessError::PropertyNotFound(property)),
                }
            }

            #[allow(unused_variables)]
            fn set_property(&mut self, property: PropertyKey, value: Value) -> Result<Value, PropertyAccessError> {
                match property {
                    $(
                        PropertyKey::$key => {
                            let old = serde_json::to_value(&self.$field)
                                .map_err(|e| PropertyAccessError::InvalidValue {
                                    property: PropertyKey::$key,
                                    reason: e.to_string(),
                                })?;

                            self.$field = serde_json::from_value(value)
                                .map_err(|e| PropertyAccessError::InvalidValue {
                                    property: PropertyKey::$key,
                                    reason: e.to_string(),
                                })?;

                            Ok(old)
                        }
                    )*
                    _ => Err(PropertyAccessError::PropertyNotFound(property)),
                }
            }

            fn property_descriptors() -> Vec<PropertyDescriptor> {
                vec![
                    $(
                        {
                            let enum_choices: Vec<Value> = property_enum_choices!($field, Self $(, $choices)?);
                            let semantic = property_semantic!($($semantic)?);
                            let semantic = if semantic == PropertySemantic::Normal && !enum_choices.is_empty() {
                                PropertySemantic::Enum
                            } else {
                                semantic
                            };
                            PropertyDescriptor {
                                name: PropertyKey::$key,
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
    BackgroundColour: background_colour => PropertySemantic::Colour,
    Selectable: selectable,
    ActiveMask: active_mask => object_reference!(DataMask, AlarmMask),
    LanguageCodes: language_codes
}}

impl_properties! {
ag_iso_stack::object_pool::object::DataMask {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    SoftKeyMask: soft_key_mask => object_reference!(SoftKeyMask)
}}

impl_properties! {
ag_iso_stack::object_pool::object::AlarmMask {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    SoftKeyMask: soft_key_mask => object_reference!(SoftKeyMask),
    Priority: priority,
    AcousticSignal: acoustic_signal
}}

impl_properties! {
ag_iso_stack::object_pool::object::Container {
    Width: width,
    Height: height,
    Hidden: hidden
}}

impl_properties! {
ag_iso_stack::object_pool::object::SoftKeyMask {
    BackgroundColour: background_colour => PropertySemantic::Colour
}}

impl_properties! {
ag_iso_stack::object_pool::object::Key {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    KeyCode: key_code
}}

impl_properties! {
ag_iso_stack::object_pool::object::Button {
    Width: width,
    Height: height,
    BackgroundColour: background_colour => PropertySemantic::Colour,
    BorderColour: border_colour => PropertySemantic::Colour,
    KeyCode: key_code,
    Options: options => PropertySemantic::Bitflags
}}

impl_properties! {
ag_iso_stack::object_pool::object::InputBoolean {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    Width: width,
    ForegroundColour: foreground_colour => object_reference!(FontAttributes),
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value,
    Enabled: enabled
}}

impl_properties! {
ag_iso_stack::object_pool::object::InputString {
    Width: width,
    Height: height,
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FontAttributes: font_attributes => object_reference!(FontAttributes),
    InputAttributes: input_attributes => object_reference!(InputAttributes, ExtendedInputAttributes),
    Options: options => PropertySemantic::Bitflags,
    VariableReference: variable_reference => object_reference!(StringVariable),
    Value: value,
    Justification: justification,
    Enabled: enabled
}}

impl_properties! {
ag_iso_stack::object_pool::object::InputNumber {
    Width: width,
    Height: height,
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FontAttributes: font_attributes => object_reference!(FontAttributes),
    Options: options => PropertySemantic::Bitflags,
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value,
    MinValue: min_value,
    MaxValue: max_value,
    Offset: offset,
    Scale: scale,
    NrOfDecimals: nr_of_decimals,
    Format: format => PropertySemantic::Enum,
    Justification: justification,
    Options2: options2 => PropertySemantic::Bitflags
}}

impl_properties! {
ag_iso_stack::object_pool::object::InputList {
    Width: width,
    Height: height,
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value,
    Options: options => PropertySemantic::Bitflags
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputString {
    Width: width,
    Height: height,
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FontAttributes: font_attributes => object_reference!(FontAttributes),
    Options: options => PropertySemantic::Bitflags,
    VariableReference: variable_reference => object_reference!(StringVariable),
    Justification: justification,
    Value: value
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputNumber {
    Width: width,
    Height: height,
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FontAttributes: font_attributes => object_reference!(FontAttributes),
    Options: options => PropertySemantic::Bitflags,
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value,
    Offset: offset,
    Scale: scale,
    NrOfDecimals: nr_of_decimals,
    Format: format => PropertySemantic::Enum,
    Justification: justification
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputList {
    Width: width,
    Height: height,
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputLine {
    LineAttributes: line_attributes => object_reference!(LineAttributes),
    Width: width,
    Height: height,
    LineDirection: line_direction => PropertySemantic::Enum
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputRectangle {
    LineAttributes: line_attributes => object_reference!(LineAttributes),
    Width: width,
    Height: height,
    LineSuppression: line_suppression => PropertySemantic::Bitflags,
    FillAttributes: fill_attributes => object_reference!(FillAttributes)
}}

// ISO 11783-6:2018, Table B.31: the type field is an enumerated byte.
impl_properties! {
ag_iso_stack::object_pool::object::OutputEllipse {
    LineAttributes: line_attributes => object_reference!(LineAttributes),
    Width: width,
    Height: height,
    EllipseType: ellipse_type => PropertySemantic::Enum; choices (0..=3).map(Value::from).collect(),
    StartAngle: start_angle,
    EndAngle: end_angle,
    FillAttributes: fill_attributes => object_reference!(FillAttributes)
}}

// ISO 11783-6:2018, Table B.33: the type field is an enumerated byte.
impl_properties! {
ag_iso_stack::object_pool::object::OutputPolygon {
    Width: width,
    Height: height,
    LineAttributes: line_attributes => object_reference!(LineAttributes),
    FillAttributes: fill_attributes => object_reference!(FillAttributes),
    PolygonType: polygon_type => PropertySemantic::Enum; choices (0..=3).map(Value::from).collect(),
    Points: points
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputMeter {
    Width: width,
    NeedleColour: needle_colour => PropertySemantic::Colour,
    BorderColour: border_colour => PropertySemantic::Colour,
    ArcAndTickColour: arc_and_tick_colour => PropertySemantic::Colour,
    Options: options => PropertySemantic::Bitflags,
    NrOfTicks: nr_of_ticks,
    StartAngle: start_angle,
    EndAngle: end_angle,
    MinValue: min_value,
    MaxValue: max_value,
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputLinearBarGraph {
    Width: width,
    Height: height,
    Colour: colour => PropertySemantic::Colour,
    TargetLineColour: target_line_colour => PropertySemantic::Colour,
    Options: options => PropertySemantic::Bitflags,
    NrOfTicks: nr_of_ticks,
    MinValue: min_value,
    MaxValue: max_value,
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value,
    TargetValueVariableReference: target_value_variable_reference => object_reference!(NumberVariable),
    TargetValue: target_value
}}

impl_properties! {
ag_iso_stack::object_pool::object::OutputArchedBarGraph {
    Width: width,
    Height: height,
    Colour: colour => PropertySemantic::Colour,
    TargetLineColour: target_line_colour => PropertySemantic::Colour,
    Options: options => PropertySemantic::Bitflags,
    StartAngle: start_angle,
    EndAngle: end_angle,
    BarGraphWidth: bar_graph_width,
    MinValue: min_value,
    MaxValue: max_value,
    VariableReference: variable_reference => object_reference!(NumberVariable),
    Value: value,
    TargetValueVariableReference: target_value_variable_reference => object_reference!(NumberVariable),
    TargetValue: target_value
}}

impl_properties! {
ag_iso_stack::object_pool::object::PictureGraphic {
    Width: width,
    ActualWidth: actual_width,
    ActualHeight: actual_height,
    Format: format => PropertySemantic::Enum,
    Options: options => PropertySemantic::Bitflags,
    TransparencyColour: transparency_colour => PropertySemantic::Colour,
    Data: data
}}

impl_properties! {
ag_iso_stack::object_pool::object::NumberVariable { Value: value }}

impl_properties! {
ag_iso_stack::object_pool::object::StringVariable { Value: value }}

impl_properties! {
ag_iso_stack::object_pool::object::FontAttributes {
    FontColour: font_colour => PropertySemantic::Colour,
    FontSize: font_size => PropertySemantic::Enum,
    FontType: font_type => PropertySemantic::Enum,
    FontStyle: font_style => PropertySemantic::Bitflags
}}

impl_properties! {
ag_iso_stack::object_pool::object::LineAttributes {
    LineColour: line_colour => PropertySemantic::Colour,
    LineWidth: line_width,
    LineArt: line_art => PropertySemantic::Bitflags
}}

// ISO 11783-6:2018, Table B.50: the type field is an enumerated byte.
impl_properties! {
ag_iso_stack::object_pool::object::FillAttributes {
    FillType: fill_type => PropertySemantic::Enum; choices (0..=3).map(Value::from).collect(),
    FillColour: fill_colour => PropertySemantic::Colour,
    FillPattern: fill_pattern => object_reference!(PictureGraphic)
}}

impl_properties! {
ag_iso_stack::object_pool::object::InputAttributes {
    ValidationType: validation_type => PropertySemantic::Enum,
    ValidationString: validation_string
}}

impl_properties! {
ag_iso_stack::object_pool::object::ObjectPointer {
    Value: value => object_reference!()
}}

impl_properties! {
ag_iso_stack::object_pool::object::Macro { Commands: commands }}

// ISO 11783-6:2018, Table J.1: the type field is an enumerated byte.
impl_properties! {
ag_iso_stack::object_pool::object::AuxiliaryFunctionType1 {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FunctionType: function_type => PropertySemantic::Enum; choices (0..=2).map(Value::from).collect()
}}

// ISO 11783-6:2018, Table J.3: the type field is an enumerated byte.
impl_properties! {
ag_iso_stack::object_pool::object::AuxiliaryInputType1 {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FunctionType: function_type => PropertySemantic::Enum; choices (0..=2).map(Value::from).collect(),
    InputId: input_id
}}

impl_properties! {
ag_iso_stack::object_pool::object::AuxiliaryFunctionType2 {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FunctionAttributes: function_attributes => PropertySemantic::Bitflags
}}

impl_properties! {
ag_iso_stack::object_pool::object::AuxiliaryInputType2 {
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FunctionAttributes: function_attributes => PropertySemantic::Bitflags
}}

// ISO 11783-6:2018, Table J.6: the type field is an enumerated byte.
impl_properties! {
ag_iso_stack::object_pool::object::AuxiliaryControlDesignatorType2 {
    PointerType: pointer_type => PropertySemantic::Enum; choices (0..=3).map(Value::from).collect(),
    AuxiliaryObjectId: auxiliary_object_id => object_reference!(AuxiliaryFunctionType2, AuxiliaryInputType2)
}}

impl_properties! {
ag_iso_stack::object_pool::object::WindowMask {
    CellFormat: cell_format => PropertySemantic::Enum,
    WindowType: window_type => PropertySemantic::Enum,
    BackgroundColour: background_colour => PropertySemantic::Colour,
    Options: options => PropertySemantic::Bitflags,
    Name: name => object_reference!(OutputString, ObjectPointer),
    WindowTitle: window_title => object_reference!(OutputString, ObjectPointer),
    WindowIcon: window_icon => object_label_graphic_reference!()
}}

impl_properties! {
ag_iso_stack::object_pool::object::KeyGroup {
    Options: options => PropertySemantic::Bitflags,
    Name: name => object_reference!(OutputString, ObjectPointer),
    KeyGroupIcon: key_group_icon => object_label_graphic_reference!()
}}

impl_properties! {
ag_iso_stack::object_pool::object::GraphicsContext {
    ViewportWidth: viewport_width,
    ViewportHeight: viewport_height,
    ViewportX: viewport_x,
    ViewportY: viewport_y,
    CanvasWidth: canvas_width,
    CanvasHeight: canvas_height,
    ViewportZoom: viewport_zoom,
    GraphicsCursorX: graphics_cursor_x,
    GraphicsCursorY: graphics_cursor_y,
    ForegroundColour: foreground_colour => PropertySemantic::Colour,
    BackgroundColour: background_colour => PropertySemantic::Colour,
    FontAttributesObject: font_attributes_object => object_reference!(FontAttributes),
    LineAttributesObject: line_attributes_object => object_reference!(LineAttributes),
    FillAttributesObject: fill_attributes_object => object_reference!(FillAttributes),
    Format: format => PropertySemantic::Enum,
    Options: options => PropertySemantic::Bitflags,
    TransparencyColour: transparency_colour => PropertySemantic::Colour
}}

impl_properties! {
ag_iso_stack::object_pool::object::ExtendedInputAttributes {
    ValidationType: validation_type => PropertySemantic::Enum,
    CodePlanes: code_planes
}}

impl_properties! {
ag_iso_stack::object_pool::object::ColourMap { ColourMap: colour_map }}

impl_properties! {
ag_iso_stack::object_pool::object::ObjectLabelReferenceList {}}

impl_properties! {
ag_iso_stack::object_pool::object::ExternalObjectDefinition {
    Options: options => PropertySemantic::Bitflags,
    Name: name
}}

impl_properties! {
ag_iso_stack::object_pool::object::ExternalReferenceName {
    Options: options => PropertySemantic::Bitflags,
    Name: name
}}

impl_properties! {
ag_iso_stack::object_pool::object::ExternalObjectPointer {
    DefaultObjectId: default_object_id => object_reference!(),
    ExternalReferenceNameId: external_reference_name_id => object_reference!(ExternalReferenceName),
    // Numeric identity in the external ECU's pool, not a local picker target.
    ExternalObjectId: external_object_id
}}

impl_properties! {
ag_iso_stack::object_pool::object::Animation {
    Width: width,
    Height: height,
    RefreshInterval: refresh_interval,
    Value: value,
    Enabled: enabled,
    FirstChildIndex: first_child_index,
    LastChildIndex: last_child_index,
    DefaultChildIndex: default_child_index,
    Options: options => PropertySemantic::Bitflags
}}

impl_properties! {
ag_iso_stack::object_pool::object::ColourPalette {
    Options: options => PropertySemantic::Bitflags,
    Colours: colours
}}

impl_properties! {
ag_iso_stack::object_pool::object::GraphicData {
    // ISO 11783-6:2018, Table B.74: PNG is the only defined format.
    Format: format => PropertySemantic::Enum; choices vec![Value::from(0)],
    Data: data
}}

impl_properties! {
ag_iso_stack::object_pool::object::WorkingSetSpecialControls {
    IdOfColourMap: id_of_colour_map => object_reference!(ColourMap),
    IdOfColourPalette: id_of_colour_palette => object_reference!(ColourPalette),
    LanguagePairs: language_pairs
}}

impl_properties! {
ag_iso_stack::object_pool::object::ScaledGraphic {
    Width: width,
    Height: height,
    // Table B.76: scaling 0–4, horizontal/vertical justification 0–2.
    ScaleType: scale_type => PropertySemantic::Enum; choices (0..=2)
        .flat_map(|vertical| (0..=2).flat_map(move |horizontal|
            (0..=4).map(move |scaling| Value::from(scaling | (horizontal << 3) | (vertical << 5)))))
        .collect(),
    Options: options => PropertySemantic::Bitflags,
    Value: value => object_reference!(PictureGraphic, GraphicData, ObjectPointer)
}}

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

/// Get a recognized property from an object in the pool.
///
/// A recognized key can still be unsupported for the object's type; that
/// returns [`PropertyError::PropertyNotFound`]. Existing upstream values are
/// serialized as stored so callers can inspect and repair legacy values.
pub fn get_property(
    pool: &ObjectPool,
    object_id: ObjectId,
    property: PropertyKey,
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
            property,
            reason,
        },
    })
}

/// Get a recognized, declared property without requiring a pool lookup.
///
/// A recognized key unsupported by this object's type returns
/// [`PropertyAccessError::PropertyNotFound`]. Values are read as stored, even
/// when they need repair by the editor.
pub fn get_object_property(
    object: &Object,
    property: PropertyKey,
) -> Result<Value, PropertyAccessError> {
    dispatch_property!(object, get_property, property)
}

/// Query the field's existing deserializer without changing the object.
pub(crate) fn property_accepts_null(object: &Object, property: PropertyKey) -> bool {
    dispatch_property!(&mut object.clone(), set_property, property, Value::Null).is_ok()
}

/// Set a recognized property on an object in the pool.
///
/// The key must be supported by the object's type and the value must deserialize
/// to the upstream field type. Editor-level constraints are checked separately
/// by [`validate_property`], allowing existing recognized values to be repaired.
pub fn set_property(
    pool: &mut ObjectPool,
    object_id: ObjectId,
    property: PropertyKey,
    value: Value,
) -> Result<Value, PropertyError> {
    let obj = pool
        .object_mut_by_id(object_id)
        .ok_or(PropertyError::ObjectNotFound(object_id))?;

    // Older authoring data represented Boolean values as JSON booleans.
    // Accept that form while retaining the full VT4+ byte value range.
    let value = if matches!(obj, Object::InputBoolean(_)) && property == PropertyKey::Value {
        value
            .as_bool()
            .map(|value| Value::from(u8::from(value)))
            .unwrap_or(value)
    } else {
        value
    };

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
            property,
            reason,
        },
    })
}

fn property_descriptors_for_type(object_type: ObjectType) -> Vec<PropertyDescriptor> {
    supported_property_objects!(property_descriptors_impl, object_type)
}

/// Return property metadata shared by editor controls and assignment validation.
/// Numeric field ranges also describe constraints checked during complete-pool validation.
pub fn get_property_descriptors(object_type: ObjectType) -> Vec<PropertyDescriptor> {
    let mut descriptors = property_descriptors_for_type(object_type);
    if matches!(
        object_type,
        ObjectType::InputNumber | ObjectType::OutputNumber
    ) {
        if let Some(descriptor) = descriptors
            .iter_mut()
            .find(|descriptor| descriptor.name == PropertyKey::NrOfDecimals)
        {
            descriptor.valid_range = Some((Value::from(0), Value::from(7)));
        }
    }
    let bounded_properties: &[(PropertyKey, i64, i64)] = match object_type {
        ObjectType::OutputEllipse | ObjectType::OutputMeter | ObjectType::OutputArchedBarGraph => {
            &[
                (PropertyKey::StartAngle, 0, 180),
                (PropertyKey::EndAngle, 0, 180),
            ]
        }
        ObjectType::GraphicsContext => &[
            (PropertyKey::ViewportWidth, 0, 32767),
            (PropertyKey::ViewportHeight, 0, 32767),
            (PropertyKey::CanvasWidth, 0, 32767),
            (PropertyKey::CanvasHeight, 0, 32767),
        ],
        _ => &[],
    };
    for (property, minimum, maximum) in bounded_properties {
        if let Some(descriptor) = descriptors
            .iter_mut()
            .find(|descriptor| descriptor.name == *property)
        {
            descriptor.valid_range = Some((Value::from(*minimum), Value::from(*maximum)));
        }
    }
    descriptors
}

/// Whether materialization must preserve an imported component value so the
/// user can repair it instead of making the existing project unopenable.
pub(crate) fn property_has_repairable_iso_domain(
    object_type: ObjectType,
    property: PropertyKey,
) -> bool {
    get_property_descriptors(object_type)
        .into_iter()
        .any(|descriptor| descriptor.name == property && descriptor.valid_range.is_some())
        || matches!(
            (object_type, property),
            (ObjectType::ExtendedInputAttributes, PropertyKey::CodePlanes)
                | (ObjectType::WorkingSet, PropertyKey::LanguageCodes)
                | (
                    ObjectType::WorkingSetSpecialControls,
                    PropertyKey::LanguagePairs
                )
                | (ObjectType::NumberVariable, PropertyKey::Value)
        )
}

/// Remap all property-backed object references simultaneously. Structural
/// reference collections are handled by the operations layer.
pub(crate) fn remap_object_references(
    object: &mut Object,
    remap: &std::collections::HashMap<ObjectId, ObjectId>,
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
        if let Some(new_id) = remap.get(&reference_id) {
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
/// Get every non-identity, non-placement property exposed by an object.
///
/// Returned keys use their typed property identity.
pub fn get_properties(
    pool: &ObjectPool,
    object_id: ObjectId,
) -> Result<HashMap<PropertyKey, Value>, PropertyError> {
    let object = pool
        .object_by_id(object_id)
        .ok_or(PropertyError::ObjectNotFound(object_id))?;
    get_property_descriptors(object.object_type())
        .into_iter()
        .map(|descriptor| {
            get_property(pool, object_id, descriptor.name).map(|value| (descriptor.name, value))
        })
        .collect()
}

fn property_access_supported(object_type: ObjectType) -> bool {
    supported_property_objects!(property_access_supported_impl, object_type)
}

/// Validate a proposed value for a recognized property without mutating the pool.
///
/// A recognized key unsupported by the object's type returns
/// [`PropertyError::PropertyNotFound`]. This checks a candidate edit; it does not
/// discard or rewrite an existing value that needs repair.
/// Target-version availability is checked by authoring operations, separately
/// from this domain check. Loading and history use [`set_property`] directly.
pub fn validate_property(
    pool: &ObjectPool,
    object_id: ObjectId,
    property: PropertyKey,
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
            property,
        })?;

    if !descriptor.writable {
        return Err(PropertyError::InvalidValue {
            object_id,
            property,
            reason: "property is read-only".to_string(),
        });
    }

    // Byte enums and Font Type payloads can deserialize outside their defined domain.
    if descriptor.semantic == PropertySemantic::Enum
        && (descriptor.enum_choices.iter().any(Value::is_number)
            || (object.object_type() == ObjectType::FontAttributes
                && property == PropertyKey::FontType))
        && !descriptor.enum_choices.contains(value)
    {
        return Err(PropertyError::InvalidValue {
            object_id,
            property,
            reason: "value is outside the defined enum choices".into(),
        });
    }

    if let Some((min, max)) = descriptor.valid_range {
        let number = value.as_f64();
        let minimum = min.as_f64();
        let maximum = max.as_f64();
        if !matches!((number, minimum, maximum), (Some(value), Some(min), Some(max)) if value >= min && value <= max)
        {
            return Err(PropertyError::OutOfRange {
                property,
                value: value.to_string(),
                min: min.to_string(),
                max: max.to_string(),
            });
        }
    }

    validate_iso_property_payload(pool, object, property, value)?;

    match descriptor.semantic {
        PropertySemantic::Colour => {
            if !matches!(value.as_u64(), Some(0..=255)) {
                return Err(PropertyError::InvalidValue {
                    object_id,
                    property,
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
                        property,
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

pub(crate) fn validate_iso_property_payload(
    pool: &ObjectPool,
    object: &Object,
    property: PropertyKey,
    value: &Value,
) -> Result<(), PropertyError> {
    let invalid = |reason: String| PropertyError::InvalidValue {
        object_id: object.id(),
        property,
        reason,
    };
    match (object, property) {
        (Object::ExtendedInputAttributes(_), PropertyKey::CodePlanes) => {
            let planes: Vec<ag_iso_stack::object_pool::object::CodePlane> =
                serde_json::from_value(value.clone())
                    .map_err(|error| invalid(error.to_string()))?;
            if !(1..=17).contains(&planes.len()) {
                return Err(invalid(format!(
                    "code planes contain {}; expected 1 through 17",
                    planes.len()
                )));
            }
            for plane in planes {
                if plane.number > 16 {
                    return Err(invalid(format!(
                        "code plane {} is outside 0 through 16",
                        plane.number
                    )));
                }
                if !(1..=255).contains(&plane.character_ranges.len()) {
                    return Err(invalid(format!(
                        "code plane {} contains {} character ranges; expected 1 through 255",
                        plane.number,
                        plane.character_ranges.len()
                    )));
                }
                if let Some(range) = plane
                    .character_ranges
                    .iter()
                    .find(|range| range.first_character > range.last_character)
                {
                    return Err(invalid(format!(
                        "code plane {} has reversed character range {} through {}",
                        plane.number, range.first_character, range.last_character
                    )));
                }
            }
        }
        (Object::WorkingSet(_), PropertyKey::LanguageCodes) => {
            let codes: Vec<String> = serde_json::from_value(value.clone())
                .map_err(|error| invalid(error.to_string()))?;
            if let Some(code) = codes.iter().find(|code| !two_ascii_letters(code)) {
                return Err(invalid(format!(
                    "language code {code:?} must contain exactly two ASCII letters"
                )));
            }
        }
        (Object::WorkingSetSpecialControls(_), PropertyKey::LanguagePairs) => {
            let pairs: Vec<(String, String)> = serde_json::from_value(value.clone())
                .map_err(|error| invalid(error.to_string()))?;
            if let Some((language, country)) = pairs.iter().find(|(language, country)| {
                !two_ascii_letters(language) || !(two_ascii_letters(country) || country == "  ")
            }) {
                return Err(invalid(format!("language/country pair ({language:?}, {country:?}) must use two ASCII letters per code; an unspecified country is two spaces")));
            }
        }
        (Object::NumberVariable(_), PropertyKey::Value) => {
            let Some(new_value) = value.as_u64().and_then(|value| u32::try_from(value).ok()) else {
                return Err(invalid(
                    "Number Variable value must be an unsigned 32-bit integer".into(),
                ));
            };
            if new_value > u16::MAX as u32 && number_variable_has_16_bit_consumer(pool, object.id())
            {
                return Err(invalid(format!("Number Variable value {new_value} referenced by Output Meter or bar graphs exceeds 65535")));
            }
        }
        (Object::OutputMeter(_), PropertyKey::VariableReference)
        | (Object::OutputLinearBarGraph(_), PropertyKey::VariableReference)
        | (Object::OutputArchedBarGraph(_), PropertyKey::VariableReference)
        | (Object::OutputLinearBarGraph(_), PropertyKey::TargetValueVariableReference)
        | (Object::OutputArchedBarGraph(_), PropertyKey::TargetValueVariableReference) => {
            if !value.is_null() {
                let id = serde_json::from_value::<ObjectId>(value.clone())
                    .map_err(|error| invalid(error.to_string()))?;
                if let Some(Object::NumberVariable(variable)) = pool.object_by_id(id) {
                    if variable.value > u16::MAX as u32 {
                        return Err(invalid(format!("Number Variable value {} referenced by Output Meter or bar graphs exceeds 65535", variable.value)));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn two_ascii_letters(value: &str) -> bool {
    value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_alphabetic())
}

fn number_variable_has_16_bit_consumer(pool: &ObjectPool, id: ObjectId) -> bool {
    pool.objects().iter().any(|object| match object {
        Object::OutputMeter(object) => object.variable_reference.0 == Some(id),
        Object::OutputLinearBarGraph(object) => {
            object.variable_reference.0 == Some(id)
                || object.target_value_variable_reference.0 == Some(id)
        }
        Object::OutputArchedBarGraph(object) => {
            object.variable_reference.0 == Some(id)
                || object.target_value_variable_reference.0 == Some(id)
        }
        _ => false,
    })
}

/// Array element prototypes shared by command insertion and schema discovery.
/// The empty path denotes the property itself; `*` denotes an array index.
/// Record prototypes list required scalar fields while nested arrays start empty.
pub fn property_array_element_schemas(
    property: PropertyKey,
) -> std::collections::BTreeMap<&'static str, Value> {
    let mut schemas = std::collections::BTreeMap::new();
    let element = match property {
        PropertyKey::Points => serde_json::json!({"x":0,"y":0}),
        PropertyKey::LanguageCodes => serde_json::json!(""),
        PropertyKey::ColourMap | PropertyKey::Data => serde_json::json!(0u8),
        PropertyKey::Colours => serde_json::to_value(ag_iso_stack::object_pool::Colour::BLACK)
            .expect("colours serialize"),
        PropertyKey::CodePlanes => {
            schemas.insert(
                "*.character_ranges",
                serde_json::json!({"first_character":0u16,"last_character":0u16}),
            );
            serde_json::json!({"number":0u8,"character_ranges":[]})
        }
        PropertyKey::LanguagePairs => serde_json::json!(["", ""]),
        _ => return schemas,
    };
    schemas.insert("", element);
    schemas
}

/// Return the typed element prototype for a top-level array property.
pub fn property_array_element_schema(property: PropertyKey) -> Option<Value> {
    property_array_element_schema_at(property, "")
}

/// Return the element prototype of a nested array, using zero-based indices.
pub fn property_array_element_schema_at(property: PropertyKey, path: &str) -> Option<Value> {
    let normalized = path
        .split('.')
        .map(|part| {
            if part.parse::<usize>().is_ok() {
                "*"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(".");
    property_array_element_schemas(property).remove(normalized.as_str())
}
