//! Name-based CLI lookup for project authoring entities.
use crate::components::{ComponentId, ComponentInstanceId, ComponentNodeId, ComponentParameterId};
use crate::graphics::GraphicAssetId;
use crate::{smart_naming, EditorProject};
use ag_iso_stack::object_pool::ObjectType;

fn resolve<T: Copy>(
    entity: &str,
    name: &str,
    candidates: impl IntoIterator<Item = (String, T)>,
) -> Result<T, String> {
    let wanted = smart_naming::normalized_name(name);
    if wanted.is_empty() {
        return Err(format!("A {entity} name cannot be empty"));
    }
    let mut matches = candidates
        .into_iter()
        .filter(|(candidate, _)| smart_naming::normalized_name(candidate) == wanted)
        .map(|(_, id)| id);
    let Some(found) = matches.next() else {
        return Err(format!("No {entity} named '{name}'"));
    };
    if matches.next().is_some() {
        return Err(format!("Ambiguous {entity} name '{name}'"));
    }
    Ok(found)
}

/// Resolve a component name using case- and separator-insensitive matching.
pub fn component_id(project: &EditorProject, name: &str) -> Result<ComponentId, String> {
    resolve(
        "component",
        name,
        project
            .component_source()
            .definitions
            .values()
            .map(|definition| (definition.name.clone(), definition.id)),
    )
}

/// Resolve a component instance name, reporting collisions as ambiguous.
pub fn instance_id(project: &EditorProject, name: &str) -> Result<ComponentInstanceId, String> {
    resolve(
        "component instance",
        name,
        project
            .component_source()
            .instances
            .iter()
            .map(|instance| (instance.name.clone(), instance.id)),
    )
}

/// Resolve a node name within the named component definition.
pub fn node_id(
    project: &EditorProject,
    component_name: &str,
    node_name: &str,
) -> Result<ComponentNodeId, String> {
    let component = component_id(project, component_name)?;
    let definition = project
        .component_source()
        .definitions
        .get(&component)
        .ok_or_else(|| format!("No component named '{component_name}'"))?;
    resolve(
        "component node",
        node_name,
        definition
            .nodes
            .iter()
            .map(|node| (node.name.clone(), node.id)),
    )
}

/// Resolve a node name within the definition used by a named instance.
pub fn node_id_for_instance(
    project: &EditorProject,
    instance_name: &str,
    node_name: &str,
) -> Result<ComponentNodeId, String> {
    let instance_id = instance_id(project, instance_name)?;
    let instance = project
        .component_source()
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| format!("No component instance named '{instance_name}'"))?;
    let definition = project
        .component_source()
        .definitions
        .get(&instance.component_id)
        .ok_or_else(|| format!("Instance '{instance_name}' refers to a missing component"))?;
    resolve(
        "component node",
        node_name,
        definition
            .nodes
            .iter()
            .map(|node| (node.name.clone(), node.id)),
    )
}

/// Resolve a parameter name within the named component definition.
pub fn parameter_id(
    project: &EditorProject,
    component_name: &str,
    parameter_name: &str,
) -> Result<ComponentParameterId, String> {
    let component = component_id(project, component_name)?;
    let definition = project
        .component_source()
        .definitions
        .get(&component)
        .ok_or_else(|| format!("No component named '{component_name}'"))?;
    resolve(
        "component parameter",
        parameter_name,
        definition
            .parameters
            .iter()
            .map(|parameter| (parameter.name.clone(), parameter.id)),
    )
}

/// Resolve a parameter name within the definition used by a named instance.
pub fn parameter_id_for_instance(
    project: &EditorProject,
    instance_name: &str,
    parameter_name: &str,
) -> Result<ComponentParameterId, String> {
    let instance_id = instance_id(project, instance_name)?;
    let instance = project
        .component_source()
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| format!("No component instance named '{instance_name}'"))?;
    let definition = project
        .component_source()
        .definitions
        .get(&instance.component_id)
        .ok_or_else(|| format!("Instance '{instance_name}' refers to a missing component"))?;
    resolve(
        "component parameter",
        parameter_name,
        definition
            .parameters
            .iter()
            .map(|parameter| (parameter.name.clone(), parameter.id)),
    )
}

/// Resolve the name of an available, unmaterialized graphic asset.
pub fn graphic_asset_id(project: &EditorProject, name: &str) -> Result<GraphicAssetId, String> {
    resolve(
        "available graphic",
        name,
        project
            .graphic_library()
            .assets
            .values()
            .map(|asset| (asset.name.clone(), asset.id)),
    )
}

/// Resolve a materialized PictureGraphic by its authoring name.
pub fn picture_graphic_id(project: &EditorProject, name: &str) -> Result<u16, String> {
    resolve(
        "PictureGraphic",
        name,
        project
            .get_pool()
            .objects_by_type(ObjectType::PictureGraphic)
            .iter()
            .map(|object| {
                (
                    project.get_object_info(object).get_name(object),
                    object.id().value(),
                )
            }),
    )
}

/// Resolve an ordinary pool object by its authoring name.
pub fn object_id(project: &EditorProject, name: &str) -> Result<u16, String> {
    resolve(
        "object",
        name,
        project.get_pool().objects().iter().map(|object| {
            (
                project.get_object_info(object).get_name(object),
                object.id().value(),
            )
        }),
    )
}
