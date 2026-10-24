use ag_iso_stack::object_pool::{
    object::Object, NullableObjectId, ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::cli::editing::execute_batch;
use ag_iso_terminal_designer::{
    default_object, EditorProject, ObjectIdRange, ObjectTarget, Operation, OperationError,
    OperationTransaction, PropertyKey, CURRENT_OPERATION_SCHEMA_VERSION,
};
use serde_json::json;

fn object_with_id(object_type: ObjectType, id: u16) -> Object {
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    object
}

#[test]
fn incomplete_draft_round_trips_but_complete_diagnostics_and_iop_export_reject_it() {
    let mut project = EditorProject::default();
    let mut create = OperationTransaction::new(None);
    create.add_operation(Operation::CreateObject {
        object_id: Some(1),
        object_type: ObjectType::OutputPolygon,
        name: "Draft polygon".into(),
    });
    create.add_operation(Operation::SetProperty {
        object: ObjectTarget::id(1),
        property: PropertyKey::Points,
        value: json!([]),
    });
    project.execute_transaction(create).unwrap();
    let restored = EditorProject::load_project(project.save_project().unwrap()).unwrap();

    assert!(restored.save_project().is_ok());
    let diagnostics = restored.compatibility_diagnostics().unwrap();
    assert!(diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.code == "WORKING_SET_CARDINALITY" }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E007_OBJECT_NOT_FOUND" && diagnostic.affected_objects == Some(vec![0])
    }));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "POLYGON_TOO_FEW_POINTS"));

    let mut edit_width = OperationTransaction::new(None);
    edit_width.add_operation(Operation::SetProperty {
        object: ObjectTarget::id(1),
        property: PropertyKey::Width,
        value: 40.into(),
    });
    let mut restored = restored;
    restored.execute_transaction(edit_width).unwrap();
    assert!(restored.export_iop().is_err());
}

#[test]
fn edit_batch_commits_objects_and_settings_as_one_checkpoint() {
    let mut project = EditorProject::default();
    let before = project.save_project().unwrap();
    let revision = project.revision();
    execute_batch(&mut project, json!({"actions": [
        {"kind": "object_create", "object_type": "NumberVariable", "name": "Counter", "override_id": null},
        {"kind": "settings", "values": {"mask_size": "640", "soft_key_width": "80", "soft_key_height": "96"}}
    ]})).unwrap();
    assert_eq!(project.revision(), revision + 1);
    assert_eq!(project.get_pool().size(), 1);
    assert_eq!(project.mask_size, 640);
    assert_eq!(project.soft_key_size(), (80, 96));
    let after = project.save_project().unwrap();
    assert!(project.undo_operation());
    assert_eq!(project.save_project().unwrap(), before);
    assert!(!project.undo_operation());
    assert!(project.redo_operation());
    assert_eq!(project.save_project().unwrap(), after);
}

#[test]
fn edit_batch_failure_check_and_noop_preserve_live_content_and_redo() {
    let mut project = EditorProject::default();
    project.set_design_dimensions(640, (80, 96)).unwrap();
    assert!(project.undo_operation());
    let before = project.save_project().unwrap();
    let revision = project.revision();
    assert!(execute_batch(&mut project, json!({"actions": [
        {"kind": "object_create", "object_type": "NumberVariable", "name": "Temporary", "override_id": null},
        {"kind": "settings", "values": {"mask_size": "0"}}
    ]})).is_err());
    let checked = execute_batch(
        &mut project,
        json!({"check": true, "actions": [
            {"kind": "settings", "values": {"mask_size": "800"}}
        ]}),
    )
    .unwrap();
    assert!(checked["transaction_id"].is_null());
    let size = project.mask_size.to_string();
    let noop = execute_batch(
        &mut project,
        json!({"actions": [
            {"kind": "settings", "values": {"mask_size": size}}
        ]}),
    )
    .unwrap();
    assert!(noop["transaction_id"].is_null());
    assert_eq!(project.save_project().unwrap(), before);
    assert_eq!(project.revision(), revision);
    assert!(project.redo_operation());
    assert_eq!(project.mask_size, 640);
}

#[test]
fn operation_delete_object_rejects_referenced() {
    use ag_iso_stack::object_pool::object_attributes::{ObjectRef, Point};
    use ag_iso_terminal_designer::operations::{Operation, OperationTransaction};

    let mut parent = object_with_id(ObjectType::DataMask, 10);
    let child = object_with_id(ObjectType::Button, 11);
    if let Object::DataMask(mask) = &mut parent {
        mask.object_refs.push(ObjectRef {
            id: child.id(),
            offset: Point::default(),
        });
    }
    let mut pool = ObjectPool::default();
    pool.add(parent);
    pool.add(child);
    let mut project = EditorProject::from(pool);

    let mut tx = OperationTransaction::new(Some("Delete referenced button".to_string()));
    tx.add_operation(Operation::DeleteObject { object_id: 11 });

    let result = project.execute_transaction(tx);
    assert!(matches!(
        result,
        Err(OperationError::DeleteReferencedObject(11))
    ));
}

#[test]
fn allocator_reports_type_range_exhaustion() {
    let button = object_with_id(ObjectType::Button, 6000);
    let mut pool = ObjectPool::default();
    pool.add(button);
    let mut project = EditorProject::from(pool);
    let mut ranges = project.object_id_ranges().clone();
    ranges.set_range(
        ObjectType::Button,
        ObjectIdRange {
            start: 6000,
            end: 6000,
        },
    );
    project.set_object_id_ranges(ranges).unwrap();
    let error = project.allocate_object_id(ObjectType::Button).unwrap_err();
    assert_eq!(error.object_type, ObjectType::Button);
    assert_eq!(error.range.start, 6000);
    assert!(error.to_string().contains("No free Button object ID"));
}

#[test]
fn allocator_reserves_ids_for_queued_automatic_creates() {
    let mut project = EditorProject::new_pool();
    project.queue_operation(Operation::CreateObject {
        object_id: None,
        object_type: ObjectType::Button,
        name: "Queued button".into(),
    });
    assert_eq!(
        project
            .allocate_object_id(ObjectType::Button)
            .unwrap()
            .value(),
        6001
    );
    assert!(project.commit_queued("Create queued button"));
    let next = project.allocate_object_id(ObjectType::Button).unwrap();
    assert_eq!(next.value(), 6001);
}

#[test]
fn changing_an_object_id_rewrites_all_reference_kinds_and_selection() {
    use ag_iso_stack::object_pool::object_attributes::{
        Event, MacroRef, ObjectLabel, ObjectRef, Point,
    };

    let target = object_with_id(ObjectType::Macro, 10);
    let mut positioned_parent = object_with_id(ObjectType::Button, 20);
    if let Object::Button(button) = &mut positioned_parent {
        button.object_refs.push(ObjectRef {
            id: target.id(),
            offset: Point::default(),
        });
        button.macro_refs.push(MacroRef {
            macro_id: 10,
            event_id: Event::OnKeyPress,
        });
    }
    let mut required_list = object_with_id(ObjectType::SoftKeyMask, 21);
    if let Object::SoftKeyMask(mask) = &mut required_list {
        mask.objects.push(target.id());
    }
    let mut nullable_list = object_with_id(ObjectType::InputList, 22);
    if let Object::InputList(list) = &mut nullable_list {
        list.list_items.push(target.id().into());
    }
    let mut labels = object_with_id(ObjectType::ObjectLabelReferenceList, 23);
    if let Object::ObjectLabelReferenceList(list) = &mut labels {
        list.object_labels.push(ObjectLabel {
            id: target.id(),
            string_variable_reference: target.id().into(),
            font_type: 0,
            graphic_representation: target.id().into(),
        });
    }
    let mut property_parent = object_with_id(ObjectType::ObjectPointer, 24);
    if let Object::ObjectPointer(pointer) = &mut property_parent {
        pointer.value = target.id().into();
    }

    let mut pool = ObjectPool::default();
    for object in [
        target.clone(),
        positioned_parent,
        required_list,
        nullable_list,
        labels,
        property_parent,
    ] {
        pool.add(object);
    }
    let mut project = EditorProject::from(pool);
    project
        .get_mut_selected()
        .replace(NullableObjectId::new(target.id().value()));
    project.update_selected();
    assert!(project.rename_object(target.id(), "Renumbered macro".to_owned()));

    let mut transaction = OperationTransaction::new(Some("Renumber macro".to_owned()));
    transaction.add_operation(Operation::ChangeObjectId {
        old_id: 10,
        new_id: 30,
    });
    project.execute_transaction(transaction).unwrap();

    let new_id = ObjectId::new(30).unwrap();
    assert!(project.get_pool().object_by_id(target.id()).is_none());
    assert!(project.get_pool().object_by_id(new_id).is_some());
    assert_eq!(project.get_selected(), NullableObjectId::new(30));
    assert!(matches!(
        project
            .get_pool()
            .object_by_id(ObjectId::new(20).unwrap()),
        Some(Object::Button(button))
            if button.macro_refs.first().map(|reference| reference.macro_id) == Some(30)
    ));
    assert_eq!(
        project
            .get_pool()
            .objects()
            .iter()
            .filter(|object| object.id() != new_id)
            .flat_map(Object::referenced_objects)
            .filter(|id| *id == target.id())
            .count(),
        0
    );
    assert_eq!(
        project
            .get_object_info(project.get_pool().object_by_id(new_id).unwrap())
            .name
            .as_deref(),
        Some("Renumbered macro")
    );

    assert!(project.undo_operation());
    assert!(project.get_pool().object_by_id(target.id()).is_some());
    assert_eq!(project.get_selected(), NullableObjectId::new(10));
    assert!(project.redo_operation());
    assert!(project.get_pool().object_by_id(new_id).is_some());
    assert_eq!(project.get_selected(), NullableObjectId::new(30));
}

#[test]
fn simultaneous_id_swap_updates_selection_once_and_round_trips_history() {
    let values = |project: &EditorProject| {
        [10, 20].map(|id| {
            let Object::NumberVariable(number) = project
                .get_pool()
                .object_by_id(ObjectId::new(id).unwrap())
                .unwrap()
            else {
                panic!("expected number variable");
            };
            number.value
        })
    };
    let mut first = object_with_id(ObjectType::NumberVariable, 10);
    let mut second = object_with_id(ObjectType::NumberVariable, 20);
    if let Object::NumberVariable(number) = &mut first {
        number.value = 123;
    }
    if let Object::NumberVariable(number) = &mut second {
        number.value = 456;
    }
    let mut pool = ObjectPool::default();
    pool.add(first);
    pool.add(second);
    let mut project = EditorProject::from(pool);
    project
        .get_mut_selected()
        .replace(NullableObjectId::new(10));
    project.update_selected();

    project
        .execute_transaction(OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some("Swap IDs".into()),
            operations: vec![Operation::RemapObjectIds {
                mappings: vec![(10, 20), (20, 10)],
                target_types: vec![
                    (10, ObjectType::NumberVariable),
                    (20, ObjectType::NumberVariable),
                ],
            }],
        })
        .unwrap();
    assert!(project
        .get_pool()
        .object_by_id(ObjectId::new(10).unwrap())
        .is_some());
    assert!(project
        .get_pool()
        .object_by_id(ObjectId::new(20).unwrap())
        .is_some());
    assert_eq!(project.get_selected(), NullableObjectId::new(20));
    assert_eq!(values(&project), [456, 123]);

    assert!(project.undo_operation());
    assert_eq!(project.get_selected(), NullableObjectId::new(10));
    assert_eq!(values(&project), [123, 456]);
    assert!(project.redo_operation());
    assert_eq!(project.get_selected(), NullableObjectId::new(20));
    assert_eq!(values(&project), [456, 123]);
}

#[test]
fn reassignment_preview_rejects_malformed_macro_without_mutation() {
    let button = object_with_id(ObjectType::Button, 10);
    let mut macro_object = object_with_id(ObjectType::Macro, 11);
    if let Object::Macro(macro_object) = &mut macro_object {
        macro_object.commands = vec![0xFF];
    }
    let mut pool = ObjectPool::default();
    pool.add(button);
    pool.add(macro_object);
    let project = EditorProject::from(pool);
    let before = project.get_pool().as_iop();

    assert!(project.plan_id_reassignment(false).is_err());
    assert_eq!(project.get_pool().as_iop(), before);
    assert!(!project.undo_available());
}

#[test]
fn checked_macro_remapping_rejects_invalid_commands_without_mutation() {
    let remap = || OperationTransaction {
        schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
        description: Some("Renumber button".into()),
        operations: vec![Operation::RemapObjectIds {
            mappings: vec![(10, 6000)],
            target_types: vec![(10, ObjectType::Button)],
        }],
    };
    for commands in [
        vec![0xA0, 12, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF],
        vec![0xBE, 10, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        vec![0xAF, 10, 0, 99, 10, 0, 0, 0],
        vec![0xAF, 10, 0, 3, 10, 0, 0, 0],
    ] {
        let mut macro_object = object_with_id(ObjectType::Macro, 11);
        if let Object::Macro(object) = &mut macro_object {
            object.commands = commands.clone();
        }
        let mut pool = ObjectPool::default();
        pool.add(object_with_id(ObjectType::Button, 10));
        pool.add(macro_object);
        let mut project = EditorProject::from(pool);
        let before = project.save_project().unwrap();
        let revision = project.revision();
        if commands[3] == 3 {
            project.execute_transaction(remap()).unwrap();
            let Object::Macro(object) = project
                .get_pool()
                .object_by_id(ObjectId::new(11).unwrap())
                .unwrap()
            else {
                panic!("expected macro");
            };
            assert_eq!(&object.commands[1..3], &6000_u16.to_le_bytes());
            assert_eq!(&object.commands[4..], &commands[4..]);
            assert!(project.undo_operation());
            assert_eq!(project.save_project().unwrap(), before);
        } else {
            assert!(project.execute_transaction(remap()).is_err());
            assert_eq!(project.save_project().unwrap(), before);
            assert_eq!(project.revision(), revision);
            assert!(!project.undo_available());
        }
    }
}

#[test]
fn variables_consumed_by_components_reject_invalid_edits_and_preserve_repair_history() {
    use ag_iso_stack::object_pool::{object_attributes::Point, ObjectRef};
    use ag_iso_terminal_designer::components::{plan_component_capture, ComponentService};

    let mut mask = object_with_id(ObjectType::DataMask, 1);
    if let Object::DataMask(mask) = &mut mask {
        mask.object_refs.push(ObjectRef {
            id: ObjectId::new(2).unwrap(),
            offset: Point::default(),
        });
    }
    let mut meter = object_with_id(ObjectType::OutputMeter, 2);
    if let Object::OutputMeter(meter) = &mut meter {
        meter.width = 40;
        meter.variable_reference = NullableObjectId::new(3);
    }
    let mut variable = object_with_id(ObjectType::NumberVariable, 3);
    if let Object::NumberVariable(variable) = &mut variable {
        variable.value = 65_535;
    }
    let mut pool = ObjectPool::new();
    for object in [mask, meter, variable] {
        pool.add(object);
    }
    let mut project = EditorProject::from(pool);
    let capture = plan_component_capture(&project, 2, "Meter".into()).unwrap();
    ComponentService::create_component_from_selection(&mut project, capture, true).unwrap();
    assert!(project
        .get_pool()
        .object_by_id(ObjectId::new(2).unwrap())
        .is_none());

    let set_value = |value| {
        let mut transaction = OperationTransaction::new(None);
        transaction.add_operation(Operation::SetProperty {
            object: ObjectTarget::id(3),
            property: PropertyKey::Value,
            value: json!(value),
        });
        transaction
    };
    let before = project.save_project().unwrap();
    let revision = project.revision();
    assert!(project.execute_transaction(set_value(65_536)).is_err());
    assert_eq!(project.save_project().unwrap(), before);
    assert_eq!(project.revision(), revision);

    // Simulate an existing authoring draft with a now-invalid shared value.
    let mut saved: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let variable = saved["objects"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|object| object["type"] == "NumberVariable")
        .unwrap();
    variable["properties"]["value"] = json!(65_536);
    let mut project = EditorProject::load_project(serde_json::to_vec(&saved).unwrap()).unwrap();
    let draft = project.save_project().unwrap();
    assert!(project.execute_transaction(set_value(65_537)).is_err());
    assert_eq!(project.save_project().unwrap(), draft);
    let mut unrelated = OperationTransaction::new(None);
    unrelated.add_operation(Operation::SetProperty {
        object: ObjectTarget::id(1),
        property: PropertyKey::BackgroundColour,
        value: json!(12),
    });
    project.execute_transaction(unrelated).unwrap();
    let before_repair = project.save_project().unwrap();
    project.execute_transaction(set_value(65_535)).unwrap();
    assert!(!project
        .compatibility_diagnostics()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic.code == "ISO_PROPERTY_INVALID"));
    assert!(project.undo_operation());
    assert_eq!(project.save_project().unwrap(), before_repair);
    assert!(project
        .compatibility_diagnostics()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic.code == "ISO_PROPERTY_INVALID"));
    assert!(project.redo_operation());
    assert!(!project
        .compatibility_diagnostics()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic.code == "ISO_PROPERTY_INVALID"));
}
