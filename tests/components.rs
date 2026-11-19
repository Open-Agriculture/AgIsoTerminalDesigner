use ag_iso_stack::object_pool::{
    object::Object, object_attributes::FontType, ObjectId, ObjectPool, ObjectRef, ObjectType,
};
use ag_iso_terminal_designer::operations::object_refs;
use ag_iso_terminal_designer::{components::*, *};
use serde_json::json;
use std::collections::BTreeMap;

fn property(name: &str) -> object_properties::PropertyKey {
    object_properties::PropertyKey::parse(name).unwrap()
}

fn fixture() -> EditorProject {
    fixture_with_sibling(false)
}

fn fixture_with_sibling(sibling: bool) -> EditorProject {
    fixture_with_options(sibling, false)
}

fn fixture_with_options(sibling: bool, sibling_first: bool) -> EditorProject {
    fixture_with_extra(sibling, sibling_first, false)
}

fn fixture_with_destination() -> EditorProject {
    fixture_with_extra(true, false, true)
}

fn fixture_with_extra(sibling: bool, sibling_first: bool, destination: bool) -> EditorProject {
    let mut pool = ObjectPool::default();
    for (id, ty) in [
        (2, ObjectType::DataMask),
        (100, ObjectType::Button),
        (101, ObjectType::OutputString),
        (102, ObjectType::FontAttributes),
    ] {
        let mut object = default_object(ty);
        object.mut_id().set_value(id).unwrap();
        pool.add(object);
    }
    if destination {
        for (id, width, height) in [(104, 220, 120), (105, 20, 20)] {
            let mut object = default_object(ObjectType::Container);
            object.mut_id().set_value(id).unwrap();
            if let Object::Container(container) = &mut object {
                container.width = width;
                container.height = height;
            }
            pool.add(object);
        }
    }
    if sibling {
        let mut object = default_object(ObjectType::OutputString);
        object.mut_id().set_value(103).unwrap();
        pool.add(object);
    }
    if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
        if sibling && sibling_first {
            mask.object_refs.push(ObjectRef {
                id: ObjectId::new(103).unwrap(),
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 40, y: 60 },
            });
        }
        mask.object_refs.push(ObjectRef {
            id: ObjectId::new(100).unwrap(),
            offset: ag_iso_stack::object_pool::object_attributes::Point { x: 10, y: 20 },
        });
        if sibling && !sibling_first {
            mask.object_refs.push(ObjectRef {
                id: ObjectId::new(103).unwrap(),
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 40, y: 60 },
            });
        }
        if destination {
            for id in [104, 105] {
                mask.object_refs.push(ObjectRef {
                    id: ObjectId::new(id).unwrap(),
                    offset: ag_iso_stack::object_pool::object_attributes::Point { x: 0, y: 0 },
                });
            }
        }
    }
    if let Some(Object::Button(button)) = pool.object_mut_by_id(ObjectId::new(100).unwrap()) {
        // Keep the captured label inside the Button's four-pixel content inset.
        button.width = 148;
        button.height = 60;
        button.background_colour = 12;
        button.object_refs.push(ObjectRef {
            id: ObjectId::new(101).unwrap(),
            offset: ag_iso_stack::object_pool::object_attributes::Point { x: 0, y: 8 },
        });
    }
    if let Some(Object::OutputString(label)) = pool.object_mut_by_id(ObjectId::new(101).unwrap()) {
        label.width = 140;
        label.height = 20;
        label.value = "Action".into();
        label.font_attributes = ObjectId::new(102).unwrap().into();
    }
    let mut project = EditorProject::from(pool);
    project.mask_size = 500;
    project
}
fn captured() -> (EditorProject, ComponentId, ComponentInstanceId) {
    let mut p = fixture();
    let plan = plan_component_capture(&p, 100, "Primary Button".into()).unwrap();
    let component = plan.definition.id;
    let instance = ComponentService::create_component_from_selection(&mut p, plan, false).unwrap();
    (p, component, instance)
}
fn parameter(p: &mut EditorProject, component: ComponentId) -> ComponentParameterId {
    let definition = &p.component_source().definitions[&component];
    let node = definition
        .nodes
        .iter()
        .find(|n| n.object_type == ObjectType::OutputString)
        .unwrap()
        .id;
    let id = ComponentParameterId::default();
    ComponentService::add_component_parameter(
        p,
        component,
        ComponentParameter {
            id,
            name: "Text".into(),
            description: Some("Caption".into()),
            target_node: node,
            property: ComponentParameterTarget::Property(property("value")),
            kind: ComponentParameterKind::Text,
            default_value: json!("Action"),
        },
    )
    .unwrap();
    id
}

#[test]
fn existing_component_iso_domain_values_remain_materializable_for_repair() {
    use ag_iso_stack::object_pool::ObjectType;
    use ag_iso_terminal_designer::components::{
        materialize_source, ComponentDefinition, ComponentInstance, ComponentNode, ComponentSource,
        EditorParentRef,
    };
    use std::collections::{BTreeMap, HashMap};

    let mut canonical = ObjectPool::default();
    let mut parent = default_object(ObjectType::DataMask);
    parent.mut_id().set_value(1).unwrap();
    canonical.add(parent);
    let mut external_variable = default_object(ObjectType::NumberVariable);
    external_variable.mut_id().set_value(20).unwrap();
    if let Object::NumberVariable(variable) = &mut external_variable {
        variable.value = 65_536;
    }
    canonical.add(external_variable);

    let component_id = ComponentId::default();
    let meter_node = ComponentNodeId::default();
    let context_node = ComponentNodeId::default();
    let input_attributes_node = ComponentNodeId::default();
    let variable_node = ComponentNodeId::default();
    let variable_parameter = ComponentParameterId::default();
    let definition = ComponentDefinition {
        schema_version: 1,
        id: component_id,
        name: "Repair draft".into(),
        description: String::new(),
        category_path: vec![],
        tags: vec![],
        revision: 1,
        root_node: meter_node,
        nodes: vec![
            ComponentNode {
                id: meter_node,
                name: "Meter".into(),
                object_type: ObjectType::OutputMeter,
                properties: BTreeMap::from([(property("start_angle"), json!(181))]),
                children: vec![],
                references: BTreeMap::from([(
                    property("variable_reference"),
                    ComponentReference::Internal(variable_node),
                )]),
                macro_references: vec![],
                object_list: None,
            },
            ComponentNode {
                id: context_node,
                name: "Graphics Context".into(),
                object_type: ObjectType::GraphicsContext,
                properties: BTreeMap::from([(property("canvas_width"), json!(32768))]),
                children: vec![],
                references: BTreeMap::new(),
                macro_references: vec![],
                object_list: None,
            },
            ComponentNode {
                id: input_attributes_node,
                name: "Input Attributes".into(),
                object_type: ObjectType::ExtendedInputAttributes,
                properties: BTreeMap::from([(property("code_planes"), json!([]))]),
                children: vec![],
                references: BTreeMap::new(),
                macro_references: vec![],
                object_list: None,
            },
            ComponentNode {
                id: variable_node,
                name: "Number Variable".into(),
                object_type: ObjectType::NumberVariable,
                properties: BTreeMap::from([(property("value"), json!(65_536))]),
                children: vec![],
                references: BTreeMap::new(),
                macro_references: vec![],
                object_list: None,
            },
        ],
        parameters: vec![ComponentParameter {
            id: variable_parameter,
            name: "Meter value".into(),
            description: None,
            target_node: meter_node,
            property: ComponentParameterTarget::Property(property("variable_reference")),
            kind: ComponentParameterKind::ObjectReference,
            default_value: json!(20),
        }],
    };
    let instance_id = ComponentInstanceId::default();
    let source = ComponentSource {
        definitions: BTreeMap::from([(component_id, definition)]),
        instances: vec![ComponentInstance {
            id: instance_id,
            component_id,
            name: "Repair draft instance".into(),
            parent: EditorParentRef::Primitive(1),
            child_index: None,
            x: 0,
            y: 0,
            overrides: BTreeMap::new(),
            compiled_ids: BTreeMap::from([
                (meter_node, 100),
                (context_node, 101),
                (input_attributes_node, 102),
                (variable_node, 103),
            ]),
        }],
    };

    let materialized = materialize_source(
        &canonical,
        &source,
        ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
        &HashMap::new(),
        &ObjectIdRanges::default(),
    )
    .expect("stored invalid values should remain available for repair");
    assert!(materialized
        .pool
        .object_by_id(ObjectId::new(100).unwrap())
        .is_some());
    assert!(materialized
        .pool
        .object_by_id(ObjectId::new(101).unwrap())
        .is_some());
    assert!(materialized
        .pool
        .object_by_id(ObjectId::new(102).unwrap())
        .is_some());
    assert!(materialized
        .pool
        .object_by_id(ObjectId::new(103).unwrap())
        .is_some());
    let Object::OutputMeter(meter) = materialized
        .pool
        .object_by_id(ObjectId::new(100).unwrap())
        .unwrap()
    else {
        panic!("compiled root should remain a meter");
    };
    assert_eq!(meter.start_angle, 181);
    // The persisted parameter overrides the stored internal reference.
    assert_eq!(meter.variable_reference.0.unwrap().value(), 20);
}
fn objects_by_id(pool: &ObjectPool) -> BTreeMap<u16, serde_json::Value> {
    pool.objects()
        .iter()
        .map(|o| (o.id().value(), serde_json::to_value(o).unwrap()))
        .collect()
}
#[test]
fn capture_preserves_domain_output_and_is_undoable() {
    let before = objects_by_id(fixture().get_pool());
    let (mut p, _, _) = captured();
    assert_eq!(p.get_pool().objects().len(), 1);
    assert_eq!(objects_by_id(&p.materialize().unwrap().pool), before);
    assert!(p.undo_operation());
    assert!(p.component_source().instances.is_empty());
    assert_eq!(objects_by_id(p.get_pool()), before);
    assert!(p.redo_operation());
    assert_eq!(objects_by_id(&p.materialize().unwrap().pool), before);
}

#[test]
fn hierarchy_reorders_component_roots_with_primitive_siblings() {
    use ag_iso_terminal_designer::canvas::edits::{position, reparent_at_index, PlacementTarget};

    let mut p = fixture_with_sibling(true);
    let plan = plan_component_capture(&p, 100, "Primary Button".into()).unwrap();
    let component = plan.definition.id;
    let instance = ComponentService::create_component_from_selection(&mut p, plan, false).unwrap();
    let target = PlacementTarget::Component {
        instance_id: instance,
    };
    let before = position(&p, &target).unwrap();

    reparent_at_index(&mut p, &target, 2, 0, 0, Some(2)).unwrap();

    let moved = &p.component_source().instances[0];
    assert_eq!(moved.child_index, Some(1));
    assert_eq!(
        (moved.x, moved.y),
        (i32::from(before.0), i32::from(before.1))
    );
    let root = p.component_source().definitions[&component].root_node;
    let compiled_root = moved.compiled_ids[&root];
    let materialized = p.materialize().unwrap();
    let refs = object_refs(
        materialized
            .pool
            .object_by_id(ObjectId::new(2).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(refs[0].id.value(), 103);
    assert_eq!(refs[1].id.value(), compiled_root);

    let before_noop_pool = objects_by_id(p.get_pool());
    let before_noop_source = p.component_source().clone();
    reparent_at_index(&mut p, &target, 2, 0, 0, Some(2)).unwrap();
    assert_eq!(objects_by_id(p.get_pool()), before_noop_pool);
    assert_eq!(p.component_source(), &before_noop_source);
    let before_invalid_pool = objects_by_id(p.get_pool());
    let before_invalid_source = p.component_source().clone();
    assert!(reparent_at_index(&mut p, &target, 101, 0, 0, Some(0)).is_err());
    assert_eq!(objects_by_id(p.get_pool()), before_invalid_pool);
    assert_eq!(p.component_source(), &before_invalid_source);

    let primitive = PlacementTarget::Primitive {
        parent_id: 2,
        child_index: 0,
    };
    reparent_at_index(&mut p, &primitive, 2, 0, 0, Some(2)).unwrap();
    let materialized = p.materialize().unwrap();
    let refs = object_refs(
        materialized
            .pool
            .object_by_id(ObjectId::new(2).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(refs[0].id.value(), compiled_root);
    assert_eq!(refs[1].id.value(), 103);

    assert!(p.undo_operation());
    let materialized = p.materialize().unwrap();
    let refs = object_refs(
        materialized
            .pool
            .object_by_id(ObjectId::new(2).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(refs[0].id.value(), 103);
    assert_eq!(refs[1].id.value(), compiled_root);
    assert!(p.undo_operation());
    assert_eq!(p.component_source().instances[0].child_index, Some(0));
}

#[test]
fn grouping_mixed_component_and_primitive_siblings_preserves_identity_and_order() {
    use ag_iso_terminal_designer::canvas::edits::{group_placements, PlacementTarget};

    let mut p = fixture_with_destination();
    let plan = plan_component_capture(&p, 100, "Primary Button".into()).unwrap();
    let component = plan.definition.id;
    let instance_id =
        ComponentService::create_component_from_selection(&mut p, plan, false).unwrap();
    let instance_target = PlacementTarget::Component { instance_id };
    let primitive_target = PlacementTarget::Primitive {
        parent_id: 2,
        child_index: 0,
    };

    let grouped = group_placements(
        &mut p,
        &[instance_target, primitive_target],
        ObjectType::Container,
        "Mixed group",
    )
    .unwrap();
    let PlacementTarget::Primitive {
        parent_id: 2,
        child_index,
    } = grouped
    else {
        panic!("expected a primitive group occurrence");
    };
    let group_id = object_refs(
        p.get_pool()
            .object_by_id(ObjectId::new(2).unwrap())
            .unwrap(),
    )
    .unwrap()[child_index]
        .id
        .value();
    let instance = p
        .component_source()
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .unwrap();
    assert_eq!(instance.component_id, component);
    assert_eq!(instance.parent, EditorParentRef::Primitive(group_id));
    assert_eq!((instance.x, instance.y), (0, 0));
    let definition = &p.component_source().definitions[&component];
    let compiled_root = instance.compiled_ids[&definition.root_node];
    let group = p.materialize().unwrap();
    let grouped_refs = object_refs(
        group
            .pool
            .object_by_id(ObjectId::new(group_id).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        grouped_refs
            .iter()
            .map(|r| (r.id.value(), r.offset.x, r.offset.y))
            .collect::<Vec<_>>(),
        vec![(compiled_root, 0, 0), (103, 30, 40)]
    );
    let mask_refs = object_refs(
        p.get_pool()
            .object_by_id(ObjectId::new(2).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        mask_refs.iter().map(|r| r.id.value()).collect::<Vec<_>>(),
        vec![group_id, 104, 105]
    );
}

#[test]
fn batch_reparent_preserves_mixed_positions_and_rejects_overflow_atomically() {
    use ag_iso_terminal_designer::canvas::edits::{move_placements, position, PlacementTarget};

    let mut p = fixture_with_destination();
    let plan = plan_component_capture(&p, 100, "Primary Button".into()).unwrap();
    let instance_id =
        ComponentService::create_component_from_selection(&mut p, plan, false).unwrap();
    let component_target = PlacementTarget::Component { instance_id };
    let primitive_target = PlacementTarget::Primitive {
        parent_id: 2,
        child_index: 0,
    };
    let before_component = position(&p, &component_target).unwrap();
    let before_primitive = position(&p, &primitive_target).unwrap();
    let moved = move_placements(
        &mut p,
        &[component_target.clone(), primitive_target],
        104,
        None,
        (0, 0),
    )
    .unwrap();
    assert_eq!(position(&p, &component_target), Some(before_component));
    let primitive_after = moved
        .iter()
        .find(|target| matches!(target, PlacementTarget::Primitive { .. }))
        .unwrap();
    assert_eq!(position(&p, primitive_after), Some(before_primitive));
    let instance = p
        .component_source()
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .unwrap();
    assert_eq!(instance.parent, EditorParentRef::Primitive(104));
    assert_eq!(instance.child_index, Some(0));
    let destination_refs = object_refs(
        p.get_pool()
            .object_by_id(ObjectId::new(104).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        destination_refs
            .iter()
            .map(|r| r.id.value())
            .collect::<Vec<_>>(),
        vec![103]
    );

    let before_pool = objects_by_id(p.get_pool());
    let before_source = p.component_source().clone();
    assert!(move_placements(&mut p, &moved, 105, None, (0, 0)).is_err());
    assert_eq!(objects_by_id(p.get_pool()), before_pool);
    assert_eq!(p.component_source(), &before_source);
    assert!(p.undo_operation());
    assert_eq!(
        p.component_source().instances[0].parent,
        EditorParentRef::Primitive(2)
    );
}

#[test]
fn hierarchy_destination_path_shifts_after_removing_an_earlier_ancestor_sibling() {
    use ag_iso_terminal_designer::canvas::edits::{hierarchy_path_after_removal, PlacementTarget};

    let p = fixture_with_options(true, true);
    let source = PlacementTarget::Primitive {
        parent_id: 2,
        child_index: 0,
    };
    let destination_parent = vec![0, 1, 0];
    assert_eq!(
        hierarchy_path_after_removal(&p, 2, &destination_parent, &source).unwrap(),
        vec![0, 0, 0]
    );
}

#[test]
fn master_update_preserves_overrides_and_ids_without_generated_source_mutation() {
    let (mut p, component, a) = captured();
    let text = parameter(&mut p, component);
    ComponentService::set_component_override(&mut p, a, text, json!("Connect")).unwrap();
    let b = ComponentService::instantiate_component(
        &mut p,
        component,
        2,
        170,
        20,
        BTreeMap::from([(text, json!("Save"))]),
    )
    .unwrap();
    let instances = p.component_source().instances.clone();
    let canonical = p.get_pool().clone();
    let mut master = p.component_source().definitions[&component].clone();
    for node in &mut master.nodes {
        if node.object_type == ObjectType::Button {
            node.properties.insert(property("height"), json!(48));
            node.properties
                .insert(property("background_colour"), json!(14));
        }
        if node.object_type == ObjectType::OutputString {
            node.properties.insert(
                property("justification"),
                json!({"horizontal":"Middle","vertical":"Middle"}),
            );
        }
    }
    ComponentService::edit_component_definition(&mut p, master).unwrap();
    assert_eq!(p.component_source().instances, instances);
    assert_eq!(p.get_pool(), &canonical);
    let materialized = p.materialize().unwrap();
    for (id, caption) in [(a, "Connect"), (b, "Save")] {
        let instance = p
            .component_source()
            .instances
            .iter()
            .find(|i| i.id == id)
            .unwrap();
        for node in &p.component_source().definitions[&component].nodes {
            let object = materialized
                .pool
                .object_by_id(ObjectId::new(instance.compiled_ids[&node.id]).unwrap())
                .unwrap();
            match object {
                Object::Button(button) => {
                    assert_eq!(button.height, 48);
                    assert_eq!(button.background_colour, 14);
                }
                Object::OutputString(label) => {
                    assert_eq!(label.value.decode(FontType::Latin1).unwrap(), caption);
                    assert_eq!(
                        serde_json::to_value(label.justification).unwrap(),
                        json!({"horizontal":"Middle","vertical":"Middle"})
                    );
                }
                _ => {}
            }
        }
    }
    assert_eq!(
        objects_by_id(&materialized.pool),
        objects_by_id(&p.materialize().unwrap().pool)
    );
}

#[test]
fn save_load_detach_and_history_preserve_source_and_output() {
    let (mut p, component, a) = captured();
    let text = parameter(&mut p, component);
    ComponentService::set_component_override(&mut p, a, text, json!("Connect")).unwrap();
    ComponentService::instantiate_component(
        &mut p,
        component,
        2,
        170,
        20,
        BTreeMap::from([(text, json!("Save"))]),
    )
    .unwrap();
    let saved = p.save_project().unwrap();
    let mut loaded = EditorProject::load_project(saved).unwrap();
    assert_eq!(p.component_source(), loaded.component_source());
    let before = objects_by_id(&loaded.materialize().unwrap().pool);
    assert_eq!(before, objects_by_id(&p.materialize().unwrap().pool));
    ComponentService::detach_component_instance(&mut loaded, a).unwrap();
    assert_eq!(before, objects_by_id(&loaded.materialize().unwrap().pool));
    assert!(loaded.undo_operation());
    assert_eq!(loaded.component_source(), p.component_source());
    assert!(loaded.redo_operation());
    assert_eq!(before, objects_by_id(&loaded.materialize().unwrap().pool));
}

#[test]
fn invalid_override_collision_reverse_reference_and_delete_are_atomic() {
    let (mut p, component, a) = captured();
    let text = parameter(&mut p, component);
    let before = p.save_project().unwrap();
    let revision = p.revision();
    assert!(ComponentService::set_component_override(&mut p, a, text, json!(44)).is_err());
    assert!(ComponentService::delete_definition(&mut p, component).is_err());
    assert_eq!(before, p.save_project().unwrap());
    assert_eq!(revision, p.revision());
    let mut tx = OperationTransaction::new(None);
    tx.add_operation(Operation::CreateObject {
        object_id: Some(100),
        object_type: ObjectType::Button,
        name: "collision".into(),
    });
    assert!(p.execute_transaction(tx).is_err());
    let mut tx = OperationTransaction::new(None);
    tx.add_operation(Operation::AddChild {
        parent: ObjectTarget::id(2),
        child: ObjectTarget::id(100),
        x: 0,
        y: 0,
    });
    assert!(p.execute_transaction(tx).is_err());
    assert_eq!(before, p.save_project().unwrap());
}

#[test]
fn invalid_source_cycles_and_compiled_collisions_are_rejected() {
    let (p, component, _) = captured();
    let mut source = p.component_source().clone();
    let nodes = &mut source.definitions.get_mut(&component).unwrap().nodes;
    let root = nodes
        .iter()
        .find(|n| n.object_type == ObjectType::Button)
        .unwrap()
        .id;
    nodes
        .iter_mut()
        .find(|n| n.id == root)
        .unwrap()
        .children
        .push(ComponentChild {
            child: root,
            x: 0,
            y: 0,
        });
    assert_eq!(
        materialize_source(
            p.get_pool(),
            &source,
            p.vt_version(),
            &Default::default(),
            p.object_id_ranges(),
        )
        .unwrap_err()
        .code,
        "COMPONENT_INTERNAL_REFERENCE_INVALID"
    );
    let mut source = p.component_source().clone();
    *source.instances[0]
        .compiled_ids
        .values_mut()
        .next()
        .unwrap() = 2;
    assert_eq!(
        materialize_source(
            p.get_pool(),
            &source,
            p.vt_version(),
            &Default::default(),
            p.object_id_ranges(),
        )
        .unwrap_err()
        .code,
        "COMPONENT_COMPILED_ID_COLLISION"
    );
}

#[test]
fn reassignment_includes_compiled_nodes_and_round_trips_as_one_change() {
    let (mut project, _, instance) = captured();
    let before = project.save_project().unwrap();
    let plan = project.plan_id_reassignment(false).unwrap();
    assert!(plan.rows.iter().any(|row| row.component_node.is_some()));
    let mappings: BTreeMap<_, _> = plan.mappings.iter().copied().collect();
    project.apply_id_reassignment(plan).unwrap();
    let source = project.component_source();
    let compiled = source
        .instances
        .iter()
        .find(|item| item.id == instance)
        .unwrap();
    assert_eq!(compiled.parent, EditorParentRef::Primitive(mappings[&2]));
    let materialized = project.materialize().unwrap();
    for old in [2, 100, 101, 102] {
        assert!(materialized
            .pool
            .object_by_id(ObjectId::new(mappings[&old]).unwrap())
            .is_some());
    }
    let after = project.save_project().unwrap();
    assert!(project.undo_operation());
    assert_eq!(project.save_project().unwrap(), before);
    assert!(project.redo_operation());
    assert_eq!(project.save_project().unwrap(), after);
}
