use std::collections::HashMap;

use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::{ObjectId, ObjectPool, ObjectType};
use ag_iso_terminal_designer::{
    default_object, object_properties::PropertyKey, ObjectInfo, ObjectTarget, Operation,
    OperationError, OperationExecutor, OperationTransaction,
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
            property: PropertyKey::Width,
            value: json!(120),
        },
        Operation::SetProperty {
            object: name("Actual Rate"),
            property: PropertyKey::Height,
            value: json!(40),
        },
        Operation::SetObjectReference {
            object: name("Actual Rate"),
            property: PropertyKey::VariableReference,
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
        ag_iso_terminal_designer::object_properties::get_property(
            &changed,
            output_id,
            PropertyKey::Width
        )
        .unwrap(),
        json!(120)
    );
    assert_eq!(
        ag_iso_terminal_designer::object_properties::get_property(
            &changed,
            output_id,
            PropertyKey::VariableReference
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
        property: PropertyKey::Width,
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
        property: PropertyKey::Width,
        value: json!(1),
    });
    assert!(
        matches!(OperationExecutor::new().execute(tx, &duplicate_pool, &infos), Err(OperationError::AmbiguousObjectName(value)) if value == "Existing")
    );
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
            property: PropertyKey::VariableReference,
            value: json!(11),
        },
        Operation::SetObjectReference {
            object: name("Output"),
            property: PropertyKey::Width,
            reference: Some(name("Number")),
        },
        Operation::SetObjectReference {
            object: name("Output"),
            property: PropertyKey::VariableReference,
            reference: Some(name("String")),
        },
        Operation::SetObjectReference {
            object: name("Output"),
            property: PropertyKey::VariableReference,
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
        property: PropertyKey::VariableReference,
        reference: Some(name("Number")),
    });
    let (changed, changed_infos, applied) = OperationExecutor::new()
        .execute(valid, &pool, &infos)
        .unwrap();
    let mut clear = OperationTransaction::new(None);
    clear.add_operation(Operation::SetObjectReference {
        object: name("Output"),
        property: PropertyKey::VariableReference,
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
