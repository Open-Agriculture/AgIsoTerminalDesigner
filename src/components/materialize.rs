use super::*;
use crate::object_properties::{get_object_property, get_property_descriptors};
use crate::operations::{resolve_target, OperationContext};
use crate::{EditorProject, ObjectTarget, Operation, PropertySemantic};
use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectId, ObjectPool};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// Same lowest-free, non-null allocation order used by primitive CreateObject.
pub fn prepare_compiled_ids(
    pool: &ObjectPool,
    source: &mut ComponentSource,
) -> ComponentResult<()> {
    let mut used: BTreeSet<u16> = pool.objects().iter().map(|o| o.id().value()).collect();
    source.instances.sort_by_key(|i| i.id);
    for instance in &mut source.instances {
        let definition = source
            .definitions
            .get(&instance.component_id)
            .ok_or_else(|| {
                ComponentError::new("COMPONENT_NOT_FOUND", "Instance definition is missing")
            })?;
        instance
            .compiled_ids
            .retain(|node, _| definition.nodes.iter().any(|n| n.id == *node));
        for id in instance.compiled_ids.values() {
            if *id == u16::MAX || !used.insert(*id) {
                return Err(ComponentError::new(
                    "COMPONENT_COMPILED_ID_COLLISION",
                    format!("VT ID {id} is already reserved"),
                ));
            }
        }
    }
    for instance in &mut source.instances {
        let mut nodes: Vec<_> = source.definitions[&instance.component_id]
            .nodes
            .iter()
            .map(|n| n.id)
            .collect();
        nodes.sort();
        for node in nodes {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                instance.compiled_ids.entry(node)
            {
                let id = crate::operations::operation::first_available_object_id(|id| {
                    used.contains(&id)
                })
                .ok_or_else(|| {
                    ComponentError::new("COMPONENT_COMPILED_ID_COLLISION", "VT ID space exhausted")
                })?;
                used.insert(id);
                entry.insert(id);
            }
        }
    }
    Ok(())
}

/// Compile a standalone preview using the same node compiler and ID allocation
/// as instances. No instance or history entry is added to the user's project.
pub(crate) fn preview_definition(
    project: &EditorProject,
    definition: &ComponentDefinition,
) -> ComponentResult<(ObjectPool, std::collections::BTreeMap<ComponentNodeId, u16>)> {
    validate_definition(definition)?;
    let mut source = ComponentSource::default();
    source.definitions.insert(definition.id, definition.clone());
    source.instances.push(ComponentInstance {
        id: ComponentInstanceId::default(),
        component_id: definition.id,
        parent: EditorParentRef::Primitive(0),
        child_index: None,
        x: 0,
        y: 0,
        overrides: Default::default(),
        compiled_ids: Default::default(),
    });
    prepare_compiled_ids(project.get_pool(), &mut source)?;
    let instance = &source.instances[0];
    let mut pool = project.get_pool().clone();
    compile_nodes(
        &mut pool,
        project.get_pool(),
        &project.component_object_info(),
        definition,
        instance,
        project.vt_version,
    )?;
    Ok((pool, instance.compiled_ids.clone()))
}

pub fn materialize_project(project: &EditorProject) -> ComponentResult<MaterializedProject> {
    materialize_source_with_graphics(
        project.get_pool(),
        project.component_source(),
        project.vt_version,
        &project.component_object_info(),
        project.graphic_library(),
    )
}

pub fn materialize_source(
    canonical: &ObjectPool,
    source: &ComponentSource,
    version: VtVersion,
    info: &HashMap<ObjectId, crate::ObjectInfo>,
) -> ComponentResult<MaterializedProject> {
    materialize_source_with_graphics(canonical, source, version, info, &Default::default())
}

pub fn materialize_source_with_graphics(
    canonical: &ObjectPool,
    source: &ComponentSource,
    version: VtVersion,
    info: &HashMap<ObjectId, crate::ObjectInfo>,
    graphics: &crate::graphics::GraphicLibrary,
) -> ComponentResult<MaterializedProject> {
    let mut result = MaterializedProject {
        pool: canonical.clone(),
        source_map: MaterializedSourceMap::default(),
    };
    let mut reserved = BTreeSet::new();
    for object in canonical.objects() {
        reserved.insert(object.id().value());
        result.source_map.by_object_id.insert(
            object.id().value(),
            MaterializedSource::Primitive {
                object_id: object.id().value(),
            },
        );
    }
    for (key, definition) in &source.definitions {
        if *key != definition.id {
            return Err(ComponentError::new(
                "COMPONENT_NODE_INVALID",
                "Definition key does not match its identity",
            ));
        }
        validate_definition(definition).map_err(|mut error| {
            error.component_id = Some(definition.id);
            error
        })?;
    }
    let mut instances: Vec<_> = source.instances.iter().collect();
    instances.sort_by_key(|i| i.id);
    let mut instance_ids = BTreeSet::new();
    for instance in &instances {
        if !instance_ids.insert(instance.id) {
            return Err(ComponentError::new(
                "COMPONENT_NODE_INVALID",
                "Duplicate instance identity",
            ));
        }
        let definition = source
            .definitions
            .get(&instance.component_id)
            .ok_or_else(|| {
                ComponentError::new("COMPONENT_NOT_FOUND", "Instance definition is missing")
            })?;
        if instance.compiled_ids.len() != definition.nodes.len()
            || definition
                .nodes
                .iter()
                .any(|n| !instance.compiled_ids.contains_key(&n.id))
        {
            return Err(ComponentError::new("COMPONENT_NODE_INVALID", "Compiled ID reservations must match current definition nodes; prepare source state first"));
        }
        for (node, id) in &instance.compiled_ids {
            if *id == u16::MAX || !reserved.insert(*id) {
                return Err(ComponentError::new(
                    "COMPONENT_COMPILED_ID_COLLISION",
                    format!("VT ID {id} is already reserved"),
                ));
            }
            result.source_map.by_object_id.insert(
                *id,
                MaterializedSource::ComponentNode {
                    instance_id: instance.id,
                    node_id: *node,
                },
            );
            result
                .source_map
                .by_component_node
                .insert((instance.id, *node), *id);
        }
        for (parameter, value) in &instance.overrides {
            let p = definition
                .parameters
                .iter()
                .find(|p| p.id == *parameter)
                .ok_or_else(|| {
                    ComponentError::new("COMPONENT_PARAMETER_INVALID", "Unknown override parameter")
                })?;
            validate_value(p, value)?;
        }
    }
    for object in canonical.objects() {
        for reference in object.referenced_objects() {
            if matches!(
                result.source_map.by_object_id.get(&reference.value()),
                Some(MaterializedSource::ComponentNode { .. })
            ) {
                return Err(ComponentError::new(
                    "COMPONENT_EXTERNAL_REFERENCE_INVALID",
                    format!(
                        "Ordinary object {} references encapsulated component node {}",
                        object.id().value(),
                        reference.value()
                    ),
                ));
            }
        }
    }
    for instance in instances {
        let definition = &source.definitions[&instance.component_id];
        compile_nodes(
            &mut result.pool,
            canonical,
            info,
            definition,
            instance,
            version,
        )?;
        let EditorParentRef::Primitive(parent) = instance.parent;
        if canonical
            .object_by_id(
                ObjectId::new(parent).map_err(|_| {
                    ComponentError::new("COMPONENT_NODE_INVALID", "Invalid parent ID")
                })?,
            )
            .is_none()
        {
            return Err(ComponentError::new(
                "COMPONENT_NODE_INVALID",
                "Instance parent must be a canonical ordinary object",
            ));
        }
        apply(
            &mut result.pool,
            version,
            Operation::AddChild {
                parent: ObjectTarget::id(parent),
                child: ObjectTarget::id(instance.compiled_ids[&definition.root_node]),
                x: position(instance.x)?,
                y: position(instance.y)?,
            },
        )
        .map_err(|e| locate(e, definition, instance, definition.root_node, None))?;
    }
    // Merge virtual roots into the parent's ordered display list. Detach preserves z-order.
    let parents: BTreeSet<_> = source
        .instances
        .iter()
        .map(|i| {
            let EditorParentRef::Primitive(parent) = i.parent;
            parent
        })
        .collect();
    for parent in parents {
        let mut children: Vec<_> = crate::operations::object_refs(
            canonical
                .object_by_id(ObjectId::new(parent).unwrap())
                .unwrap(),
        )
        .unwrap_or_default()
        .iter()
        .map(|r| crate::ChildPlacement {
            child: ObjectTarget::id(r.id.value()),
            x: r.offset.x,
            y: r.offset.y,
        })
        .collect();
        let mut roots: Vec<_> = source
            .instances
            .iter()
            .filter(|i| i.parent == EditorParentRef::Primitive(parent))
            .collect();
        roots.sort_by_key(|i| (i.child_index.unwrap_or(usize::MAX), i.id));
        for instance in roots {
            let root = source.definitions[&instance.component_id].root_node;
            let placement = crate::ChildPlacement {
                child: ObjectTarget::id(instance.compiled_ids[&root]),
                x: position(instance.x)?,
                y: position(instance.y)?,
            };
            children.insert(
                instance
                    .child_index
                    .unwrap_or(children.len())
                    .min(children.len()),
                placement,
            );
        }
        apply(
            &mut result.pool,
            version,
            Operation::SetChildren {
                parent: ObjectTarget::id(parent),
                children,
            },
        )?;
    }
    // Validate master defaults too, even when every live instance overrides them.
    // Compile into a disposable pool without root attachment.
    for definition in source.definitions.values() {
        let mut probe = ComponentSource::default();
        probe.definitions.insert(definition.id, definition.clone());
        probe.instances.push(ComponentInstance {
            id: ComponentInstanceId::default(),
            component_id: definition.id,
            parent: EditorParentRef::Primitive(0),
            child_index: None,
            x: 0,
            y: 0,
            overrides: Default::default(),
            compiled_ids: Default::default(),
        });
        prepare_compiled_ids(canonical, &mut probe)?;
        let mut pool = canonical.clone();
        compile_nodes(
            &mut pool,
            canonical,
            info,
            definition,
            &probe.instances[0],
            version,
        )?;
        validate_pool(&pool, version, &MaterializedSourceMap::default(), source)?;
    }
    crate::graphics::integration::apply(source, graphics, &mut result.pool)?;
    validate_pool(&result.pool, version, &result.source_map, source)?;
    Ok(result)
}
fn position(value: i32) -> ComponentResult<i16> {
    i16::try_from(value).map_err(|_| {
        ComponentError::new(
            "COMPONENT_NODE_INVALID",
            "Position exceeds VT signed 16-bit range",
        )
    })
}
fn apply(pool: &mut ObjectPool, _version: VtVersion, operation: Operation) -> ComponentResult<()> {
    operation
        .apply(
            pool,
            &mut HashMap::new(),
            &mut OperationContext {
                // Materialization preserves authoring drafts. The target's
                // restrictions are checked on the completed pool, so an
                // incompatible imported component remains available to repair.
                vt_version: VtVersion::Version6,
                ..Default::default()
            },
        )
        .map(|_| ())
        .map_err(|e| ComponentError::new("COMPONENT_MATERIALIZATION_FAILED", e.to_string()))
}
fn locate(
    mut error: ComponentError,
    definition: &ComponentDefinition,
    instance: &ComponentInstance,
    node: ComponentNodeId,
    property: Option<&str>,
) -> ComponentError {
    error.component_id = Some(definition.id);
    error.instance_id = Some(instance.id);
    error.node_id = Some(node);
    error.property = property.map(str::to_owned);
    error.object_id = instance.compiled_ids.get(&node).copied();
    error.message = format!(
        "{} / {}: {}",
        definition.name,
        definition
            .nodes
            .iter()
            .find(|n| n.id == node)
            .map(|n| n.name.as_str())
            .unwrap_or("?"),
        error.message
    );
    error
}
fn compile_nodes(
    pool: &mut ObjectPool,
    canonical: &ObjectPool,
    info: &HashMap<ObjectId, crate::ObjectInfo>,
    definition: &ComponentDefinition,
    instance: &ComponentInstance,
    version: VtVersion,
) -> ComponentResult<()> {
    let mut nodes: Vec<_> = definition.nodes.iter().collect();
    nodes.sort_by_key(|n| n.id);
    for node in &nodes {
        // Compilation restores authoring content, including incompatible
        // imported drafts. Version compatibility is checked at edit/export
        // boundaries rather than discarding content during materialization.
        let mut object = crate::default_object(node.object_type);
        object
            .mut_id()
            .set_value(instance.compiled_ids[&node.id])
            .map_err(|error| ComponentError::new("COMPONENT_NODE_INVALID", format!("{error:?}")))?;
        apply(
            pool,
            version,
            Operation::RestoreObject { object, info: None },
        )
        .map_err(|e| locate(e, definition, instance, node.id, None))?;
    }
    for node in nodes {
        let id = instance.compiled_ids[&node.id];
        for (property, value) in &node.properties {
            apply(
                pool,
                version,
                Operation::SetProperty {
                    object: ObjectTarget::id(id),
                    property: property.clone(),
                    value: value.clone(),
                },
            )
            .map_err(|e| locate(e, definition, instance, node.id, Some(property)))?;
        }
        for (property, reference) in &node.references {
            let target = match reference {
                ComponentReference::Internal(node) => {
                    Some(instance.compiled_ids.get(node).copied().ok_or_else(|| {
                        ComponentError::new(
                            "COMPONENT_INTERNAL_REFERENCE_INVALID",
                            "Missing internal node",
                        )
                    })?)
                }
                ComponentReference::ExternalObject(target) => Some(
                    resolve_target(target, canonical, info)
                        .map_err(|e| {
                            locate(
                                ComponentError::new(
                                    "COMPONENT_EXTERNAL_REFERENCE_INVALID",
                                    e.to_string(),
                                ),
                                definition,
                                instance,
                                node.id,
                                Some(property),
                            )
                        })?
                        .value(),
                ),
                ComponentReference::Parameter(parameter) => {
                    let p = definition
                        .parameters
                        .iter()
                        .find(|p| p.id == *parameter)
                        .ok_or_else(|| {
                            ComponentError::new(
                                "COMPONENT_PARAMETER_INVALID",
                                "Unknown reference parameter",
                            )
                        })?;
                    external_value(
                        instance
                            .overrides
                            .get(parameter)
                            .unwrap_or(&p.default_value),
                        canonical,
                        info,
                    )?
                }
                ComponentReference::Null => None,
            };
            apply(
                pool,
                version,
                Operation::SetObjectReference {
                    object: ObjectTarget::id(id),
                    property: property.clone(),
                    reference: target.map(ObjectTarget::id),
                },
            )
            .map_err(|e| locate(e, definition, instance, node.id, Some(property)))?;
        }
        for parameter in definition
            .parameters
            .iter()
            .filter(|p| p.target_node == node.id)
        {
            let value = instance
                .overrides
                .get(&parameter.id)
                .unwrap_or(&parameter.default_value);
            if parameter.kind == ComponentParameterKind::Graphic {
                continue;
            }
            let op = if parameter.kind == ComponentParameterKind::ObjectReference {
                Operation::SetObjectReference {
                    object: ObjectTarget::id(id),
                    property: parameter.property.clone(),
                    reference: external_value(value, canonical, info)
                        .map_err(|e| {
                            locate(e, definition, instance, node.id, Some(&parameter.property))
                        })?
                        .map(ObjectTarget::id),
                }
            } else {
                Operation::SetProperty {
                    object: ObjectTarget::id(id),
                    property: parameter.property.clone(),
                    value: value.clone(),
                }
            };
            apply(pool, version, op)
                .map_err(|e| locate(e, definition, instance, node.id, Some(&parameter.property)))?;
        }
        if let Some(list) = &node.object_list {
            let resolve = |target: &ObjectTarget| -> ComponentResult<ObjectTarget> {
                resolve_target(target, canonical, info)
                    .map(|id| ObjectTarget::id(id.value()))
                    .map_err(|e| {
                        locate(
                            ComponentError::new(
                                "COMPONENT_EXTERNAL_REFERENCE_INVALID",
                                e.to_string(),
                            ),
                            definition,
                            instance,
                            node.id,
                            Some("object_list"),
                        )
                    })
            };
            let objects = match list {
                ComponentObjectList::Required(ids) => crate::ObjectTargetList::Required(
                    ids.iter().map(resolve).collect::<ComponentResult<_>>()?,
                ),
                ComponentObjectList::Nullable(ids) => crate::ObjectTargetList::Nullable(
                    ids.iter()
                        .map(|id| id.as_ref().map(resolve).transpose())
                        .collect::<ComponentResult<_>>()?,
                ),
            };
            apply(
                pool,
                version,
                Operation::SetObjectList {
                    object: ObjectTarget::id(id),
                    objects,
                },
            )
            .map_err(|e| locate(e, definition, instance, node.id, Some("object_list")))?;
        }
        if !node.macro_references.is_empty() {
            let mut macro_refs = Vec::new();
            for reference in &node.macro_references {
                let id = resolve_target(&reference.target, canonical, info).map_err(|e| {
                    locate(
                        ComponentError::new("COMPONENT_EXTERNAL_REFERENCE_INVALID", e.to_string()),
                        definition,
                        instance,
                        node.id,
                        Some("macro_refs"),
                    )
                })?;
                macro_refs.push(crate::MacroReferenceTarget {
                    event_id: reference.event,
                    macro_object: ObjectTarget::id(id.value()),
                });
            }
            apply(
                pool,
                version,
                Operation::SetMacroReferences {
                    object: ObjectTarget::id(id),
                    macro_refs,
                },
            )
            .map_err(|e| locate(e, definition, instance, node.id, Some("macro_refs")))?;
        }
        for child in &node.children {
            apply(
                pool,
                version,
                Operation::AddChild {
                    parent: ObjectTarget::id(id),
                    child: ObjectTarget::id(instance.compiled_ids[&child.child]),
                    x: position(child.x)?,
                    y: position(child.y)?,
                },
            )
            .map_err(|e| locate(e, definition, instance, node.id, None))?;
        }
    }
    Ok(())
}
fn external_value(
    value: &Value,
    pool: &ObjectPool,
    info: &HashMap<ObjectId, crate::ObjectInfo>,
) -> ComponentResult<Option<u16>> {
    if value.is_null() {
        return Ok(None);
    }
    let target: ObjectTarget =
        if let Some(id) = value.as_u64().and_then(|id| u16::try_from(id).ok()) {
            ObjectTarget::id(id)
        } else {
            serde_json::from_value(value.clone()).map_err(|_| {
                ComponentError::new(
                    "COMPONENT_EXTERNAL_REFERENCE_INVALID",
                    "Expected ordinary object target or null",
                )
            })?
        };
    resolve_target(&target, pool, info)
        .map(|id| Some(id.value()))
        .map_err(|e| ComponentError::new("COMPONENT_EXTERNAL_REFERENCE_INVALID", e.to_string()))
}
pub fn validate_definition(definition: &ComponentDefinition) -> ComponentResult<()> {
    let invalid = |message| ComponentError::new("COMPONENT_NODE_INVALID", message);
    if definition.schema_version != 1 {
        return Err(invalid("Unsupported component schema"));
    }
    let ids: BTreeSet<_> = definition.nodes.iter().map(|n| n.id).collect();
    if ids.len() != definition.nodes.len() || !ids.contains(&definition.root_node) {
        return Err(invalid("Missing root or duplicate node identity"));
    }
    let mut parameters = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for p in &definition.parameters {
        if !parameters.insert(p.id)
            || !names.insert(&p.name)
            || p.name.trim().is_empty()
            || !targets.insert((p.target_node, &p.property))
        {
            return Err(ComponentError::new(
                "COMPONENT_PARAMETER_INVALID",
                "Duplicate parameter identity, name or target",
            ));
        }
        let node = definition
            .nodes
            .iter()
            .find(|n| n.id == p.target_node)
            .ok_or_else(|| invalid("Parameter node missing"))?;
        if p.kind == ComponentParameterKind::Graphic {
            if node.object_type != ag_iso_stack::object_pool::ObjectType::PictureGraphic
                || p.property != "graphic"
            {
                return Err(ComponentError::new(
                    "COMPONENT_PARAMETER_INVALID",
                    "Graphic parameters must target a PictureGraphic node with property graphic",
                ));
            }
            validate_value(p, &p.default_value)?;
            continue;
        }
        let descriptor = get_property_descriptors(node.object_type)
            .into_iter()
            .find(|d| d.name == p.property && d.writable)
            .ok_or_else(|| {
                ComponentError::new(
                    "COMPONENT_PARAMETER_INVALID",
                    "Parameter property is not writable",
                )
            })?;
        let compatible = match descriptor.semantic {
            PropertySemantic::ObjectReference { .. } => {
                p.kind == ComponentParameterKind::ObjectReference
            }
            PropertySemantic::Colour => p.kind == ComponentParameterKind::Colour,
            PropertySemantic::Enum => p.kind == ComponentParameterKind::Enum,
            _ => {
                let value =
                    get_object_property(&crate::default_object(node.object_type), &p.property)
                        .map_err(|_| invalid("Unsupported property"))?;
                match p.kind {
                    ComponentParameterKind::Graphic => {
                        serde_json::from_value::<crate::graphics::GraphicAssetId>(value.clone())
                            .is_ok()
                    }
                    ComponentParameterKind::Text => value.is_string(),
                    ComponentParameterKind::Number => value.is_number(),
                    ComponentParameterKind::Boolean => {
                        value.is_boolean()
                            || (node.object_type
                                == ag_iso_stack::object_pool::ObjectType::InputBoolean
                                && p.property == "value")
                    }
                    _ => false,
                }
            }
        };
        if !compatible {
            return Err(ComponentError::new(
                "COMPONENT_PARAMETER_INVALID",
                "Parameter kind does not match authoritative property descriptor",
            ));
        }
        validate_value(p, &p.default_value)?;
    }
    for node in &definition.nodes {
        for child in &node.children {
            if !ids.contains(&child.child) {
                return Err(invalid("Child node missing"));
            }
        }
        for (property, reference) in &node.references {
            if node.properties.contains_key(property) {
                return Err(invalid("Reference cannot also be a raw property"));
            }
            match reference {
                ComponentReference::Internal(id) if !ids.contains(id) => {
                    return Err(invalid("Internal reference node missing"))
                }
                ComponentReference::Parameter(id) => {
                    if !definition.parameters.iter().any(|p| {
                        p.id == *id
                            && p.kind == ComponentParameterKind::ObjectReference
                            && p.target_node == node.id
                            && p.property == *property
                    }) {
                        return Err(invalid("Reference parameter must target this property"));
                    }
                }
                _ => {}
            }
        }
    }
    fn visit(
        id: ComponentNodeId,
        definition: &ComponentDefinition,
        active: &mut BTreeSet<ComponentNodeId>,
        done: &mut BTreeSet<ComponentNodeId>,
    ) -> ComponentResult<()> {
        if done.contains(&id) {
            return Ok(());
        }
        if !active.insert(id) {
            return Err(ComponentError::new(
                "COMPONENT_INTERNAL_REFERENCE_INVALID",
                "Cycle in component nodes",
            ));
        }
        let node = definition.nodes.iter().find(|n| n.id == id).unwrap();
        for child in
            node.children
                .iter()
                .map(|c| c.child)
                .chain(node.references.values().filter_map(|r| {
                    if let ComponentReference::Internal(id) = r {
                        Some(*id)
                    } else {
                        None
                    }
                }))
        {
            visit(child, definition, active, done)?;
        }
        active.remove(&id);
        done.insert(id);
        Ok(())
    }
    let mut done = BTreeSet::new();
    for node in &definition.nodes {
        visit(node.id, definition, &mut BTreeSet::new(), &mut done)?;
    }
    Ok(())
}
fn validate_value(parameter: &ComponentParameter, value: &Value) -> ComponentResult<()> {
    let valid = match parameter.kind {
        ComponentParameterKind::Graphic => value.as_u64().is_some_and(|id| id < 65535),
        ComponentParameterKind::Text => {
            serde_json::from_value::<ag_iso_stack::object_pool::object::VtString>(value.clone())
                .is_ok()
        }
        ComponentParameterKind::Number => value.is_number(),
        ComponentParameterKind::Boolean => value.is_boolean(),
        ComponentParameterKind::Colour => value.as_u64().is_some_and(|n| n <= 255),
        ComponentParameterKind::ObjectReference => {
            value.is_null()
                || value.as_u64().is_some_and(|n| n < 65535)
                || serde_json::from_value::<ObjectTarget>(value.clone()).is_ok()
        }
        ComponentParameterKind::Enum => !value.is_null(),
    };
    if valid {
        Ok(())
    } else {
        Err(ComponentError::new(
            "COMPONENT_OVERRIDE_TYPE_MISMATCH",
            format!("Invalid value for parameter {}", parameter.name),
        ))
    }
}
fn validate_pool(
    pool: &ObjectPool,
    version: VtVersion,
    map: &MaterializedSourceMap,
    source: &ComponentSource,
) -> ComponentResult<()> {
    for diagnostic in crate::pool_validation::validate_pool_state_for_version(pool, version) {
        if diagnostic.severity == crate::ValidationSeverity::Error
            && !diagnostic.code.starts_with("VT_")
        {
            let mut error =
                ComponentError::new("COMPONENT_MATERIALIZATION_FAILED", diagnostic.message);
            for id in diagnostic.affected_objects.unwrap_or_default() {
                if let Some(MaterializedSource::ComponentNode {
                    instance_id,
                    node_id,
                }) = map.by_object_id.get(&id)
                {
                    let instance = source
                        .instances
                        .iter()
                        .find(|i| i.id == *instance_id)
                        .unwrap();
                    error = locate(
                        error,
                        &source.definitions[&instance.component_id],
                        instance,
                        *node_id,
                        None,
                    );
                    break;
                }
            }
            return Err(error);
        }
    }
    Ok(())
}
pub fn component_bounds(definition: &ComponentDefinition) -> Option<(u16, u16)> {
    let root = definition
        .nodes
        .iter()
        .find(|n| n.id == definition.root_node)?;
    Some((
        u16::try_from(root.properties.get("width")?.as_u64()?).ok()?,
        u16::try_from(root.properties.get("height")?.as_u64()?).ok()?,
    ))
}

/// Source reservations also permit semantic diagnostics without recompiling geometry.
pub fn source_map(source: &ComponentSource) -> MaterializedSourceMap {
    let mut map = MaterializedSourceMap::default();
    for instance in &source.instances {
        for (node, id) in &instance.compiled_ids {
            map.by_object_id.insert(
                *id,
                MaterializedSource::ComponentNode {
                    instance_id: instance.id,
                    node_id: *node,
                },
            );
            map.by_component_node.insert((instance.id, *node), *id);
        }
    }
    map
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use ag_iso_stack::object_pool::{object::Object, ObjectType};

    #[test]
    fn standalone_preview_uses_normal_compiler_without_project_mutations() {
        let mut pool = ObjectPool::default();
        let mut button = crate::default_object(ObjectType::Button);
        button.mut_id().set_value(100).unwrap();
        if let Object::Button(button) = &mut button {
            button.width = 140;
            button.height = 40;
        }
        pool.add(button);
        let mut mask = crate::default_object(ObjectType::DataMask);
        mask.mut_id().set_value(2).unwrap();
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
                id: ObjectId::new(100).unwrap(),
                offset: Default::default(),
            });
        }
        pool.add(mask);
        let project = EditorProject::from(pool);
        let plan = plan_component_capture(&project, 100, "Preview Button".into()).unwrap();
        let before = project.save_project().unwrap();
        let revision = project.revision();
        let (preview, nodes) = preview_definition(&project, &plan.definition).unwrap();
        assert_eq!(nodes.len(), plan.definition.nodes.len());
        let root = preview
            .object_by_id(ObjectId::new(nodes[&plan.definition.root_node]).unwrap())
            .unwrap();
        assert_eq!(preview.content_size(root), (140, 40));
        assert_eq!(project.save_project().unwrap(), before);
        assert_eq!(project.revision(), revision);
        assert!(!project.undo_available());
    }
}
