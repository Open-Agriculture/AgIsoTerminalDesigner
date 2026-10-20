use super::*;
use crate::object_properties::{get_object_property, get_property_descriptors};
use crate::operations::{macro_refs, object_list, object_refs};
use crate::{EditorProject, ObjectTarget, Operation, PropertySemantic};
use ag_iso_stack::object_pool::{ObjectId, ObjectType};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct CapturePlan {
    pub definition: ComponentDefinition,
    pub instance: ComponentInstance,
    pub remove_ids: BTreeSet<u16>,
    pub external_references: Vec<(ComponentNodeId, String, u16)>,
    pub base_revision: u64,
    pub expected_objects: Vec<(ComponentNodeId, ag_iso_stack::object_pool::object::Object)>,
}
/// Prepare a reviewable capture; no source changes and no automatic parameters.
pub fn plan_component_capture(
    project: &EditorProject,
    root: u16,
    name: String,
) -> ComponentResult<CapturePlan> {
    let pool = project.get_pool();
    let fail = |message: &str| ComponentError::new("COMPONENT_CAPTURE_INVALID", message);
    let root_id = ObjectId::new(root).map_err(|_| fail("Invalid root ID"))?;
    let mut placements = Vec::new();
    for parent in pool.objects() {
        for child in object_refs(parent).unwrap_or_default() {
            if child.id == root_id {
                placements.push((parent.id().value(), child.offset.x, child.offset.y));
            }
        }
    }
    if placements.len() != 1 {
        return Err(fail(
            "Capture requires exactly one ordinary positioned parent",
        ));
    }
    let mut included = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !included.insert(id) {
            continue;
        }
        let object = pool
            .object_by_id(ObjectId::new(id).unwrap())
            .ok_or_else(|| fail("Missing capture object"))?;
        pending.extend(
            object_refs(object)
                .unwrap_or_default()
                .iter()
                .map(|r| r.id.value()),
        );
    }
    let subtree = included.clone();
    pending.extend(included.iter().copied());
    while let Some(id) = pending.pop() {
        let object = pool.object_by_id(ObjectId::new(id).unwrap()).unwrap();
        for reference in object.referenced_objects() {
            let dependency = pool
                .object_by_id(reference)
                .ok_or_else(|| fail("Missing dependency"))?;
            if matches!(
                dependency.object_type(),
                ObjectType::FontAttributes
                    | ObjectType::FillAttributes
                    | ObjectType::LineAttributes
                    | ObjectType::PictureGraphic
            ) && included.insert(reference.value())
            {
                pending.push(reference.value());
            }
        }
    }
    let mut remove = included.clone();
    // Shared visual dependencies are copied; shared positioned children are ambiguous.
    loop {
        let mut changed = false;
        for object in pool.objects() {
            if remove.contains(&object.id().value()) {
                continue;
            }
            for reference in object.referenced_objects() {
                let id = reference.value();
                if !remove.contains(&id) {
                    continue;
                }
                if id == root && object.id().value() == placements[0].0 {
                    continue;
                }
                if subtree.contains(&id) {
                    return Err(fail(
                        "An outside object references the selected subtree; detach that reference first",
                    ));
                }
                remove.remove(&id);
                changed = true;
                break;
            }
            if changed {
                break;
            }
        }
        if !changed {
            break;
        }
    }
    let node_ids: BTreeMap<_, _> = included
        .iter()
        .map(|id| (*id, ComponentNodeId::default()))
        .collect();
    let mut nodes = Vec::new();
    let mut external_references = Vec::new();
    for id in &included {
        let object = pool.object_by_id(ObjectId::new(*id).unwrap()).unwrap();
        // These structured reference surfaces need explicit future capture support.
        if object.object_type() == ObjectType::ObjectLabelReferenceList {
            return Err(fail(
                "Object label tables cannot be captured as positioned components",
            ));
        }
        let object_list = object_list(object).map(|list| {
            let mut target = |target_id: ObjectId| {
                external_references.push((node_ids[id], "object_list".into(), target_id.value()));
                ObjectTarget::id(target_id.value())
            };
            match list {
                crate::operations::ObjectReferenceList::Required(ids) => {
                    ComponentObjectList::Required(ids.into_iter().map(&mut target).collect())
                }
                crate::operations::ObjectReferenceList::Nullable(ids) => {
                    ComponentObjectList::Nullable(
                        ids.into_iter().map(|id| id.0.map(&mut target)).collect(),
                    )
                }
            }
        });
        let mut properties = BTreeMap::new();
        let mut references = BTreeMap::new();
        for descriptor in get_property_descriptors(object.object_type())
            .into_iter()
            .filter(|d| d.writable)
        {
            let value =
                get_object_property(object, descriptor.name).map_err(|e| fail(&e.to_string()))?;
            if matches!(
                descriptor.semantic,
                PropertySemantic::ObjectReference { .. }
            ) {
                let reference = if let Some(target) = value
                    .as_u64()
                    .and_then(|v| u16::try_from(v).ok())
                    .filter(|id| *id != 65535)
                {
                    if let Some(node) = node_ids.get(&target) {
                        ComponentReference::Internal(*node)
                    } else {
                        external_references.push((
                            node_ids[id],
                            descriptor.name.to_string(),
                            target,
                        ));
                        ComponentReference::ExternalObject(ObjectTarget::id(target))
                    }
                } else {
                    ComponentReference::Null
                };
                references.insert(descriptor.name, reference);
            } else {
                properties.insert(descriptor.name, value);
            }
        }
        let macro_references = macro_refs(object)
            .unwrap_or_default()
            .iter()
            .map(|reference| {
                external_references.push((
                    node_ids[id],
                    format!("macro {:?}", reference.event_id),
                    reference.macro_id.into(),
                ));
                ComponentMacroReference {
                    event: reference.event_id,
                    target: ObjectTarget::id(reference.macro_id.into()),
                }
            })
            .collect();
        nodes.push(ComponentNode {
            macro_references,
            object_list,
            id: node_ids[id],
            name: project.get_object_info(object).get_name(object),
            object_type: object.object_type(),
            properties,
            references,
            children: object_refs(object)
                .unwrap_or_default()
                .iter()
                .map(|c| ComponentChild {
                    child: node_ids[&c.id.value()],
                    x: c.offset.x.into(),
                    y: c.offset.y.into(),
                })
                .collect(),
        });
    }
    let component_id = ComponentId::default();
    let effective = project.materialize()?;
    let child_index = crate::operations::object_refs(
        effective
            .pool
            .object_by_id(ObjectId::new(placements[0].0).unwrap())
            .unwrap(),
    )
    .unwrap()
    .iter()
    .position(|r| r.id == root_id);
    Ok(CapturePlan {
        definition: ComponentDefinition {
            schema_version: 1,
            id: component_id,
            name,
            description: String::new(),
            category_path: vec![],
            tags: vec![],
            revision: 1,
            root_node: node_ids[&root],
            nodes,
            parameters: vec![],
        },
        instance: ComponentInstance {
            id: ComponentInstanceId::default(),
            component_id,
            name: String::new(),
            child_index,
            parent: EditorParentRef::Primitive(placements[0].0),
            x: placements[0].1.into(),
            y: placements[0].2.into(),
            overrides: Default::default(),
            compiled_ids: remove.iter().map(|id| (node_ids[id], *id)).collect(),
        },
        expected_objects: included
            .iter()
            .map(|id| {
                (
                    node_ids[id],
                    pool.object_by_id(ObjectId::new(*id).unwrap())
                        .unwrap()
                        .clone(),
                )
            })
            .collect(),
        remove_ids: remove,
        external_references,
        base_revision: project.revision(),
    })
}
impl ComponentService {
    /// Caller must explicitly accept the listed functional references.
    pub fn create_component_from_selection(
        project: &mut EditorProject,
        plan: CapturePlan,
        keep_external_references: bool,
    ) -> ComponentResult<ComponentInstanceId> {
        if project.revision() != plan.base_revision {
            return Err(ComponentError::new(
                "COMPONENT_CAPTURE_STALE",
                "Project changed; prepare capture again",
            ));
        }
        if !plan.external_references.is_empty() && !keep_external_references {
            return Err(ComponentError::new(
                "COMPONENT_EXTERNAL_REFERENCE_INVALID",
                "Review and explicitly keep external references, or cancel",
            ));
        }
        let id = plan.instance.id;
        let EditorParentRef::Primitive(parent) = plan.instance.parent;
        let root = plan.instance.compiled_ids[&plan.definition.root_node];
        let version = project.vt_version;
        let info = project.component_object_info();
        let ranges = project.object_id_ranges().clone();
        project.edit_components("Create component from selection", |pool, source| {
            validate_definition_names(source, &plan.definition)?;
            Operation::RemoveChild { parent_id: parent, child_id: root }.apply(pool, &mut Default::default(), &mut Default::default()).map_err(|e| ComponentError::new("COMPONENT_CAPTURE_INVALID", e.to_string()))?;
            for id in plan.remove_ids { pool.remove(ObjectId::new(id).unwrap()); }
            source.definitions.insert(plan.definition.id, plan.definition);
            let mut instance = plan.instance;
            instance.name = unique_instance_name(source, instance.component_id);
            source.instances.push(instance);
            prepare_compiled_ids(pool, source, &ranges)?;
            let materialized = materialize_source(pool, source, version, &info, &ranges)?;
            let instance = source.instances.iter().find(|i| i.id == id).unwrap();
            let mut captured = ag_iso_stack::object_pool::ObjectPool::default();
            for generated in instance.compiled_ids.values() { captured.add(materialized.pool.object_by_id(ObjectId::new(*generated).unwrap()).unwrap().clone()); }
            for (node, original) in &plan.expected_objects {
                let generated = instance.compiled_ids[node];
                if generated != original.id().value() {
                    Operation::ChangeObjectId { old_id: generated, new_id: original.id().value() }.apply(&mut captured, &mut Default::default(), &mut Default::default()).map_err(|e| ComponentError::new("COMPONENT_CAPTURE_INVALID", e.to_string()))?;
                }
            }
            for (_, original) in &plan.expected_objects {
                if captured.object_by_id(original.id()) != Some(original) { return Err(ComponentError::new("COMPONENT_CAPTURE_INVALID", format!("Object {} contains state not representable by this component template; capture would change its output", original.id().value()))); }
            }
            Ok(())
        })?;
        Ok(id)
    }
}
