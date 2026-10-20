//! Typed, ordinary CLI actions for authoring reusable component definitions.
//!
//! These actions edit the same canonical `ComponentSource` used by the visual
//! editor. `EditorProject::edit_components` validates and commits each action
//! through the normal component materialization path.

use crate::components::{
    ComponentChild, ComponentDefinition, ComponentId, ComponentInstance, ComponentInstanceId,
    ComponentNode, ComponentNodeId, ComponentParameter, ComponentParameterId, ComponentReference,
    ComponentSource, EditorParentRef,
};
use crate::EditorProject;
use ag_iso_stack::object_pool::ObjectType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ComponentAuthoringAction {
    DefinitionCreate {
        id: ComponentId,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        category_path: Vec<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        revision: u64,
        root_node: ComponentNodeId,
        root_name: String,
        root_type: ObjectType,
    },
    DefinitionMetadata {
        component_id: ComponentId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        category_path: Option<Vec<String>>,
        #[serde(default)]
        tags: Option<Vec<String>>,
        #[serde(default)]
        revision: Option<u64>,
    },
    NodeCreate {
        component_id: ComponentId,
        id: ComponentNodeId,
        name: String,
        object_type: ObjectType,
    },
    NodeOrderSet {
        component_id: ComponentId,
        nodes: Vec<ComponentNodeId>,
    },
    NodeMetadata {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        name: String,
    },
    NodePropertySet {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        property: crate::object_properties::PropertyKey,
        value: Value,
    },
    NodePictureAssignFile {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        #[serde(with = "crate::cli::editing::base64_bytes")]
        bytes: Vec<u8>,
        format: ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat,
        encoding: ag_iso_stack::object_pool::object_attributes::DataCodeType,
    },
    NodeMacroAppend {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        instruction: String,
        arguments: Vec<String>,
    },
    NodeMacroClear {
        component_id: ComponentId,
        node_id: ComponentNodeId,
    },
    NodeReferenceSet {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        property: crate::object_properties::PropertyKey,
        target: ComponentReference,
    },
    NodeChildrenSet {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        children: Vec<ComponentChild>,
    },
    NodeMacroReferencesSet {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        references: Vec<crate::components::ComponentMacroReference>,
    },
    NodeObjectListSet {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        object_list: Option<crate::components::ComponentObjectList>,
    },
    ParameterAdd {
        component_id: ComponentId,
        parameter: ComponentParameter,
    },
    ParameterSet {
        component_id: ComponentId,
        parameter: ComponentParameter,
    },
    InstanceCreate {
        id: ComponentInstanceId,
        component_id: ComponentId,
        name: String,
        parent: u16,
        x: i32,
        y: i32,
        #[serde(default)]
        child_index: Option<usize>,
        #[serde(default)]
        overrides: BTreeMap<ComponentParameterId, Value>,
        #[serde(default)]
        compiled_ids: BTreeMap<ComponentNodeId, u16>,
    },
    InstanceMetadata {
        instance_id: ComponentInstanceId,
        #[serde(default)]
        child_index: Option<usize>,
        #[serde(default)]
        compiled_ids: BTreeMap<ComponentNodeId, u16>,
    },
    OverrideSet {
        instance_id: ComponentInstanceId,
        parameter_id: ComponentParameterId,
        value: Value,
    },
}

/// Convert canonical component source into ordinary typed authoring actions.
/// IDs and authoring order are retained; no serialized project snapshot is
/// embedded in the result. Definitions are created before their links and
/// instances, so references can target nodes declared later in the source.
pub fn export_actions(
    source: &crate::components::ComponentSource,
) -> Result<Vec<ComponentAuthoringAction>, String> {
    let mut actions = Vec::new();
    for definition in source.definitions.values() {
        crate::components::validate_definition(definition).map_err(|error| error.to_string())?;
        let root = definition
            .nodes
            .iter()
            .find(|node| node.id == definition.root_node);
        let Some(root) = root else {
            return Err(format!(
                "Component {} is missing its root node",
                definition.id
            ));
        };
        actions.push(ComponentAuthoringAction::DefinitionCreate {
            id: definition.id,
            name: definition.name.clone(),
            description: definition.description.clone(),
            category_path: definition.category_path.clone(),
            tags: definition.tags.clone(),
            revision: 0,
            root_node: definition.root_node,
            root_name: root.name.clone(),
            root_type: root.object_type,
        });
        for node in definition
            .nodes
            .iter()
            .filter(|node| node.id != definition.root_node)
        {
            actions.push(ComponentAuthoringAction::NodeCreate {
                component_id: definition.id,
                id: node.id,
                name: node.name.clone(),
                object_type: node.object_type,
            });
        }
        actions.push(ComponentAuthoringAction::NodeOrderSet {
            component_id: definition.id,
            nodes: definition.nodes.iter().map(|node| node.id).collect(),
        });
        for node in &definition.nodes {
            if !node.properties.is_empty() {
                for (property, value) in &node.properties {
                    actions.push(ComponentAuthoringAction::NodePropertySet {
                        component_id: definition.id,
                        node_id: node.id,
                        property: property.clone(),
                        value: value.clone(),
                    });
                }
            }
        }
        for parameter in &definition.parameters {
            actions.push(ComponentAuthoringAction::ParameterAdd {
                component_id: definition.id,
                parameter: parameter.clone(),
            });
        }
        for node in &definition.nodes {
            if !node.references.is_empty() {
                for (property, target) in &node.references {
                    actions.push(ComponentAuthoringAction::NodeReferenceSet {
                        component_id: definition.id,
                        node_id: node.id,
                        property: property.clone(),
                        target: target.clone(),
                    });
                }
            }
            if !node.children.is_empty() {
                actions.push(ComponentAuthoringAction::NodeChildrenSet {
                    component_id: definition.id,
                    node_id: node.id,
                    children: node.children.clone(),
                });
            }
            if !node.macro_references.is_empty() {
                actions.push(ComponentAuthoringAction::NodeMacroReferencesSet {
                    component_id: definition.id,
                    node_id: node.id,
                    references: node.macro_references.clone(),
                });
            }
            if node.object_list.is_some() {
                actions.push(ComponentAuthoringAction::NodeObjectListSet {
                    component_id: definition.id,
                    node_id: node.id,
                    object_list: node.object_list.clone(),
                });
            }
        }
        // The edits above increment definition revisions. Restore the author's
        // exact revision after all definition content has been emitted.
        actions.push(ComponentAuthoringAction::DefinitionMetadata {
            component_id: definition.id,
            name: None,
            description: None,
            category_path: None,
            tags: None,
            revision: Some(definition.revision),
        });
    }
    for instance in &source.instances {
        let EditorParentRef::Primitive(parent) = &instance.parent;
        actions.push(ComponentAuthoringAction::InstanceCreate {
            id: instance.id,
            component_id: instance.component_id,
            name: instance.name.clone(),
            parent: *parent,
            x: instance.x,
            y: instance.y,
            child_index: instance.child_index,
            overrides: instance.overrides.clone(),
            compiled_ids: instance.compiled_ids.clone(),
        });
    }
    Ok(actions)
}

fn definition_name(project: &EditorProject, id: ComponentId) -> Result<String, String> {
    project
        .component_source()
        .definitions
        .get(&id)
        .map(|d| d.name.clone())
        .ok_or_else(|| format!("Unknown component definition {id}"))
}

fn node_name(
    project: &EditorProject,
    component: ComponentId,
    id: ComponentNodeId,
) -> Result<String, String> {
    project
        .component_source()
        .definitions
        .get(&component)
        .and_then(|d| d.nodes.iter().find(|n| n.id == id))
        .map(|n| n.name.clone())
        .ok_or_else(|| format!("Unknown node {id} in component {component}"))
}

fn parameter_name(
    project: &EditorProject,
    component: ComponentId,
    id: ComponentParameterId,
) -> Result<String, String> {
    project
        .component_source()
        .definitions
        .get(&component)
        .and_then(|d| d.parameters.iter().find(|p| p.id == id))
        .map(|p| p.name.clone())
        .ok_or_else(|| format!("Unknown parameter {id} in component {component}"))
}

fn named_object_target(
    project: &EditorProject,
    target: &crate::operations::ObjectTarget,
) -> Result<crate::operations::ObjectTarget, String> {
    match target {
        crate::operations::ObjectTarget::Name { name } => {
            Ok(crate::operations::ObjectTarget::Name { name: name.clone() })
        }
        crate::operations::ObjectTarget::Id { id } => Ok(crate::operations::ObjectTarget::Name {
            name: crate::cli::zip::object_name(project, *id),
        }),
    }
}

fn named_macro_references(
    project: &EditorProject,
    refs: &[crate::components::ComponentMacroReference],
) -> Result<Vec<crate::components::ComponentMacroReference>, String> {
    refs.iter()
        .map(|r| {
            Ok(crate::components::ComponentMacroReference {
                event: r.event,
                target: named_object_target(project, &r.target)?,
            })
        })
        .collect()
}

fn named_object_list(
    project: &EditorProject,
    list: &Option<crate::components::ComponentObjectList>,
) -> Result<Option<crate::components::ComponentObjectList>, String> {
    use crate::components::ComponentObjectList as L;
    list.as_ref()
        .map(|list| match list {
            L::Required(items) => items
                .iter()
                .map(|t| named_object_target(project, t))
                .collect::<Result<Vec<_>, _>>()
                .map(L::Required),
            L::Nullable(items) => items
                .iter()
                .map(|t| {
                    t.as_ref()
                        .map(|t| named_object_target(project, t))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()
                .map(L::Nullable),
        })
        .transpose()
}

fn named_parameter_value(
    project: &EditorProject,
    kind: crate::components::ComponentParameterKind,
    value: &Value,
) -> Result<Value, String> {
    use crate::components::ComponentParameterKind as K;
    match kind {
        K::Graphic => {
            let id: crate::graphics::GraphicAssetId =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            let asset = project
                .graphic_library()
                .assets
                .get(&id)
                .ok_or_else(|| format!("Unknown graphic asset {id}"))?;
            Ok(Value::String(asset.name.clone()))
        }
        K::ObjectReference if !value.is_null() => {
            let target: crate::operations::ObjectTarget = if let Some(id) = value.as_u64() {
                let id = u16::try_from(id).map_err(|_| "Object reference ID is out of range")?;
                crate::operations::ObjectTarget::Name {
                    name: crate::cli::zip::object_name(project, id),
                }
            } else {
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?
            };
            serde_json::to_value(named_object_target(project, &target)?).map_err(|e| e.to_string())
        }
        _ => Ok(value.clone()),
    }
}

fn parameter_default_token(
    project: &EditorProject,
    parameter: &ComponentParameter,
) -> Result<String, String> {
    use crate::components::ComponentParameterKind as K;
    match parameter.kind {
        K::Graphic => Ok(
            named_parameter_value(project, parameter.kind, &parameter.default_value)?
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ),
        K::ObjectReference if !parameter.default_value.is_null() => {
            let named = named_parameter_value(project, parameter.kind, &parameter.default_value)?;
            let name = named
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(if name == "null" {
                "name:null".into()
            } else {
                name.to_owned()
            })
        }
        K::ObjectReference => Ok("null".into()),
        _ => serde_json::to_string(&parameter.default_value).map_err(|e| e.to_string()),
    }
}

fn named_overrides(
    project: &EditorProject,
    component: ComponentId,
    overrides: &BTreeMap<ComponentParameterId, Value>,
) -> Result<BTreeMap<String, Value>, String> {
    let definition = project
        .component_source()
        .definitions
        .get(&component)
        .ok_or_else(|| format!("Unknown component definition {component}"))?;
    overrides
        .iter()
        .map(|(id, value)| {
            let parameter = definition
                .parameters
                .iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| format!("Unknown parameter {id}"))?;
            Ok((
                parameter.name.clone(),
                named_parameter_value(project, parameter.kind, value)?,
            ))
        })
        .collect()
}

fn named_reference(
    project: &EditorProject,
    component: ComponentId,
    target: &ComponentReference,
) -> Result<Value, String> {
    Ok(match target {
        ComponentReference::Internal(id) => {
            serde_json::json!({"Internal": node_name(project, component, *id)?})
        }
        ComponentReference::Parameter(id) => {
            serde_json::json!({"Parameter": parameter_name(project, component, *id)?})
        }
        ComponentReference::ExternalObject(target) => {
            serde_json::json!({"ExternalObject": named_object_target(project, target)?})
        }
        ComponentReference::Null => serde_json::json!("Null"),
    })
}

/// Render a typed authoring action as ordinary name-based CLI command tokens.
/// The supplied project resolves internal component, instance, and object
/// identities to their persisted authoring names. The caller should quote each
/// token with the shared CLI shell serializer.
pub fn export_command_tokens(
    action: &ComponentAuthoringAction,
    project: &EditorProject,
) -> Result<Vec<String>, String> {
    use ComponentAuthoringAction as A;
    let mut tokens = vec!["component".to_owned()];
    match action {
        A::DefinitionCreate {
            id: _,
            name,
            description,
            category_path,
            tags,
            root_node: _,
            root_name,
            root_type,
            ..
        } => {
            tokens.extend([
                "definition-create".into(),
                name.clone(),
                root_name.clone(),
                format!("{root_type:?}"),
            ]);
            if !description.is_empty() {
                tokens.extend(["--description".into(), description.clone()]);
            }
            for category in category_path {
                tokens.extend(["--category".into(), category.clone()]);
            }
            for tag in tags {
                tokens.extend(["--tag".into(), tag.clone()]);
            }
        }
        A::DefinitionMetadata {
            component_id,
            name,
            description,
            category_path,
            tags,
            revision,
        } => {
            tokens.extend([
                "definition-metadata".into(),
                definition_name(project, *component_id)?,
            ]);
            if let Some(value) = name {
                tokens.extend(["--name".into(), value.clone()]);
            }
            if let Some(value) = description {
                tokens.extend(["--description".into(), value.clone()]);
            }
            if let Some(values) = category_path {
                for value in values {
                    tokens.extend(["--category".into(), value.clone()]);
                }
            }
            if let Some(values) = tags {
                for value in values {
                    tokens.extend(["--tag".into(), value.clone()]);
                }
            }
            if let Some(value) = revision {
                tokens.extend(["--revision".into(), value.to_string()]);
            }
        }
        A::NodeCreate {
            component_id,
            id: _,
            name,
            object_type,
        } => tokens.extend([
            "node-create".into(),
            definition_name(project, *component_id)?,
            name.clone(),
            format!("{object_type:?}"),
        ]),
        A::NodeOrderSet {
            component_id,
            nodes,
        } => {
            tokens.extend([
                "node-order-set".into(),
                definition_name(project, *component_id)?,
            ]);
            tokens.extend(
                nodes
                    .iter()
                    .map(|id| node_name(project, *component_id, *id))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        A::NodeMetadata {
            component_id,
            node_id,
            name,
        } => tokens.extend([
            "node-rename".into(),
            definition_name(project, *component_id)?,
            node_name(project, *component_id, *node_id)?,
            name.clone(),
        ]),
        A::NodePropertySet {
            component_id,
            node_id,
            property,
            value,
        } => tokens.extend([
            "node-property-set".into(),
            definition_name(project, *component_id)?,
            node_name(project, *component_id, *node_id)?,
            property.to_string(),
            serde_json::to_string(value).map_err(|error| error.to_string())?,
        ]),
        A::NodePictureAssignFile { .. } => {
            return Err(
                "PictureGraphic component updates require a project-relative indexed PNG path"
                    .into(),
            );
        }
        A::NodeMacroAppend {
            component_id,
            node_id,
            instruction,
            arguments,
        } => {
            tokens.extend([
                "node-macro-append".into(),
                definition_name(project, *component_id)?,
                node_name(project, *component_id, *node_id)?,
                instruction.clone(),
            ]);
            tokens.extend(arguments.iter().cloned());
        }
        A::NodeMacroClear {
            component_id,
            node_id,
        } => tokens.extend([
            "node-macro-clear".into(),
            definition_name(project, *component_id)?,
            node_name(project, *component_id, *node_id)?,
        ]),
        A::NodeReferenceSet {
            component_id,
            node_id,
            property,
            target,
        } => tokens.extend([
            "node-reference-set".into(),
            definition_name(project, *component_id)?,
            node_name(project, *component_id, *node_id)?,
            property.to_string(),
            serde_json::to_string(&named_reference(project, *component_id, target)?)
                .map_err(|error| error.to_string())?,
        ]),
        A::NodeChildrenSet {
            component_id,
            node_id,
            children,
        } => {
            tokens.extend([
                "node-children-set".into(),
                definition_name(project, *component_id)?,
                node_name(project, *component_id, *node_id)?,
            ]);
            for child in children {
                tokens.push(format!(
                    "{}:{}:{}",
                    node_name(project, *component_id, child.child)?,
                    child.x,
                    child.y
                ));
            }
        }
        A::NodeMacroReferencesSet {
            component_id,
            node_id,
            references,
        } => tokens.extend([
            "node-macro-references-set".into(),
            definition_name(project, *component_id)?,
            node_name(project, *component_id, *node_id)?,
            serde_json::to_string(&named_macro_references(project, references)?)
                .map_err(|error| error.to_string())?,
        ]),
        A::NodeObjectListSet {
            component_id,
            node_id,
            object_list,
        } => tokens.extend([
            "node-object-list-set".into(),
            definition_name(project, *component_id)?,
            node_name(project, *component_id, *node_id)?,
            serde_json::to_string(&named_object_list(project, object_list)?)
                .map_err(|error| error.to_string())?,
        ]),
        A::ParameterAdd {
            component_id,
            parameter,
        } => {
            tokens.extend([
                "parameter-add-typed".into(),
                definition_name(project, *component_id)?,
                parameter.name.clone(),
                node_name(project, *component_id, parameter.target_node)?,
                parameter.property.as_str().into(),
                format!("{:?}", parameter.kind),
                parameter_default_token(project, parameter)?,
            ]);
            if let Some(description) = &parameter.description {
                tokens.extend(["--description".into(), description.clone()]);
            }
        }
        A::ParameterSet {
            component_id,
            parameter,
        } => {
            tokens.extend([
                "parameter-set-typed".into(),
                definition_name(project, *component_id)?,
                parameter_name(project, *component_id, parameter.id)?,
                parameter.name.clone(),
                node_name(project, *component_id, parameter.target_node)?,
                parameter.property.as_str().into(),
                format!("{:?}", parameter.kind),
                parameter_default_token(project, parameter)?,
            ]);
            if let Some(description) = &parameter.description {
                tokens.extend(["--description".into(), description.clone()]);
            }
        }
        A::InstanceCreate {
            id: _,
            component_id,
            name,
            parent,
            x,
            y,
            child_index,
            overrides,
            compiled_ids,
        } => {
            tokens.extend([
                "instance-create".into(),
                name.clone(),
                definition_name(project, *component_id)?,
                crate::cli::zip::object_token(project, *parent),
                x.to_string(),
                y.to_string(),
            ]);
            if let Some(index) = child_index {
                tokens.extend(["--child-index".into(), index.to_string()]);
            }
            if !overrides.is_empty() {
                tokens.extend([
                    "--overrides".into(),
                    serde_json::to_string(&named_overrides(project, *component_id, overrides)?)
                        .map_err(|error| error.to_string())?,
                ]);
            }
            let _ = compiled_ids;
        }
        A::InstanceMetadata {
            instance_id,
            child_index,
            compiled_ids,
        } => {
            let instance = project
                .component_source()
                .instances
                .iter()
                .find(|instance| instance.id == *instance_id)
                .ok_or_else(|| format!("Unknown component instance {instance_id}"))?;
            tokens.extend(["instance-metadata".into(), instance.name.clone()]);
            if let Some(index) = child_index {
                tokens.extend(["--child-index".into(), index.to_string()]);
            }
            let _ = compiled_ids;
        }
        A::OverrideSet {
            instance_id,
            parameter_id,
            value,
        } => {
            let instance = project
                .component_source()
                .instances
                .iter()
                .find(|instance| instance.id == *instance_id)
                .ok_or_else(|| format!("Unknown component instance {instance_id}"))?;
            let parameter = project.component_source().definitions[&instance.component_id]
                .parameters
                .iter()
                .find(|p| p.id == *parameter_id)
                .ok_or_else(|| format!("Unknown parameter {parameter_id}"))?;
            let value = named_parameter_value(project, parameter.kind, value)?;
            tokens.extend([
                "override-set".into(),
                instance.name.clone(),
                parameter.name.clone(),
                serde_json::to_string(&value).map_err(|error| error.to_string())?,
            ]);
        }
    }
    Ok(tokens)
}

pub fn apply(project: &mut EditorProject, action: ComponentAuthoringAction) -> Result<(), String> {
    let pool = project.get_pool().clone();
    let version = project.vt_version();
    let original_source = project.component_source().clone();
    project
        .edit_components("CLI component authoring", |_, source| {
            match action {
                ComponentAuthoringAction::DefinitionCreate {
                    id,
                    name,
                    description,
                    category_path,
                    tags,
                    revision,
                    root_node,
                    root_name,
                    root_type,
                } => {
                    if source.definitions.contains_key(&id) {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            format!("Component definition {id} already exists"),
                        ));
                    }
                    source.definitions.insert(
                        id,
                        ComponentDefinition {
                            schema_version: 1,
                            id,
                            name,
                            description,
                            category_path,
                            tags,
                            revision,
                            root_node,
                            nodes: vec![empty_node(root_node, root_name, root_type)],
                            parameters: Vec::new(),
                        },
                    );
                }
                ComponentAuthoringAction::DefinitionMetadata {
                    component_id,
                    name,
                    description,
                    category_path,
                    tags,
                    revision,
                } => {
                    let definition = definition_mut(source, component_id)?;
                    if let Some(name) = name {
                        definition.name = name;
                    }
                    if let Some(description) = description {
                        definition.description = description;
                    }
                    if let Some(category_path) = category_path {
                        definition.category_path = category_path;
                    }
                    if let Some(tags) = tags {
                        definition.tags = tags;
                    }
                    definition.revision = revision.unwrap_or_else(|| definition.revision + 1);
                }
                ComponentAuthoringAction::NodeCreate {
                    component_id,
                    id,
                    name,
                    object_type,
                } => {
                    let definition = definition_mut(source, component_id)?;
                    if definition.nodes.iter().any(|node| node.id == id) {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            format!("Component node {id} already exists"),
                        ));
                    }
                    definition.nodes.push(empty_node(id, name, object_type));
                    definition.revision += 1;
                }
                ComponentAuthoringAction::NodeOrderSet {
                    component_id,
                    nodes,
                } => {
                    let definition = definition_mut(source, component_id)?;
                    if nodes.len() != definition.nodes.len()
                        || nodes.iter().enumerate().any(|(index, id)| {
                            nodes[..index].contains(id)
                                || !definition.nodes.iter().any(|node| node.id == *id)
                        })
                    {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            "Node order must contain each component node exactly once",
                        ));
                    }
                    let mut remaining = std::mem::take(&mut definition.nodes);
                    definition.nodes = nodes
                        .into_iter()
                        .map(|id| {
                            let index = remaining.iter().position(|node| node.id == id).unwrap();
                            remaining.remove(index)
                        })
                        .collect();
                    definition.revision += 1;
                }
                ComponentAuthoringAction::NodeMetadata {
                    component_id,
                    node_id,
                    name,
                } => {
                    node_mut(source, component_id, node_id)?.name = name;
                    definition_mut(source, component_id)?.revision += 1;
                }
                ComponentAuthoringAction::NodePropertySet {
                    component_id,
                    node_id,
                    property,
                    value,
                } => {
                    node_mut(source, component_id, node_id)?
                        .properties
                        .insert(property, value);
                    definition_mut(source, component_id)?.revision += 1;
                }
                ComponentAuthoringAction::NodePictureAssignFile {
                    component_id,
                    node_id,
                    bytes,
                    format,
                    encoding,
                } => {
                    let node = node_mut(source, component_id, node_id)?;
                    if node.object_type != ag_iso_stack::object_pool::ObjectType::PictureGraphic {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            format!("Component node {node_id} must be a PictureGraphic"),
                        ));
                    }
                    crate::graphics::indexed_png::apply_png_to_component_properties(
                        &mut node.properties,
                        &pool,
                        &bytes,
                        format,
                        encoding,
                    )
                    .map_err(|error| {
                        crate::components::ComponentError::new(
                            "GRAPHIC_CONVERSION_FAILED",
                            format!("Component {component_id}, node {node_id}: {error}"),
                        )
                    })?;
                    definition_mut(source, component_id)?.revision += 1;
                }
                ComponentAuthoringAction::NodeMacroClear {
                    component_id,
                    node_id,
                } => {
                    let definition = definition_mut(source, component_id)?;
                    let node = definition
                        .nodes
                        .iter_mut()
                        .find(|node| node.id == node_id)
                        .ok_or_else(|| {
                            crate::components::ComponentError::new(
                                "COMPONENT_NODE_INVALID",
                                format!("Unknown component node {node_id}"),
                            )
                        })?;
                    if node.object_type != ObjectType::Macro {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            format!("Component node {node_id} must be a Macro"),
                        ));
                    }
                    node.properties.insert(
                        crate::object_properties::PropertyKey::Commands,
                        Value::Array(Vec::new()),
                    );
                    definition.revision += 1;
                }
                ComponentAuthoringAction::NodeMacroAppend {
                    component_id,
                    node_id,
                    instruction,
                    arguments,
                } => {
                    let definition = definition_mut(source, component_id)?;
                    let node = definition
                        .nodes
                        .iter_mut()
                        .find(|node| node.id == node_id)
                        .ok_or_else(|| {
                            crate::components::ComponentError::new(
                                "COMPONENT_NODE_INVALID",
                                format!("Unknown component node {node_id}"),
                            )
                        })?;
                    if node.object_type != ObjectType::Macro {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            format!("Component node {node_id} must be a Macro"),
                        ));
                    }
                    let key = crate::object_properties::PropertyKey::Commands;
                    let mut commands = match node.properties.get(&key) {
                        Some(Value::Array(values)) => values
                            .iter()
                            .enumerate()
                            .map(|(index, value)| {
                                value
                                    .as_u64()
                                    .and_then(|byte| u8::try_from(byte).ok())
                                    .ok_or_else(|| {
                                        crate::components::ComponentError::new(
                                            "COMPONENT_NODE_INVALID",
                                            format!("Macro node {node_id} command byte {index} is not a byte"),
                                        )
                                    })
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        Some(_) => {
                            return Err(crate::components::ComponentError::new(
                                "COMPONENT_NODE_INVALID",
                                format!("Macro node {node_id} commands property is not a byte array"),
                            ));
                        }
                        None => Vec::new(),
                    };
                    let encoded = crate::macro_authoring::encode_macro_instruction(
                        &instruction,
                        &arguments,
                        version,
                    )
                    .map_err(|error| {
                        crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            format!("Macro node {node_id}: {error}"),
                        )
                    })?;
                    commands.extend(encoded);
                    crate::vt_macro_commands::validate_macro_commands(&commands, version).map_err(
                        |error| {
                            crate::components::ComponentError::new(
                                "COMPONENT_NODE_INVALID",
                                format!("Macro node {node_id}: {error}"),
                            )
                        },
                    )?;
                    node.properties.insert(
                        key,
                        Value::Array(commands.into_iter().map(Value::from).collect()),
                    );
                    definition.revision += 1;
                }
                ComponentAuthoringAction::NodeReferenceSet {
                    component_id,
                    node_id,
                    property,
                    target,
                } => {
                    let node = node_mut(source, component_id, node_id)?;
                    node.properties.remove(&property);
                    node.references.insert(property, target);
                    definition_mut(source, component_id)?.revision += 1;
                }
                ComponentAuthoringAction::NodeChildrenSet {
                    component_id,
                    node_id,
                    children,
                } => {
                    let ids: BTreeSet<_> = children.iter().map(|child| child.child).collect();
                    if ids.len() != children.len() {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_NODE_INVALID",
                            "A component parent cannot contain the same child twice",
                        ));
                    }
                    node_mut(source, component_id, node_id)?.children = children;
                    definition_mut(source, component_id)?.revision += 1;
                }
                ComponentAuthoringAction::NodeMacroReferencesSet {
                    component_id,
                    node_id,
                    references,
                } => {
                    node_mut(source, component_id, node_id)?.macro_references = references;
                    definition_mut(source, component_id)?.revision += 1;
                }
                ComponentAuthoringAction::NodeObjectListSet {
                    component_id,
                    node_id,
                    object_list,
                } => {
                    node_mut(source, component_id, node_id)?.object_list = object_list;
                    definition_mut(source, component_id)?.revision += 1;
                }
                ComponentAuthoringAction::ParameterAdd {
                    component_id,
                    parameter,
                } => {
                    let definition = definition_mut(source, component_id)?;
                    if definition
                        .parameters
                        .iter()
                        .any(|item| item.id == parameter.id)
                    {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_PARAMETER_INVALID",
                            format!("Component parameter {} already exists", parameter.id),
                        ));
                    }
                    definition.parameters.push(parameter);
                    definition.revision += 1;
                }
                ComponentAuthoringAction::ParameterSet {
                    component_id,
                    parameter,
                } => {
                    let definition = definition_mut(source, component_id)?;
                    let target = definition
                        .parameters
                        .iter_mut()
                        .find(|item| item.id == parameter.id)
                        .ok_or_else(|| {
                            crate::components::ComponentError::new(
                                "COMPONENT_PARAMETER_INVALID",
                                format!("Unknown component parameter {}", parameter.id),
                            )
                        })?;
                    *target = parameter;
                    definition.revision += 1;
                }
                ComponentAuthoringAction::InstanceCreate {
                    id,
                    component_id,
                    name,
                    parent,
                    x,
                    y,
                    child_index,
                    overrides,
                    compiled_ids,
                } => {
                    if source.instances.iter().any(|instance| instance.id == id) {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_INSTANCE_INVALID",
                            format!("Component instance {id} already exists"),
                        ));
                    }
                    if crate::smart_naming::normalized_name(&name).is_empty()
                        || source.instances.iter().any(|instance| {
                            crate::smart_naming::names_equal(&instance.name, &name)
                        })
                    {
                        return Err(crate::components::ComponentError::new(
                            "COMPONENT_INSTANCE_NAME_CONFLICT",
                            format!("Component instance name is empty or already exists: {name}"),
                        ));
                    }
                    source.instances.push(ComponentInstance {
                        id,
                        component_id,
                        name,
                        parent: EditorParentRef::Primitive(parent),
                        child_index,
                        x,
                        y,
                        overrides,
                        compiled_ids,
                    });
                }
                ComponentAuthoringAction::InstanceMetadata {
                    instance_id,
                    child_index,
                    compiled_ids,
                } => {
                    let instance = source
                        .instances
                        .iter_mut()
                        .find(|instance| instance.id == instance_id)
                        .ok_or_else(|| {
                            crate::components::ComponentError::new(
                                "COMPONENT_INSTANCE_INVALID",
                                format!("Unknown component instance {instance_id}"),
                            )
                        })?;
                    instance.child_index = child_index;
                    if !compiled_ids.is_empty() {
                        instance.compiled_ids = compiled_ids;
                    }
                }
                ComponentAuthoringAction::OverrideSet {
                    instance_id,
                    parameter_id,
                    value,
                } => {
                    let instance = source
                        .instances
                        .iter_mut()
                        .find(|instance| instance.id == instance_id)
                        .ok_or_else(|| {
                            crate::components::ComponentError::new(
                                "COMPONENT_INSTANCE_INVALID",
                                format!("Unknown component instance {instance_id}"),
                            )
                        })?;
                    instance.overrides.insert(parameter_id, value);
                }
            }
            for definition in source.definitions.values() {
                crate::components::validate_definition_names(&original_source, definition)?;
            }
            Ok(())
        })
        .map_err(|error| error.to_string())
}

fn empty_node(id: ComponentNodeId, name: String, object_type: ObjectType) -> ComponentNode {
    ComponentNode {
        id,
        name,
        object_type,
        properties: BTreeMap::new(),
        children: Vec::new(),
        references: BTreeMap::new(),
        macro_references: Vec::new(),
        object_list: None,
    }
}

fn definition_mut(
    source: &mut ComponentSource,
    id: ComponentId,
) -> crate::components::ComponentResult<&mut ComponentDefinition> {
    source.definitions.get_mut(&id).ok_or_else(|| {
        crate::components::ComponentError::new(
            "COMPONENT_NOT_FOUND",
            format!("Unknown component definition {id}"),
        )
    })
}

fn node_mut(
    source: &mut ComponentSource,
    component_id: ComponentId,
    node_id: ComponentNodeId,
) -> crate::components::ComponentResult<&mut ComponentNode> {
    definition_mut(source, component_id)?
        .nodes
        .iter_mut()
        .find(|node| node.id == node_id)
        .ok_or_else(|| {
            crate::components::ComponentError::new(
                "COMPONENT_NODE_INVALID",
                format!("Unknown component node {node_id}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::ObjectType;
    use clap::Parser;

    #[test]
    fn exported_component_actions_parse_as_readable_cli_commands() {
        let component_id = ComponentId(uuid::Uuid::from_u128(1));
        let root_id = ComponentNodeId(uuid::Uuid::from_u128(2));
        let child_id = ComponentNodeId(uuid::Uuid::from_u128(3));
        let root = ComponentNode {
            id: root_id,
            name: "Root\nwith text".into(),
            object_type: ObjectType::DataMask,
            properties: BTreeMap::new(),
            children: vec![ComponentChild {
                child: child_id,
                x: 12,
                y: -4,
            }],
            references: BTreeMap::new(),
            macro_references: Vec::new(),
            object_list: None,
        };
        let child = empty_node(child_id, "Button".into(), ObjectType::Button);
        let source = ComponentSource {
            definitions: BTreeMap::from([(
                component_id,
                ComponentDefinition {
                    schema_version: 1,
                    id: component_id,
                    name: "Definition name".into(),
                    description: "Description".into(),
                    category_path: vec!["Tests".into()],
                    tags: vec!["example".into()],
                    revision: 7,
                    root_node: root_id,
                    nodes: vec![root, child],
                    parameters: Vec::new(),
                },
            )]),
            instances: Vec::new(),
        };

        let actions = export_actions(&source).unwrap();
        assert!(actions.len() >= 3);
        let mut project = EditorProject::new_pool();
        project
            .edit_components("test source", |_, current| {
                *current = source.clone();
                Ok(())
            })
            .unwrap();
        for action in &actions {
            let mut tokens = vec!["aitd".to_owned()];
            tokens.extend(export_command_tokens(action, &project).unwrap());
            let parsed = crate::cli::commands::Cli::try_parse_from(tokens).unwrap();
            assert!(matches!(
                parsed.command,
                crate::cli::commands::Command::Component { .. }
            ));
        }
    }
}
