use std::collections::HashMap;

use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::object_attributes::Event;
use ag_iso_stack::object_pool::{ObjectId, ObjectPool, ObjectType};
use ag_iso_terminal_designer::{
    default_object, ChildPlacement, MacroReferenceTarget, ObjectInfo, ObjectTarget,
    ObjectTargetList, Operation, OperationError, OperationExecutor, OperationTransaction,
    CURRENT_OPERATION_SCHEMA_VERSION,
};
use serde_json::json;

fn named_object(object_type: ObjectType, id: u16, name: &str) -> (Object, ObjectInfo) {
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    let mut info = ObjectInfo::new(&object);
    info.name = Some(name.to_owned());
    (object, info)
}

fn pool_with(objects: &[(ObjectType, u16, &str)]) -> (ObjectPool, HashMap<ObjectId, ObjectInfo>) {
    let mut pool = ObjectPool::default();
    let mut infos = HashMap::new();
    for &(object_type, id, name) in objects {
        let (object, info) = named_object(object_type, id, name);
        infos.insert(object.id(), info);
        pool.add(object);
    }
    (pool, infos)
}

fn name(name: &str) -> ObjectTarget {
    ObjectTarget::Name {
        name: name.to_owned(),
    }
}

#[test]
fn schema_v2_json_uses_only_object_shaped_targets() {
    let value = json!({
        "schema_version": 2,
        "description": "Create rate display",
        "operations": [{
            "type": "CreateObject",
            "data": { "name": "Rate Variable", "object_type": "NumberVariable" }
        }, {
            "type": "SetProperty",
            "data": { "object": { "name": "Rate Variable" }, "property": "value", "value": 7 }
        }]
    });
    let transaction: OperationTransaction = serde_json::from_value(value).unwrap();
    assert_eq!(transaction.schema_version, CURRENT_OPERATION_SCHEMA_VERSION);
    assert!(matches!(
        transaction.operations[1],
        Operation::SetProperty {
            object: ObjectTarget::Name { .. },
            ..
        }
    ));
    assert!(serde_json::from_value::<Operation>(json!({
        "type": "RestoreObject",
        "data": { "object": {}, "info": null }
    }))
    .is_err());
}

#[test]
fn same_transaction_creation_resolves_names_and_builds_layout() {
    let (pool, infos) = pool_with(&[(ObjectType::DataMask, 10, "Main Screen")]);
    let operations = vec![
        Operation::CreateObject {
            object_id: None,
            object_type: ObjectType::NumberVariable,
            name: "Rate Variable".into(),
        },
        Operation::CreateObject {
            object_id: None,
            object_type: ObjectType::OutputNumber,
            name: "Actual Rate".into(),
        },
        Operation::SetProperty {
            object: name("Actual Rate"),
            property: "width".into(),
            value: json!(120),
        },
        Operation::SetProperty {
            object: name("Actual Rate"),
            property: "height".into(),
            value: json!(40),
        },
        Operation::SetObjectReference {
            object: name("Actual Rate"),
            property: "variable_reference".into(),
            reference: Some(name("Rate Variable")),
        },
        Operation::AddChild {
            parent: name("Main Screen"),
            child: name("Actual Rate"),
            x: 20,
            y: 40,
        },
    ];
    let mut transaction = OperationTransaction::new(Some("rate display".into()));
    transaction.add_operations(operations);
    let (changed, changed_infos, applied) = OperationExecutor::new()
        .execute(transaction, &pool, &infos)
        .unwrap();

    let rate_id = changed_infos
        .iter()
        .find_map(|(id, info)| (info.name.as_deref() == Some("Rate Variable")).then_some(*id))
        .unwrap();
    let output_id = changed_infos
        .iter()
        .find_map(|(id, info)| (info.name.as_deref() == Some("Actual Rate")).then_some(*id))
        .unwrap();
    assert_eq!(
        ag_iso_terminal_designer::object_properties::get_property(&changed, output_id, "width")
            .unwrap(),
        json!(120)
    );
    assert_eq!(
        ag_iso_terminal_designer::object_properties::get_property(
            &changed,
            output_id,
            "variable_reference"
        )
        .unwrap(),
        serde_json::to_value(rate_id).unwrap()
    );
    let Object::DataMask(mask) = changed.object_by_id(ObjectId::new(10).unwrap()).unwrap() else {
        panic!()
    };
    assert!(mask
        .object_refs
        .iter()
        .any(|reference| reference.id == output_id
            && reference.offset.x == 20
            && reference.offset.y == 40));

    assert!(applied
        .inverse_operations
        .iter()
        .all(|operation| match operation {
            Operation::SetProperty { object, .. }
            | Operation::SetObjectReference { object, .. }
            | Operation::RenameObject { object, .. } => matches!(object, ObjectTarget::Id { .. }),
            Operation::AddChild { parent, child, .. } => matches!(
                (parent, child),
                (ObjectTarget::Id { .. }, ObjectTarget::Id { .. })
            ),
            _ => true,
        }));
}

#[test]
fn unknown_and_ambiguous_names_fail_atomically() {
    let (pool, mut infos) = pool_with(&[(ObjectType::Button, 10, "Existing")]);
    let before = pool.as_iop();
    let mut tx = OperationTransaction::new(None);
    tx.add_operation(Operation::SetProperty {
        object: name("Future Object"),
        property: "width".into(),
        value: json!(100),
    });
    tx.add_operation(Operation::CreateObject {
        object_id: None,
        object_type: ObjectType::Button,
        name: "Future Object".into(),
    });
    assert!(
        matches!(OperationExecutor::new().execute(tx, &pool, &infos), Err(OperationError::ObjectNameNotFound(value)) if value == "Future Object")
    );
    assert_eq!(pool.as_iop(), before);

    let (_, duplicate_info) = named_object(ObjectType::Button, 11, "Existing");
    infos.insert(ObjectId::new(11).unwrap(), duplicate_info);
    let mut duplicate_pool = pool.clone();
    duplicate_pool.add(named_object(ObjectType::Button, 11, "ignored").0);
    let mut tx = OperationTransaction::new(None);
    tx.add_operation(Operation::SetProperty {
        object: name("Existing"),
        property: "width".into(),
        value: json!(1),
    });
    assert!(
        matches!(OperationExecutor::new().execute(tx, &duplicate_pool, &infos), Err(OperationError::AmbiguousObjectName(value)) if value == "Existing")
    );
}

#[test]
fn duplicate_create_and_rename_are_rejected() {
    let (pool, infos) = pool_with(&[
        (ObjectType::Button, 10, "Taken"),
        (ObjectType::Button, 11, "Other"),
    ]);
    for operation in [
        Operation::CreateObject {
            object_id: None,
            object_type: ObjectType::Button,
            name: "Taken".into(),
        },
        Operation::RenameObject {
            object: ObjectTarget::id(11),
            name: "Taken".into(),
        },
    ] {
        let mut tx = OperationTransaction::new(None);
        tx.add_operation(operation);
        assert!(
            matches!(OperationExecutor::new().execute(tx, &pool, &infos), Err(OperationError::DuplicateObjectName(value)) if value == "Taken")
        );
    }
}

#[test]
fn reference_operations_enforce_property_semantics_and_target_type() {
    let (pool, infos) = pool_with(&[
        (ObjectType::OutputNumber, 10, "Output"),
        (ObjectType::NumberVariable, 11, "Number"),
        (ObjectType::StringVariable, 12, "String"),
    ]);
    let cases = [
        Operation::SetProperty {
            object: name("Output"),
            property: "variable_reference".into(),
            value: json!(11),
        },
        Operation::SetObjectReference {
            object: name("Output"),
            property: "width".into(),
            reference: Some(name("Number")),
        },
        Operation::SetObjectReference {
            object: name("Output"),
            property: "variable_reference".into(),
            reference: Some(name("String")),
        },
        Operation::SetObjectReference {
            object: name("Output"),
            property: "variable_reference".into(),
            reference: Some(name("Missing")),
        },
    ];
    for operation in cases {
        let mut tx = OperationTransaction::new(None);
        tx.add_operation(operation);
        assert!(OperationExecutor::new().execute(tx, &pool, &infos).is_err());
    }

    let mut valid = OperationTransaction::new(None);
    valid.add_operation(Operation::SetObjectReference {
        object: ObjectTarget::id(10),
        property: "variable_reference".into(),
        reference: Some(name("Number")),
    });
    let (changed, changed_infos, applied) = OperationExecutor::new()
        .execute(valid, &pool, &infos)
        .unwrap();
    let mut clear = OperationTransaction::new(None);
    clear.add_operation(Operation::SetObjectReference {
        object: name("Output"),
        property: "variable_reference".into(),
        reference: None,
    });
    assert!(OperationExecutor::new()
        .execute(clear, &changed, &changed_infos)
        .is_ok());
    assert!(OperationExecutor::new()
        .execute(applied.create_inverse(), &changed, &changed_infos)
        .is_ok());
}

#[test]
fn name_based_lists_macros_and_rename_round_trip() {
    let (pool, infos) = pool_with(&[
        (ObjectType::SoftKeyMask, 10, "Keys"),
        (ObjectType::Key, 11, "First Key"),
        (ObjectType::Button, 20, "Button"),
        (ObjectType::Macro, 21, "Activate Macro"),
        (ObjectType::DataMask, 30, "Main Screen"),
    ]);
    let before = pool.as_iop();
    let mut tx = OperationTransaction::new(Some("structures".into()));
    tx.add_operation(Operation::SetObjectList {
        object: name("Keys"),
        objects: ObjectTargetList::Required(vec![name("First Key")]),
    });
    tx.add_operation(Operation::SetChildren {
        parent: name("Main Screen"),
        children: vec![ChildPlacement {
            child: name("Button"),
            x: 3,
            y: 4,
        }],
    });
    tx.add_operation(Operation::SetMacroReferences {
        object: name("Button"),
        macro_refs: vec![MacroReferenceTarget {
            event_id: Event::OnActivate,
            macro_object: name("Activate Macro"),
        }],
    });
    tx.add_operation(Operation::RenameObject {
        object: name("Button"),
        name: "Renamed Button".into(),
    });
    let (changed, changed_infos, applied) =
        OperationExecutor::new().execute(tx, &pool, &infos).unwrap();
    assert_ne!(changed.as_iop(), before);

    let (undone, undone_infos, _) = OperationExecutor::new()
        .execute(applied.create_inverse(), &changed, &changed_infos)
        .unwrap();
    assert_eq!(undone.as_iop(), before);
    assert_eq!(
        undone_infos
            .get(&ObjectId::new(20).unwrap())
            .and_then(|info| info.name.as_deref()),
        Some("Button")
    );

    let mut redo = OperationTransaction::new(applied.description.clone());
    redo.add_operations(applied.forward_operations.clone());
    let (redone, redone_infos, _) = OperationExecutor::new()
        .execute(redo, &undone, &undone_infos)
        .unwrap();
    assert_eq!(redone.as_iop(), changed.as_iop());
    assert_eq!(
        redone_infos
            .get(&ObjectId::new(20).unwrap())
            .and_then(|info| info.name.as_deref()),
        Some("Renamed Button")
    );
}

#[test]
fn incompatible_schema_is_rejected() {
    let mut tx = OperationTransaction::new(None);
    tx.schema_version = 1;
    assert!(matches!(
        OperationExecutor::new().execute(tx, &ObjectPool::default(), &HashMap::new()),
        Err(OperationError::UnsupportedSchemaVersion {
            expected: 2,
            actual: 1
        })
    ));
}
