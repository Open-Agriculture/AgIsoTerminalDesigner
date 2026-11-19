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

/// Resolve a materialized PictureGraphic by its authoring name, including
/// ones generated by component instances (`<instance>/<node>`).
pub fn picture_graphic_id(project: &EditorProject, name: &str) -> Result<u16, String> {
    let ordinary = project
        .get_pool()
        .objects_by_type(ObjectType::PictureGraphic)
        .iter()
        .map(|object| {
            (
                project.get_object_info(object).get_name(object),
                object.id().value(),
            )
        })
        .collect::<Vec<_>>();
    resolve("PictureGraphic", name, ordinary.clone()).or_else(|error| {
        let id = unique_component_object(project, name, error.clone())?;
        let is_picture = project.materialize_shared().is_ok_and(|materialized| {
            ag_iso_stack::object_pool::ObjectId::new(id)
                .ok()
                .and_then(|id| materialized.pool.object_by_id(id))
                .is_some_and(|object| object.object_type() == ObjectType::PictureGraphic)
        });
        if is_picture {
            Ok(id)
        } else {
            Err(error)
        }
    })
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
    .map_err(|error| component_object_guidance(project, name).unwrap_or(error))
}

/// For a name of a component instance object: why edit commands cannot use
/// it (it is rebuilt from its component) and where to change it instead.
pub(crate) fn component_object_guidance(project: &EditorProject, name: &str) -> Option<String> {
    let (_, instance) = component_object(project, name)?;
    Some(format!(
        "'{name}' belongs to component instance '{instance}' and is rebuilt from its component, \
         so commands that change it or point to it cannot use it. Change it per instance with \
         `component override-set`, or for all instances in the master (`component node-set`, \
         `node-reference`, `node-children-set`)"
    ))
}

/// Resolve an object name for inspection, including objects generated by
/// component instances (`<instance>/<node>`). Ordinary names win.
pub fn readable_object_id(project: &EditorProject, name: &str) -> Result<u16, String> {
    object_id(project, name).or_else(|error| unique_component_object(project, name, error))
}

/// The one component object named `name`; `missing` when there is none.
fn unique_component_object(
    project: &EditorProject,
    name: &str,
    missing: String,
) -> Result<u16, String> {
    match component_objects(project, name).as_slice() {
        [] => Err(missing),
        [(id, _)] => Ok(*id),
        _ => Err(format!("Ambiguous object name '{name}'")),
    }
}

/// The ID and instance name of each component object named `<instance>/<node>`.
/// Instance and node names may themselves contain `/`, so two paths can be
/// equal; callers reject more than one match.
fn component_objects(project: &EditorProject, name: &str) -> Vec<(u16, String)> {
    let wanted = slash_normalized(name);
    let instances = &project.component_source().instances;
    project
        .component_object_names()
        .into_iter()
        .filter(|(_, candidate)| slash_normalized(candidate) == wanted)
        .map(|(id, full)| {
            let instance = instances
                .iter()
                .map(|instance| instance.name.clone())
                .find(|instance| full.starts_with(&format!("{instance}/")))
                .unwrap_or(full);
            (id, instance)
        })
        .collect()
}

fn component_object(project: &EditorProject, name: &str) -> Option<(u16, String)> {
    component_objects(project, name).into_iter().next()
}

/// Normal name matching, also ignoring separators around `/`, so
/// `Panel 1 / Label` matches `Panel 1/Label`.
fn slash_normalized(name: &str) -> String {
    smart_naming::normalized_name(name)
        .replace("-/", "/")
        .replace("/-", "/")
}
