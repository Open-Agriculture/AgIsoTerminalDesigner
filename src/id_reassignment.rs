//! Preview and planning for project-wide object ID reassignment.

use crate::{components::*, ObjectIdRanges, ObjectInfo, ObjectTarget};
use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectId, ObjectPool, ObjectType};
use std::collections::{HashMap, HashSet};

/// One object affected by a reassignment preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReassignmentRow {
    pub name: String,
    pub object_type: ObjectType,
    pub old_id: u16,
    pub new_id: u16,
    pub overridden: bool,
    /// Present for IDs assigned to compiled component nodes.
    pub component_node: Option<(
        crate::components::ComponentInstanceId,
        crate::components::ComponentNodeId,
    )>,
}

/// A validated, deterministic ID mapping ready to apply as one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdReassignmentPlan {
    pub rows: Vec<ReassignmentRow>,
    /// Changed IDs only, in preview order.
    pub mappings: Vec<(u16, u16)>,
    /// Original object types used to interpret macro references to compiled
    /// component nodes during packet rewriting.
    pub target_types: Vec<(u16, ObjectType)>,
    pub includes_overrides: bool,
}

pub(crate) fn remap_component_source(
    source: &mut ComponentSource,
    mappings: &[(u16, u16)],
    pool: &ObjectPool,
    version: VtVersion,
) -> Result<(), String> {
    let map: HashMap<_, _> = mappings.iter().copied().collect();
    let mut target_types: HashMap<u16, ObjectType> = pool
        .objects()
        .iter()
        .map(|object| (object.id().value(), object.object_type()))
        .collect();
    for instance in &source.instances {
        if let Some(definition) = source.definitions.get(&instance.component_id) {
            for node in &definition.nodes {
                if let Some(id) = instance.compiled_ids.get(&node.id) {
                    target_types.insert(*id, node.object_type);
                }
            }
        }
    }
    let map_value = |value: &mut serde_json::Value| {
        if let Some(id) = value.as_u64().and_then(|id| u16::try_from(id).ok()) {
            if let Some(new_id) = map.get(&id) {
                *value = serde_json::Value::from(*new_id);
            }
        }
    };
    let map_target = |target: &mut ObjectTarget| {
        if let ObjectTarget::Id { id } = target {
            if let Some(new_id) = map.get(id) {
                *id = *new_id;
            }
        }
    };
    for definition in source.definitions.values_mut() {
        for node in &mut definition.nodes {
            if node.object_type == ObjectType::Macro {
                if let Some(value) = node.properties.get_mut("commands") {
                    let bytes: Vec<u8> =
                        serde_json::from_value(value.clone()).map_err(|error| {
                            format!("Invalid component Macro command stream: {error}")
                        })?;
                    let remapped = crate::macro_id_remap::remap_macro_object_ids_checked(
                        &bytes,
                        &map,
                        version,
                        &target_types,
                    )?;
                    *value = serde_json::to_value(remapped)
                        .map_err(|error| format!("Cannot encode component Macro: {error}"))?;
                }
            }
            for (property, reference) in &mut node.references {
                if node.object_type == ObjectType::ExternalObjectPointer
                    && property == "external_object_id"
                {
                    continue;
                }
                if let ComponentReference::ExternalObject(target) = reference {
                    map_target(target);
                }
            }
            for reference in &mut node.macro_references {
                map_target(&mut reference.target);
            }
            match &mut node.object_list {
                Some(ComponentObjectList::Required(targets)) => {
                    for target in targets {
                        map_target(target);
                    }
                }
                Some(ComponentObjectList::Nullable(targets)) => {
                    for target in targets.iter_mut().flatten() {
                        map_target(target);
                    }
                }
                None => {}
            }
        }
        for parameter in &mut definition.parameters {
            if parameter.kind == ComponentParameterKind::ObjectReference {
                map_value(&mut parameter.default_value);
            }
        }
    }
    for instance in &mut source.instances {
        let EditorParentRef::Primitive(id) = &mut instance.parent;
        if let Some(new_id) = map.get(id) {
            *id = *new_id;
        }
        for id in instance.compiled_ids.values_mut() {
            if let Some(new_id) = map.get(id) {
                *id = *new_id;
            }
        }
        if let Some(definition) = source.definitions.get(&instance.component_id) {
            for parameter in &definition.parameters {
                if parameter.kind == ComponentParameterKind::ObjectReference {
                    if let Some(value) = instance.overrides.get_mut(&parameter.id) {
                        map_value(value);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Compose sequential identity operations into one map from the original IDs
/// to their final IDs. A RemapObjectIds entry remains simultaneous within its
/// own operation.
pub(crate) fn compose_id_mappings(operations: &[crate::Operation]) -> Vec<(u16, u16)> {
    let mut composed = HashMap::<u16, u16>::new();
    for operation in operations {
        let step: Vec<(u16, u16)> = match operation {
            crate::Operation::ChangeObjectId { old_id, new_id }
            | crate::Operation::SetObjectIdAssignment {
                object_id: old_id,
                new_id,
                ..
            } => vec![(*old_id, *new_id)],
            crate::Operation::RemapObjectIds { mappings, .. } => mappings.clone(),
            _ => Vec::new(),
        };
        if step.is_empty() {
            continue;
        }
        let previous_targets: HashSet<_> = composed.values().copied().collect();
        let step_map: HashMap<_, _> = step.iter().copied().collect();
        for final_id in composed.values_mut() {
            if let Some(next) = step_map.get(final_id) {
                *final_id = *next;
            }
        }
        for (old_id, new_id) in step {
            if !previous_targets.contains(&old_id) && !composed.contains_key(&old_id) {
                composed.insert(old_id, new_id);
            }
        }
    }
    let mut mappings: Vec<_> = composed
        .into_iter()
        .filter(|(old, new)| old != new)
        .collect();
    mappings.sort_unstable_by_key(|(old, _)| *old);
    mappings
}

impl IdReassignmentPlan {
    /// Build a lowest-free per-type mapping. Manual IDs are held fixed unless
    /// `include_overrides` is true. All source and compiled IDs participate.
    pub fn build(
        pool: &ObjectPool,
        components: &ComponentSource,
        object_info: &HashMap<ObjectId, ObjectInfo>,
        ranges: &ObjectIdRanges,
        version: VtVersion,
        include_overrides: bool,
    ) -> Result<Self, String> {
        ranges.validate(version)?;

        #[derive(Clone)]
        struct Item {
            id: u16,
            object_type: ObjectType,
            name: String,
            overridden: bool,
            component_node: Option<(
                crate::components::ComponentInstanceId,
                crate::components::ComponentNodeId,
            )>,
        }

        let mut items = Vec::new();
        let mut occupied = HashSet::new();
        for object in pool.objects() {
            let id = object.id().value();
            if !occupied.insert(id) {
                return Err(format!("Object ID {id} is duplicated"));
            }
            let info = object_info.get(&object.id());
            items.push(Item {
                id,
                object_type: object.object_type(),
                name: info.map(|info| info.get_name(object)).unwrap_or_else(|| {
                    crate::smart_naming::get_object_type_name(object.object_type()).to_owned()
                }),
                overridden: info.is_some_and(|info| info.id_overridden),
                component_node: None,
            });
        }
        for instance in &components.instances {
            let definition = components
                .definitions
                .get(&instance.component_id)
                .ok_or_else(|| {
                    format!(
                        "Component definition for instance {} is missing",
                        instance.id
                    )
                })?;
            for node in &definition.nodes {
                let id = *instance.compiled_ids.get(&node.id).ok_or_else(|| {
                    format!("Compiled ID for component node {:?} is missing", node.id)
                })?;
                ObjectId::new(id).map_err(|_| format!("Compiled ID {id} is invalid"))?;
                if !occupied.insert(id) {
                    return Err(format!("Object ID {id} is used more than once"));
                }
                items.push(Item {
                    id,
                    object_type: node.object_type,
                    name: node.name.clone(),
                    overridden: false,
                    component_node: Some((instance.id, node.id)),
                });
            }
        }
        items.sort_by_key(|item| item.id);

        // Only IDs held fixed by preserved manual assignments consume space
        // before automatic rows are allocated.
        let mut reserved = HashSet::new();
        for item in &items {
            if item.overridden && !include_overrides {
                reserved.insert(item.id);
            }
        }

        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let new_id = if item.overridden && !include_overrides {
                item.id
            } else {
                let range = ranges.range_for(item.object_type);
                (range.start..=range.end)
                    .find(|candidate| !reserved.contains(candidate))
                    .ok_or_else(|| {
                        format!(
                            "No IDs remain in the {:?} range {}–{} for {}",
                            item.object_type, range.start, range.end, item.name
                        )
                    })?
            };
            reserved.insert(new_id);
            rows.push(ReassignmentRow {
                name: item.name,
                object_type: item.object_type,
                old_id: item.id,
                new_id,
                overridden: item.overridden,
                component_node: item.component_node,
            });
        }
        let mappings = rows
            .iter()
            .filter_map(|row| (row.old_id != row.new_id).then_some((row.old_id, row.new_id)))
            .collect();
        let target_types = rows
            .iter()
            .map(|row| (row.old_id, row.object_type))
            .collect();
        Ok(Self {
            rows,
            mappings,
            target_types,
            includes_overrides: include_overrides,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn object_with_id(
        object_type: ObjectType,
        id: u16,
    ) -> ag_iso_stack::object_pool::object::Object {
        let mut object = crate::default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        object
    }

    #[test]
    fn plan_preserves_overrides_unless_explicitly_included() {
        let manual = object_with_id(ObjectType::Button, 9000);
        let automatic = object_with_id(ObjectType::Button, 9001);
        let mut pool = ObjectPool::default();
        pool.add(manual.clone());
        pool.add(automatic);
        let mut info = ObjectInfo::new(&manual);
        info.name = Some("Manual button".into());
        info.id_overridden = true;
        let info = HashMap::from([(manual.id(), info)]);
        let mut ranges = ObjectIdRanges::defaults();
        ranges.set_range(
            ObjectType::Button,
            crate::ObjectIdRange {
                start: 6000,
                end: 6001,
            },
        );

        let preserved = IdReassignmentPlan::build(
            &pool,
            &ComponentSource::default(),
            &info,
            &ranges,
            VtVersion::Version6,
            false,
        )
        .unwrap();
        assert_eq!(preserved.rows[0].name, "Manual button");
        assert_eq!(
            (preserved.rows[0].old_id, preserved.rows[0].new_id),
            (9000, 9000)
        );
        assert_eq!(
            (preserved.rows[1].old_id, preserved.rows[1].new_id),
            (9001, 6000)
        );

        let included = IdReassignmentPlan::build(
            &pool,
            &ComponentSource::default(),
            &info,
            &ranges,
            VtVersion::Version6,
            true,
        )
        .unwrap();
        assert_eq!(included.mappings, vec![(9000, 6000), (9001, 6001)]);
    }

    #[test]
    fn component_identity_bindings_and_parameter_values_are_remapped() {
        let component = ComponentId::default();
        let node = ComponentNodeId::default();
        let parameter = ComponentParameterId::default();
        let instance_id = ComponentInstanceId::default();
        let definition = ComponentDefinition {
            schema_version: 1,
            id: component,
            name: "Example".into(),
            description: String::new(),
            category_path: vec![],
            tags: vec![],
            revision: 1,
            root_node: node,
            nodes: vec![ComponentNode {
                id: node,
                name: "Child".into(),
                object_type: ObjectType::Button,
                properties: BTreeMap::new(),
                children: vec![],
                references: BTreeMap::from([(
                    "pointer".into(),
                    ComponentReference::ExternalObject(ObjectTarget::id(10)),
                )]),
                macro_references: vec![ComponentMacroReference {
                    event: ag_iso_stack::object_pool::object_attributes::Event::OnActivate,
                    target: ObjectTarget::id(10),
                }],
                object_list: Some(ComponentObjectList::Required(vec![ObjectTarget::id(10)])),
            }],
            parameters: vec![ComponentParameter {
                id: parameter,
                name: "Pointer".into(),
                description: None,
                target_node: node,
                property: "value".into(),
                kind: ComponentParameterKind::ObjectReference,
                default_value: serde_json::json!(10),
            }],
        };
        let mut source = ComponentSource::default();
        source.definitions.insert(component, definition);
        source.instances.push(ComponentInstance {
            id: instance_id,
            component_id: component,
            parent: EditorParentRef::Primitive(10),
            child_index: None,
            x: 0,
            y: 0,
            overrides: BTreeMap::from([(parameter, serde_json::json!(10))]),
            compiled_ids: BTreeMap::from([(node, 20)]),
        });
        let pool = ObjectPool::default();
        remap_component_source(
            &mut source,
            &[(10, 11), (20, 21)],
            &pool,
            VtVersion::Version6,
        )
        .unwrap();
        let instance = &source.instances[0];
        assert_eq!(instance.parent, EditorParentRef::Primitive(11));
        assert_eq!(instance.compiled_ids[&node], 21);
        assert_eq!(instance.overrides[&parameter], serde_json::json!(11));
        let remapped_node = &source.definitions[&component].nodes[0];
        assert_eq!(
            remapped_node.references["pointer"],
            ComponentReference::ExternalObject(ObjectTarget::id(11))
        );
        assert_eq!(
            remapped_node.macro_references[0].target,
            ObjectTarget::id(11)
        );
        assert_eq!(
            remapped_node.object_list,
            Some(ComponentObjectList::Required(vec![ObjectTarget::id(11)]))
        );
        assert_eq!(
            source.definitions[&component].parameters[0].default_value,
            serde_json::json!(11)
        );
    }

    #[test]
    fn sequential_identity_operations_compose_without_cascading_swaps() {
        use crate::Operation;
        assert_eq!(
            compose_id_mappings(&[
                Operation::ChangeObjectId {
                    old_id: 10,
                    new_id: 20
                },
                Operation::ChangeObjectId {
                    old_id: 20,
                    new_id: 30
                },
            ]),
            vec![(10, 30)]
        );
        assert_eq!(
            compose_id_mappings(&[Operation::RemapObjectIds {
                mappings: vec![(10, 20), (20, 10)],
                target_types: vec![(10, ObjectType::Button), (20, ObjectType::Button)],
            }]),
            vec![(10, 20), (20, 10)]
        );
    }
}
