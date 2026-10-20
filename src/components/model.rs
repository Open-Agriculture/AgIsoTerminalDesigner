//! Project-local source identities. None of these are ISO object types.
use crate::object_properties::PropertyKey;
use crate::ObjectTarget;
use ag_iso_stack::object_pool::ObjectType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

macro_rules! identity {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub uuid::Uuid);
        impl Default for $name {
            fn default() -> Self {
                Self(uuid::Uuid::new_v4())
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
identity!(ComponentId);
identity!(ComponentNodeId);
identity!(ComponentInstanceId);
identity!(ComponentParameterId);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentDefinition {
    pub schema_version: u32,
    pub id: ComponentId,
    pub name: String,
    pub description: String,
    pub category_path: Vec<String>,
    pub tags: Vec<String>,
    pub revision: u64,
    pub root_node: ComponentNodeId,
    pub nodes: Vec<ComponentNode>,
    pub parameters: Vec<ComponentParameter>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentNode {
    pub id: ComponentNodeId,
    pub name: String,
    pub object_type: ObjectType,
    pub properties: BTreeMap<PropertyKey, Value>,
    pub children: Vec<ComponentChild>,
    pub references: BTreeMap<PropertyKey, ComponentReference>,
    #[serde(default)]
    pub macro_references: Vec<ComponentMacroReference>,
    #[serde(default)]
    pub object_list: Option<ComponentObjectList>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentChild {
    pub child: ComponentNodeId,
    pub x: i32,
    pub y: i32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComponentReference {
    Internal(ComponentNodeId),
    Parameter(ComponentParameterId),
    ExternalObject(ObjectTarget),
    Null,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentParameterKind {
    Text,
    Number,
    Boolean,
    ObjectReference,
    Colour,
    Enum,
    Graphic,
}
/// A parameter targets an ordinary writable object property or a picture graphic.
/// The graphic target is structural and is not an ISO object property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComponentParameterTarget {
    Property(PropertyKey),
    Graphic,
}
impl ComponentParameterTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Property(property) => property.as_str(),
            Self::Graphic => "graphic",
        }
    }
}
impl Serialize for ComponentParameterTarget {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for ComponentParameterTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        if name == "graphic" {
            return Ok(Self::Graphic);
        }
        PropertyKey::parse(&name)
            .map(Self::Property)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown component property '{name}'")))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentParameter {
    pub id: ComponentParameterId,
    pub name: String,
    pub description: Option<String>,
    pub target_node: ComponentNodeId,
    pub property: ComponentParameterTarget,
    pub kind: ComponentParameterKind,
    pub default_value: Value,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EditorParentRef {
    Primitive(u16),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentInstance {
    pub id: ComponentInstanceId,
    pub component_id: ComponentId,
    /// Stable authoring name used by the CLI and project references.
    #[serde(default)]
    pub name: String,
    pub parent: EditorParentRef,
    /// Absolute display-list position, independent of VT ID and instance UUID.
    #[serde(default)]
    pub child_index: Option<usize>,
    pub x: i32,
    pub y: i32,
    pub overrides: BTreeMap<ComponentParameterId, Value>,
    pub compiled_ids: BTreeMap<ComponentNodeId, u16>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentSource {
    pub definitions: BTreeMap<ComponentId, ComponentDefinition>,
    pub instances: Vec<ComponentInstance>,
}

/// Generate a deterministic, collection-unique display name for an instance.
pub fn unique_instance_name(source: &ComponentSource, component_id: ComponentId) -> String {
    let base = source
        .definitions
        .get(&component_id)
        .map(|definition| definition.name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or("Component");
    let used: std::collections::HashSet<_> = source
        .instances
        .iter()
        .map(|instance| crate::smart_naming::normalized_name(&instance.name))
        .collect();
    let mut suffix = 1usize;
    loop {
        let candidate = format!("{base} {suffix}");
        if !used.contains(&crate::smart_naming::normalized_name(&candidate)) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Fill names missing from older project files without changing authored names.
/// Generated values are deterministic and unique against names seen so far.
pub fn ensure_instance_names(source: &mut ComponentSource) {
    for index in 0..source.instances.len() {
        if !crate::smart_naming::normalized_name(&source.instances[index].name).is_empty() {
            continue;
        }
        let component_id = source.instances[index].component_id;
        let candidate = unique_instance_name(source, component_id);
        source.instances[index].name = candidate;
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorSelection {
    PrimitiveObject(u16),
    ComponentInstance(ComponentInstanceId),
    ComponentNode {
        instance_id: ComponentInstanceId,
        node_id: ComponentNodeId,
    },
    ComponentDefinitionNode {
        component_id: ComponentId,
        node_id: ComponentNodeId,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterializedSource {
    Primitive {
        object_id: u16,
    },
    ComponentNode {
        instance_id: ComponentInstanceId,
        node_id: ComponentNodeId,
    },
}
#[derive(Debug, Clone, Default)]
pub struct MaterializedSourceMap {
    pub by_object_id: BTreeMap<u16, MaterializedSource>,
    pub by_component_node: BTreeMap<(ComponentInstanceId, ComponentNodeId), u16>,
}
#[derive(Debug, Clone)]
pub struct MaterializedProject {
    pub pool: ag_iso_stack::object_pool::ObjectPool,
    pub source_map: MaterializedSourceMap,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentError {
    pub code: String,
    pub message: String,
    pub component_id: Option<ComponentId>,
    pub instance_id: Option<ComponentInstanceId>,
    pub node_id: Option<ComponentNodeId>,
    pub property: Option<String>,
    pub object_id: Option<u16>,
}
impl ComponentError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            component_id: None,
            instance_id: None,
            node_id: None,
            property: None,
            object_id: None,
        }
    }
}
impl std::fmt::Display for ComponentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(id) = self.instance_id {
            write!(f, " (instance {id}")?;
            if let Some(node) = self.node_id {
                write!(f, ", node {node}")?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}
impl std::error::Error for ComponentError {}
pub type ComponentResult<T> = Result<T, ComponentError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentMacroReference {
    pub event: ag_iso_stack::object_pool::object_attributes::Event,
    pub target: ObjectTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComponentObjectList {
    Required(Vec<ObjectTarget>),
    Nullable(Vec<Option<ObjectTarget>>),
}

#[cfg(test)]
mod instance_name_tests {
    use super::*;

    #[test]
    fn missing_legacy_instance_names_are_generated_uniquely() {
        let component_id = ComponentId::default();
        let node_id = ComponentNodeId::default();
        let definition = ComponentDefinition {
            schema_version: 1,
            id: component_id,
            name: "Start Button".into(),
            description: String::new(),
            category_path: vec![],
            tags: vec![],
            revision: 1,
            root_node: node_id,
            nodes: vec![],
            parameters: vec![],
        };
        let make_instance = || ComponentInstance {
            id: ComponentInstanceId::default(),
            component_id,
            name: String::new(),
            parent: EditorParentRef::Primitive(0),
            child_index: None,
            x: 0,
            y: 0,
            overrides: BTreeMap::new(),
            compiled_ids: BTreeMap::new(),
        };
        let mut legacy = serde_json::to_value(make_instance()).unwrap();
        legacy.as_object_mut().unwrap().remove("name");
        let restored: ComponentInstance = serde_json::from_value(legacy).unwrap();

        let mut source = ComponentSource {
            definitions: [(component_id, definition)].into_iter().collect(),
            instances: vec![restored, make_instance()],
        };
        ensure_instance_names(&mut source);

        assert_eq!(source.instances[0].name, "Start Button 1");
        assert_eq!(source.instances[1].name, "Start Button 2");
    }
}
