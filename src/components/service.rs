use super::*;
use crate::EditorProject;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Shared editor commands. The primitive executor remains component-unaware.
pub struct ComponentService;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComponentMasterScope {
    AllInstances,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ComponentIntent {
    AssignGraphicAsset {
        object_id: u16,
        asset_id: crate::graphics::GraphicAssetId,
    },
    RemoveGraphicAssignment {
        object_id: u16,
    },
    InstantiateComponent {
        #[serde(default)]
        instance_id: Option<ComponentInstanceId>,
        component_id: ComponentId,
        parent: u16,
        x: i32,
        y: i32,
        #[serde(default)]
        overrides: BTreeMap<ComponentParameterId, Value>,
    },
    SetComponentOverride {
        instance_id: ComponentInstanceId,
        parameter: ComponentParameterId,
        value: Value,
    },
    SetComponentMasterProperty {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        property: String,
        value: Value,
        scope: ComponentMasterScope,
    },
    ResetComponentOverride {
        instance_id: ComponentInstanceId,
        parameter: ComponentParameterId,
    },
}
impl ComponentService {
    pub fn apply_intents(
        project: &mut EditorProject,
        intents: &[ComponentIntent],
    ) -> ComponentResult<()> {
        project.edit_source("Component actions", |pool, source, graphics| {
            for intent in intents {
                match intent {
                    ComponentIntent::AssignGraphicAsset {
                        object_id,
                        asset_id,
                    } => {
                        graphics.set_binding(*object_id, Some(*asset_id));
                    }
                    ComponentIntent::RemoveGraphicAssignment { object_id } => {
                        graphics.set_binding(*object_id, None);
                    }
                    ComponentIntent::InstantiateComponent {
                        instance_id,
                        component_id,
                        parent,
                        x,
                        y,
                        overrides,
                    } => source.instances.push(ComponentInstance {
                        id: instance_id.unwrap_or_default(),
                        component_id: *component_id,
                        child_index: Some(insertion_index(pool, source, *parent)),
                        parent: EditorParentRef::Primitive(*parent),
                        x: *x,
                        y: *y,
                        overrides: overrides.clone(),
                        compiled_ids: Default::default(),
                    }),
                    ComponentIntent::SetComponentOverride {
                        instance_id,
                        parameter,
                        value,
                    } => {
                        instance_mut(source, *instance_id)?
                            .overrides
                            .insert(*parameter, value.clone());
                    }
                    ComponentIntent::SetComponentMasterProperty {
                        component_id,
                        node_id,
                        property,
                        value,
                        scope: ComponentMasterScope::AllInstances,
                    } => {
                        let definition =
                            source.definitions.get_mut(component_id).ok_or_else(|| {
                                ComponentError::new("COMPONENT_NOT_FOUND", "Unknown master")
                            })?;
                        let node = definition
                            .nodes
                            .iter_mut()
                            .find(|n| n.id == *node_id)
                            .ok_or_else(|| {
                                ComponentError::new("COMPONENT_NODE_INVALID", "Unknown master node")
                            })?;
                        node.properties.insert(property.clone(), value.clone());
                        for parameter in definition
                            .parameters
                            .iter_mut()
                            .filter(|p| p.target_node == *node_id && p.property == *property)
                        {
                            parameter.default_value = value.clone();
                        }
                        definition.revision += 1;
                    }
                    ComponentIntent::ResetComponentOverride {
                        instance_id,
                        parameter,
                    } => {
                        let component = instance_mut(source, *instance_id)?.component_id;
                        if !source
                            .definitions
                            .get(&component)
                            .is_some_and(|d| d.parameters.iter().any(|p| p.id == *parameter))
                        {
                            return Err(ComponentError::new(
                                "COMPONENT_PARAMETER_INVALID",
                                "Unknown declared parameter",
                            ));
                        }
                        instance_mut(source, *instance_id)?
                            .overrides
                            .remove(parameter);
                    }
                }
            }
            Ok(())
        })
    }
    pub fn instantiate_component(
        project: &mut EditorProject,
        component: ComponentId,
        parent: u16,
        x: i32,
        y: i32,
        overrides: BTreeMap<ComponentParameterId, Value>,
    ) -> ComponentResult<ComponentInstanceId> {
        let id = ComponentInstanceId::default();
        project.edit_components("Instantiate component", |pool, source| {
            source.instances.push(ComponentInstance {
                id,
                component_id: component,
                child_index: Some(insertion_index(pool, source, parent)),
                parent: EditorParentRef::Primitive(parent),
                x,
                y,
                overrides,
                compiled_ids: Default::default(),
            });
            Ok(())
        })?;
        Ok(id)
    }
    pub fn edit_component_definition(
        project: &mut EditorProject,
        mut definition: ComponentDefinition,
    ) -> ComponentResult<()> {
        project.edit_components("Edit component definition", |_, source| {
            definition.revision = source
                .definitions
                .get(&definition.id)
                .map_or(1, |d| d.revision + 1);
            // Removing a reference parameter restores its authored default as an explicit reference.
            if let Some(previous) = source.definitions.get(&definition.id) {
                for removed in previous.parameters.iter().filter(|p| !definition.parameters.iter().any(|current| current.id == p.id)) {
                    for node in &mut definition.nodes {
                        for reference in node.references.values_mut() {
                            if matches!(reference, ComponentReference::Parameter(id) if *id == removed.id) {
                                *reference = if removed.default_value.is_null() { ComponentReference::Null }
                                else if let Some(id) = removed.default_value.as_u64().and_then(|id| u16::try_from(id).ok()) { ComponentReference::ExternalObject(crate::ObjectTarget::id(id)) }
                                else { ComponentReference::ExternalObject(serde_json::from_value(removed.default_value.clone()).map_err(|e| ComponentError::new("COMPONENT_PARAMETER_INVALID", e.to_string()))?) };
                            }
                        }
                    }
                }
            }
            // Parameter removal is explicit; obsolete overrides have no remaining meaning.
            for i in source
                .instances
                .iter_mut()
                .filter(|i| i.component_id == definition.id)
            {
                i.overrides
                    .retain(|id, _| definition.parameters.iter().any(|p| p.id == *id));
            }
            source.definitions.insert(definition.id, definition);
            Ok(())
        })
    }
    pub fn add_component_parameter(
        project: &mut EditorProject,
        component: ComponentId,
        parameter: ComponentParameter,
    ) -> ComponentResult<()> {
        let mut definition = definition(project, component)?;
        if definition.parameters.iter().any(|p| p.id == parameter.id) {
            return Err(ComponentError::new(
                "COMPONENT_PARAMETER_INVALID",
                "Parameter already exists",
            ));
        }
        definition.parameters.push(parameter);
        Self::edit_component_definition(project, definition)
    }
    pub fn edit_component_parameter(
        project: &mut EditorProject,
        component: ComponentId,
        parameter: ComponentParameter,
    ) -> ComponentResult<()> {
        let mut definition = definition(project, component)?;
        let target = definition
            .parameters
            .iter_mut()
            .find(|p| p.id == parameter.id)
            .ok_or_else(|| {
                ComponentError::new("COMPONENT_PARAMETER_INVALID", "Unknown parameter")
            })?;
        *target = parameter;
        Self::edit_component_definition(project, definition)
    }
    pub fn remove_component_parameter(
        project: &mut EditorProject,
        component: ComponentId,
        parameter: ComponentParameterId,
    ) -> ComponentResult<()> {
        let mut definition = definition(project, component)?;
        definition.parameters.retain(|p| p.id != parameter);
        Self::edit_component_definition(project, definition)
    }
    pub fn set_component_override(
        project: &mut EditorProject,
        instance_id: ComponentInstanceId,
        parameter: ComponentParameterId,
        value: Value,
    ) -> ComponentResult<()> {
        Self::apply_intents(
            project,
            &[ComponentIntent::SetComponentOverride {
                instance_id,
                parameter,
                value,
            }],
        )
    }
    pub fn reset_component_override(
        project: &mut EditorProject,
        instance_id: ComponentInstanceId,
        parameter: ComponentParameterId,
    ) -> ComponentResult<()> {
        Self::apply_intents(
            project,
            &[ComponentIntent::ResetComponentOverride {
                instance_id,
                parameter,
            }],
        )
    }
    pub fn reset_all_overrides(
        project: &mut EditorProject,
        id: ComponentInstanceId,
    ) -> ComponentResult<()> {
        project.edit_components("Reset component overrides", |_, source| {
            instance_mut(source, id)?.overrides.clear();
            Ok(())
        })
    }
    pub fn move_instance(
        project: &mut EditorProject,
        id: ComponentInstanceId,
        x: i32,
        y: i32,
    ) -> ComponentResult<()> {
        project.edit_components("Move component instance", |_, source| {
            let i = instance_mut(source, id)?;
            i.x = x;
            i.y = y;
            Ok(())
        })
    }
    pub fn delete_component_instance(
        project: &mut EditorProject,
        id: ComponentInstanceId,
    ) -> ComponentResult<()> {
        project.edit_components("Delete component instance", |_, source| {
            let removed = instance_mut(source, id)?.clone();
            source.instances.retain(|i| i.id != id);
            if let Some(index) = removed.child_index {
                for i in &mut source.instances {
                    if i.parent == removed.parent && i.child_index.is_some_and(|n| n > index) {
                        i.child_index = i.child_index.map(|n| n - 1);
                    }
                }
            }
            Ok(())
        })
    }
    pub fn duplicate_component_instance(
        project: &mut EditorProject,
        id: ComponentInstanceId,
        x: i32,
        y: i32,
    ) -> ComponentResult<ComponentInstanceId> {
        let new_id = ComponentInstanceId::default();
        project.edit_components("Duplicate component instance", |pool, source| {
            let mut copy = instance_mut(source, id)?.clone();
            copy.id = new_id;
            let EditorParentRef::Primitive(parent) = copy.parent;
            copy.child_index = Some(insertion_index(pool, source, parent));
            copy.x = x;
            copy.y = y;
            copy.compiled_ids.clear();
            source.instances.push(copy);
            Ok(())
        })?;
        Ok(new_id)
    }
    pub fn detach_component_instance(
        project: &mut EditorProject,
        id: ComponentInstanceId,
    ) -> ComponentResult<()> {
        let materialized = materialize_project(project)?;
        let owners = crate::graphics::integration::assignments(
            project.component_source(),
            project.graphic_library(),
        )?;
        // Strip only graphic overrides to recover the authored node bytes. The
        // detached ordinary binding continues to override these same raw bytes.
        let mut raw_source = project.component_source().clone();
        let graphic_parameters: std::collections::BTreeSet<_> = raw_source
            .definitions
            .values()
            .flat_map(|d| &d.parameters)
            .filter(|p| p.kind == ComponentParameterKind::Graphic)
            .map(|p| p.id)
            .collect();
        for d in raw_source.definitions.values_mut() {
            d.parameters.retain(|p| !graphic_parameters.contains(&p.id));
        }
        for i in &mut raw_source.instances {
            i.overrides.retain(|p, _| !graphic_parameters.contains(p));
        }
        let raw = materialize_source(
            project.get_pool(),
            &raw_source,
            project.vt_version,
            &project.component_object_info(),
        )?;
        project.edit_source("Detach component instance", |pool, source, graphics| {
            let instance = instance_mut(source, id)?.clone();
            for object_id in instance.compiled_ids.values() {
                pool.add(
                    materialized
                        .pool
                        .object_by_id(ag_iso_stack::object_pool::ObjectId::new(*object_id).unwrap())
                        .unwrap()
                        .clone(),
                );
            }
            for (asset, owner) in &owners {
                if let crate::graphics::PictureGraphicSource::ComponentNode {
                    instance_id,
                    node_id,
                } = owner
                {
                    if *instance_id == id.0 {
                        let object_id = instance.compiled_ids[&ComponentNodeId(*node_id)];
                        let object = pool
                            .object_mut_by_id(
                                ag_iso_stack::object_pool::ObjectId::new(object_id).unwrap(),
                            )
                            .unwrap();
                        *object = raw.pool.object_by_id(object.id()).unwrap().clone();
                        graphics
                            .picture_graphic_bindings
                            .insert(object_id, (*asset).into());
                    }
                }
            }
            let root = instance.compiled_ids[&source.definitions[&instance.component_id].root_node];
            let EditorParentRef::Primitive(parent) = instance.parent;
            crate::Operation::AddChild {
                parent: crate::ObjectTarget::id(parent),
                child: crate::ObjectTarget::id(root),
                x: i16::try_from(instance.x).unwrap(),
                y: i16::try_from(instance.y).unwrap(),
            }
            .apply(pool, &mut Default::default(), &mut Default::default())
            .map_err(|e| ComponentError::new("COMPONENT_MATERIALIZATION_FAILED", e.to_string()))?;
            source.instances.retain(|i| i.id != id);
            let order = crate::operations::object_refs(
                materialized
                    .pool
                    .object_by_id(ag_iso_stack::object_pool::ObjectId::new(parent).unwrap())
                    .unwrap(),
            )
            .unwrap();
            let children = order
                .iter()
                .filter(|r| pool.object_by_id(r.id).is_some())
                .map(|r| crate::ChildPlacement {
                    child: crate::ObjectTarget::id(r.id.value()),
                    x: r.offset.x,
                    y: r.offset.y,
                })
                .collect();
            crate::Operation::SetChildren {
                parent: crate::ObjectTarget::id(parent),
                children,
            }
            .apply(pool, &mut Default::default(), &mut Default::default())
            .map_err(|e| ComponentError::new("COMPONENT_MATERIALIZATION_FAILED", e.to_string()))?;
            Ok(())
        })
    }
    pub fn delete_definition(project: &mut EditorProject, id: ComponentId) -> ComponentResult<()> {
        project.edit_components("Delete component", |_, source| {
            if source.instances.iter().any(|i| i.component_id == id) {
                return Err(ComponentError::new(
                    "COMPONENT_IN_USE",
                    "Detach or delete instances before deleting their definition",
                ));
            }
            source.definitions.remove(&id);
            Ok(())
        })
    }
    pub fn duplicate_definition(
        project: &mut EditorProject,
        id: ComponentId,
    ) -> ComponentResult<ComponentId> {
        let mut copy = definition(project, id)?;
        copy.id = ComponentId::default();
        copy.name.push_str(" copy");
        let id = copy.id;
        Self::edit_component_definition(project, copy)?;
        Ok(id)
    }
}
fn definition(project: &EditorProject, id: ComponentId) -> ComponentResult<ComponentDefinition> {
    project
        .component_source()
        .definitions
        .get(&id)
        .cloned()
        .ok_or_else(|| ComponentError::new("COMPONENT_NOT_FOUND", "Unknown component"))
}
fn instance_mut(
    source: &mut ComponentSource,
    id: ComponentInstanceId,
) -> ComponentResult<&mut ComponentInstance> {
    source
        .instances
        .iter_mut()
        .find(|i| i.id == id)
        .ok_or_else(|| ComponentError::new("COMPONENT_NOT_FOUND", "Unknown component instance"))
}

fn insertion_index(
    pool: &ag_iso_stack::object_pool::ObjectPool,
    source: &ComponentSource,
    parent: u16,
) -> usize {
    pool.object_by_id(ag_iso_stack::object_pool::ObjectId::new(parent).unwrap_or_default())
        .and_then(crate::operations::object_refs)
        .map_or(0, |r| r.len())
        + source
            .instances
            .iter()
            .filter(|i| i.parent == EditorParentRef::Primitive(parent))
            .count()
}
