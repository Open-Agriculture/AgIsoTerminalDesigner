use ag_iso_stack::object_pool::{
    object::Object, object_attributes::FontType, ObjectId, ObjectPool, ObjectRef, ObjectType,
};
use ag_iso_terminal_designer::{components::*, *};
use serde_json::json;
use std::collections::BTreeMap;

fn property(name: &str) -> object_properties::PropertyKey {
    object_properties::PropertyKey::parse(name).unwrap()
}

fn fixture() -> EditorProject {
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
    if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
        mask.object_refs.push(ObjectRef {
            id: ObjectId::new(100).unwrap(),
            offset: ag_iso_stack::object_pool::object_attributes::Point { x: 10, y: 20 },
        });
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
fn unknown_saved_component_property_names_are_rejected() {
    let (mut project, component, _) = captured();
    parameter(&mut project, component);
    let definition = &project.component_source().definitions[&component];
    let mut value = serde_json::to_value(definition).unwrap();
    value["nodes"][0]["properties"]
        .as_object_mut()
        .unwrap()
        .insert("not_a_property".into(), json!(1));
    let error = serde_json::from_value::<ComponentDefinition>(value)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not_a_property"), "{error}");

    let mut value = serde_json::to_value(definition).unwrap();
    value["parameters"][0]["property"] = json!("not_a_property");
    let error = serde_json::from_value::<ComponentDefinition>(value)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not_a_property"), "{error}");
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
fn reset_default_add_remove_nodes_and_duplicate() {
    let (mut p, component, a) = captured();
    let text = parameter(&mut p, component);
    ComponentService::set_component_override(&mut p, a, text, json!("Connect")).unwrap();
    let existing = p.component_source().instances[0].compiled_ids.clone();
    let mut master = p.component_source().definitions[&component].clone();
    master.parameters[0].default_value = json!("Button");
    let icon = ComponentNodeId::default();
    master.nodes.push(ComponentNode {
        macro_references: vec![],
        object_list: None,
        id: icon,
        name: "Icon".into(),
        object_type: ObjectType::OutputRectangle,
        properties: Default::default(),
        references: Default::default(),
        children: vec![],
    });
    ComponentService::edit_component_definition(&mut p, master).unwrap();
    let ids = &p.component_source().instances[0].compiled_ids;
    for (node, id) in &existing {
        assert_eq!(ids[node], *id);
    }
    assert!(ids.contains_key(&icon));
    ComponentService::reset_component_override(&mut p, a, text).unwrap();
    let pool = p.materialize().unwrap().pool;
    assert!(pool
        .objects()
        .iter()
        .any(|o| matches!(o, Object::OutputString(s) if s.value.decode(FontType::Latin1).unwrap() == "Button")));
    let mut master = p.component_source().definitions[&component].clone();
    master.nodes.retain(|n| n.id != icon);
    ComponentService::edit_component_definition(&mut p, master).unwrap();
    assert_eq!(p.component_source().instances[0].compiled_ids, existing);
    let b = ComponentService::duplicate_component_instance(&mut p, a, 200, 20).unwrap();
    let materialized = p.materialize().unwrap();
    let n = materialized.pool.objects().len();
    ComponentService::delete_component_instance(&mut p, b).unwrap();
    assert_eq!(p.materialize().unwrap().pool.objects().len(), n - 3);
}
#[test]
fn shared_visual_dependencies_are_copied_and_internal_sharing_is_preserved() {
    let mut pool = fixture().get_pool().clone();
    let mut outside = default_object(ObjectType::OutputString);
    outside.mut_id().set_value(200).unwrap();
    if let Object::OutputString(label) = &mut outside {
        label.font_attributes = ObjectId::new(102).unwrap().into();
    }
    pool.add(outside);
    let mut p = EditorProject::from(pool);
    p.mask_size = 500;
    let plan = plan_component_capture(&p, 100, "Shared style capture".into()).unwrap();
    let component = plan.definition.id;
    let font = plan
        .definition
        .nodes
        .iter()
        .find(|n| n.object_type == ObjectType::FontAttributes)
        .unwrap()
        .id;
    assert!(!plan.remove_ids.contains(&102));
    let a = ComponentService::create_component_from_selection(&mut p, plan, false).unwrap();
    assert!(p
        .get_pool()
        .object_by_id(ObjectId::new(102).unwrap())
        .is_some());
    let a = p
        .component_source()
        .instances
        .iter()
        .find(|i| i.id == a)
        .unwrap();
    assert_ne!(a.compiled_ids[&font], 102);
    let mut master = p.component_source().definitions[&component].clone();
    let mut second_label = master
        .nodes
        .iter()
        .find(|n| n.object_type == ObjectType::OutputString)
        .unwrap()
        .clone();
    second_label.id = ComponentNodeId::default();
    master
        .nodes
        .iter_mut()
        .find(|n| n.id == master.root_node)
        .unwrap()
        .children
        .push(ComponentChild {
            child: second_label.id,
            x: 0,
            y: 22,
        });
    master.nodes.push(second_label);
    ComponentService::edit_component_definition(&mut p, master).unwrap();
    let materialized = p.materialize().unwrap();
    let instance = &p.component_source().instances[0];
    for node in &p.component_source().definitions[&component].nodes {
        if node.object_type == ObjectType::OutputString {
            let Object::OutputString(label) = materialized
                .pool
                .object_by_id(ObjectId::new(instance.compiled_ids[&node.id]).unwrap())
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(
                label.font_attributes,
                ObjectId::new(instance.compiled_ids[&font]).unwrap().into()
            );
        }
    }
}

#[test]
fn external_functional_capture_requires_explicit_choice() {
    let mut pool = fixture().get_pool().clone();
    let mut variable = default_object(ObjectType::StringVariable);
    variable.mut_id().set_value(200).unwrap();
    pool.add(variable);
    if let Some(Object::OutputString(label)) = pool.object_mut_by_id(ObjectId::new(101).unwrap()) {
        label.variable_reference = ObjectId::new(200).unwrap().into();
    }
    let mut p = EditorProject::from(pool);
    let plan = plan_component_capture(&p, 100, "Bound display".into()).unwrap();
    assert_eq!(plan.external_references.len(), 1);
    assert!(plan.definition.parameters.is_empty());
    let before = p.save_project().unwrap();
    assert!(
        ComponentService::create_component_from_selection(&mut p, plan.clone(), false).is_err()
    );
    assert_eq!(before, p.save_project().unwrap());
    ComponentService::create_component_from_selection(&mut p, plan, true).unwrap();
    assert!(p
        .get_pool()
        .object_by_id(ObjectId::new(200).unwrap())
        .is_some());
    let pool = p.materialize().unwrap().pool;
    assert!(
        matches!(pool.object_by_id(ObjectId::new(101).unwrap()), Some(Object::OutputString(label)) if label.variable_reference == ObjectId::new(200).unwrap().into())
    );
}

#[test]
fn explicit_object_reference_parameters_resolve_to_canonical_variables() {
    let (mut p, component, a) = captured();
    let mut tx = OperationTransaction::new(None);
    for id in [200, 300] {
        tx.add_operation(Operation::CreateObject {
            object_id: Some(id),
            object_type: ObjectType::NumberVariable,
            name: format!("Value {id}"),
        });
    }
    p.execute_transaction(tx).unwrap();
    let mut master = p.component_source().definitions[&component].clone();
    let node = master
        .nodes
        .iter_mut()
        .find(|n| n.object_type == ObjectType::OutputString)
        .unwrap();
    node.object_type = ObjectType::OutputNumber;
    node.properties.clear();
    node.properties.insert(property("width"), json!(140));
    node.properties.insert(property("height"), json!(20));
    node.references.remove(&property("variable_reference"));
    let parameter = ComponentParameterId::default();
    master.parameters.push(ComponentParameter {
        id: parameter,
        name: "Value Source".into(),
        description: None,
        target_node: node.id,
        property: ComponentParameterTarget::Property(property("variable_reference")),
        kind: ComponentParameterKind::ObjectReference,
        default_value: json!(200),
    });
    ComponentService::edit_component_definition(&mut p, master).unwrap();
    let b = ComponentService::instantiate_component(
        &mut p,
        component,
        2,
        200,
        20,
        BTreeMap::from([(parameter, json!(300))]),
    )
    .unwrap();
    let materialized = p.materialize().unwrap();
    for (instance, expected) in [(a, 200), (b, 300)] {
        let instance = p
            .component_source()
            .instances
            .iter()
            .find(|i| i.id == instance)
            .unwrap();
        let node = p.component_source().definitions[&component]
            .nodes
            .iter()
            .find(|n| n.object_type == ObjectType::OutputNumber)
            .unwrap();
        assert!(
            matches!(materialized.pool.object_by_id(ObjectId::new(instance.compiled_ids[&node.id]).unwrap()), Some(Object::OutputNumber(label)) if label.variable_reference == ObjectId::new(expected).unwrap().into())
        );
    }
    assert!(ComponentService::set_component_override(&mut p, a, parameter, json!(100)).is_err());
}

#[test]
fn materialized_property_errors_map_back_to_component_instance_node_and_property() {
    let (p, component, instance) = captured();
    let mut source = p.component_source().clone();
    let master = source.definitions.get_mut(&component).unwrap();
    let root = master.root_node;
    master
        .nodes
        .iter_mut()
        .find(|n| n.id == root)
        .unwrap()
        .properties
        .insert(property("height"), json!(-1));
    let source: ComponentSource = serde_json::from_value(serde_json::to_value(source).unwrap())
        .expect("invalid values on recognized properties remain loadable for repair");
    let error = materialize_source(
        p.get_pool(),
        &source,
        p.vt_version(),
        &Default::default(),
        p.object_id_ranges(),
    )
    .unwrap_err();
    assert_eq!(error.component_id, Some(component));
    assert_eq!(error.instance_id, Some(instance));
    assert_eq!(error.node_id, Some(root));
    assert_eq!(error.property.as_deref(), Some("height"));
    assert_eq!(error.object_id, Some(100));
}

#[test]
fn primitive_edits_skip_reserved_ids_and_interleave_with_source_history() {
    let (mut p, component, a) = captured();
    ComponentService::instantiate_component(&mut p, component, 2, 170, 20, Default::default())
        .unwrap();
    let reserved: Vec<_> = p
        .component_source()
        .instances
        .iter()
        .flat_map(|i| i.compiled_ids.values().copied())
        .collect();
    let before = p.component_source().clone();
    let mut tx = OperationTransaction::new(None);
    tx.add_operation(Operation::CreateObject {
        object_id: None,
        object_type: ObjectType::NumberVariable,
        name: "Ordinary value".into(),
    });
    let applied = p.execute_transaction(tx).unwrap();
    assert!(!reserved.contains(&applied.affected_objects[0]));
    ComponentService::delete_component_instance(&mut p, a).unwrap();
    assert!(p.undo_operation());
    assert_eq!(p.component_source(), &before);
    assert!(p.undo_operation());
    assert_eq!(p.get_pool().objects().len(), 1);
    assert!(p.redo_operation());
    assert!(p.redo_operation());
    assert_eq!(p.component_source().instances.len(), 1);
}

#[test]
fn legacy_project_and_empty_project_load_without_component_metadata() {
    let p = fixture();
    let mut json: serde_json::Value = serde_json::from_slice(&p.save_project().unwrap()).unwrap();
    json.as_object_mut().unwrap().remove("components");
    let loaded = EditorProject::load_project(serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(loaded.component_source().definitions.is_empty());
    let empty = EditorProject::new_pool();
    assert!(EditorProject::load_project(empty.save_project().unwrap())
        .unwrap()
        .get_pool()
        .objects()
        .is_empty());
}

#[test]
fn captured_macro_remains_explicit_external_and_is_not_copied() {
    use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef};
    let mut pool = fixture().get_pool().clone();
    let mut action = default_object(ObjectType::Macro);
    action.mut_id().set_value(8).unwrap();
    pool.add(action);
    if let Some(Object::Button(button)) = pool.object_mut_by_id(ObjectId::new(100).unwrap()) {
        button.macro_refs.push(MacroRef {
            event_id: Event::OnKeyPress,
            macro_id: 8,
        });
    }
    let mut p = EditorProject::from(pool.clone());
    let plan = plan_component_capture(&p, 100, "Action".into()).unwrap();
    assert!(plan
        .external_references
        .iter()
        .any(|(_, property, id)| property.starts_with("macro") && *id == 8));
    ComponentService::create_component_from_selection(&mut p, plan, true).unwrap();
    assert_eq!(
        objects_by_id(&pool),
        objects_by_id(&p.materialize().unwrap().pool)
    );
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
fn detached_objects_are_independent_of_later_master_changes() {
    let (mut p, component, instance) = captured();
    ComponentService::detach_component_instance(&mut p, instance).unwrap();
    let before = objects_by_id(p.get_pool());
    let mut master = p.component_source().definitions[&component].clone();
    master
        .nodes
        .iter_mut()
        .find(|n| n.id == master.root_node)
        .unwrap()
        .properties
        .insert(property("height"), json!(80));
    ComponentService::edit_component_definition(&mut p, master).unwrap();
    assert_eq!(before, objects_by_id(&p.materialize().unwrap().pool));
}

#[test]
fn browser_master_editor_and_instance_inspector_render_headlessly() {
    let (mut p, component, instance) = captured();
    let _ = parameter(&mut p, component);
    let context = egui::Context::default();
    let definition = p.component_source().definitions[&component].clone();
    context.data_mut(|d| {
        d.insert_temp(
            egui::Id::new("component_edit"),
            serde_json::to_string_pretty(&definition).unwrap(),
        )
    });
    let output = context.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            components::ui::show(ui, &mut p);
            components::ui::inspector(ui, &mut p, instance, definition.root_node);
        });
    });
    assert!(!output.shapes.is_empty());
}

#[test]
fn unpositioned_list_items_remain_explicit_external_references() {
    let mut pool = fixture().get_pool().clone();
    pool.remove(ObjectId::new(100).unwrap());
    let mut input = default_object(ObjectType::InputList);
    input.mut_id().set_value(100).unwrap();
    if let Object::InputList(list) = &mut input {
        list.list_items.push(ObjectId::new(101).unwrap().into());
    }
    pool.add(input);
    let mut p = EditorProject::from(pool.clone());
    let plan = plan_component_capture(&p, 100, "Choice".into()).unwrap();
    assert!(plan
        .external_references
        .iter()
        .any(|(_, property, target)| property == "object_list" && *target == 101));
    ComponentService::create_component_from_selection(&mut p, plan, true).unwrap();
    assert_eq!(
        objects_by_id(&pool),
        objects_by_id(&p.materialize().unwrap().pool)
    );
}

#[test]
fn component_selection_survives_navigation_and_clears_on_delete() {
    let (mut p, _, instance) = captured();
    p.get_mut_selected()
        .set(ag_iso_stack::object_pool::NullableObjectId::new(2));
    p.update_selected();
    p.get_mut_selected()
        .set(ag_iso_stack::object_pool::NullableObjectId::new(101));
    p.update_selected();
    assert!(
        matches!(p.editor_selection(), Some(EditorSelection::ComponentNode { instance_id, .. }) if instance_id == instance)
    );
    p.set_previous_selected();
    assert_eq!(
        p.editor_selection(),
        Some(EditorSelection::PrimitiveObject(2))
    );
    p.set_next_selected();
    assert!(matches!(
        p.editor_selection(),
        Some(EditorSelection::ComponentNode { .. })
    ));
    ComponentService::delete_component_instance(&mut p, instance).unwrap();
    assert!(p.editor_selection().is_none());
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
