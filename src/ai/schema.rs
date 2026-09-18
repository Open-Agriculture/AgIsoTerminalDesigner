//! Schema DTOs generated from the same metadata and relationship rules as the executor.
use super::AiDesignState;
use crate::allowed_object_relationships::get_allowed_child_refs;
use crate::possible_events::get_possible_events;
use crate::{
    default_object,
    object_properties::{get_object_property, get_property_descriptors, PropertySemantic},
};
use ag_iso_stack::object_pool::ObjectType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSchemaSnapshot {
    pub object_type: String,
    pub properties: Vec<PropertySchemaSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour_palette: Option<Value>,
    pub supports_positioned_children: bool,
    pub allowed_positioned_children: Vec<String>,
    pub object_list: Option<ObjectListSchemaSnapshot>,
    pub supports_macro_references: bool,
    pub possible_events: Vec<String>,
    pub supports_object_labels: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertySchemaSnapshot {
    pub name: String,
    pub writable: bool,
    pub semantic: PropertySemanticSnapshot,
    pub default_value: Value,
    pub valid_range: Option<(Value, Value)>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropertySemanticSnapshot {
    Normal,
    Colour,
    Enum,
    Bitflags,
    ObjectReference { allowed_object_types: Vec<String> },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectListSchemaSnapshot {
    pub nullable: bool,
}
pub fn get_object_schema(
    state: &AiDesignState,
    object_type: ObjectType,
) -> Result<ObjectSchemaSnapshot, super::tools::AiToolError> {
    let object = default_object(object_type);
    let properties: Vec<PropertySchemaSnapshot> = get_property_descriptors(object_type)
        .into_iter()
        .map(|d| {
            let semantic = match &d.semantic {
                PropertySemantic::Normal => PropertySemanticSnapshot::Normal,
                PropertySemantic::Colour => PropertySemanticSnapshot::Colour,
                PropertySemantic::Enum => PropertySemanticSnapshot::Enum,
                PropertySemantic::Bitflags => PropertySemanticSnapshot::Bitflags,
                PropertySemantic::ObjectReference { allowed_types } => {
                    PropertySemanticSnapshot::ObjectReference {
                        allowed_object_types: allowed_types
                            .into_iter()
                            .map(|t| format!("{:?}", t))
                            .collect(),
                    }
                }
            };
            PropertySchemaSnapshot {
                name: d.name.to_owned(),
                writable: d.writable,
                semantic,
                default_value: get_object_property(&object, d.name).unwrap_or(Value::Null),
                valid_range: if matches!(d.semantic, PropertySemantic::Colour) {
                    Some((Value::from(0), Value::from(255)))
                } else {
                    d.valid_range
                },
            }
        })
        .collect();
    let allowed = get_allowed_child_refs(object_type, state.vt_version);
    let supports_positioned_children = !allowed.is_empty();
    let list = crate::operations::object_list(&object).map(|v| ObjectListSchemaSnapshot {
        nullable: matches!(v, crate::operations::ObjectReferenceList::Nullable(_)),
    });
    let events = get_possible_events(&object);
    let colour_palette = properties
        .iter()
        .any(|property| matches!(property.semantic, PropertySemanticSnapshot::Colour))
        .then(|| serde_json::json!({"encoding":"Semantic name or #RRGGBB resolves against the effective renderer palette; explicit integer indices remain exact.","lookup":"resolve_colours","source":crate::effective_colour::source(&state.pool)}));
    Ok(ObjectSchemaSnapshot {
        object_type: format!("{:?}", object_type),
        properties,
        colour_palette,
        supports_positioned_children,
        allowed_positioned_children: allowed.into_iter().map(|t| format!("{:?}", t)).collect(),
        object_list: list,
        supports_macro_references: !events.is_empty(),
        possible_events: events.into_iter().map(|e| format!("{:?}", e)).collect(),
        supports_object_labels: matches!(object_type, ObjectType::ObjectLabelReferenceList),
    })
}
