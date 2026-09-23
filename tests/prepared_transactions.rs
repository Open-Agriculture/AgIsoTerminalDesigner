use std::collections::HashMap;

use ag_iso_stack::object_pool::{ObjectId, ObjectPool, ObjectType};
use ag_iso_terminal_designer::{
    default_object, ObjectInfo, ObjectTarget, Operation, OperationExecutor, OperationTransaction,
    ValidationContext,
};
use serde_json::json;

fn named_pool() -> (ObjectPool, HashMap<ObjectId, ObjectInfo>) {
    let mut pool = ObjectPool::default();
    let mut button = default_object(ObjectType::Button);
    button.mut_id().set_value(10).unwrap();
    let mut info = ObjectInfo::new(&button);
    info.name = Some("Button".into());
    let id = button.id();
    pool.add(button);
    (pool, HashMap::from([(id, info)]))
}

#[test]
fn preparation_is_non_mutating_and_matches_execution() {
    let (pool, info) = named_pool();
    let before = pool.as_iop();
    let mut tx = OperationTransaction::new(Some("resize".into()));
    tx.add_operation(Operation::SetProperty {
        object: ObjectTarget::Name {
            name: "Button".into(),
        },
        property: "width".into(),
        value: json!(123),
    });
    let executor = OperationExecutor::new();
    let prepared = executor
        .prepare_transaction(&pool, &info, &tx, &ValidationContext::default())
        .unwrap();
    assert_eq!(pool.as_iop(), before);
    let (executed_pool, _, applied) = executor.execute(tx, &pool, &info).unwrap();
    assert_eq!(prepared.pool.as_iop(), executed_pool.as_iop());
    assert_eq!(
        prepared.applied.forward_operations.len(),
        applied.forward_operations.len()
    );
}

#[test]
fn diagnostics_are_indexed_and_validation_is_preparation() {
    let (pool, info) = named_pool();
    let mut tx = OperationTransaction::new(None);
    tx.add_operation(Operation::SetProperty {
        object: ObjectTarget::id(10),
        property: "width".into(),
        value: json!(123),
    });
    tx.add_operation(Operation::SetProperty {
        object: ObjectTarget::Name {
            name: "Missing".into(),
        },
        property: "width".into(),
        value: json!(123),
    });
    let executor = OperationExecutor::new();
    let prepared = executor
        .prepare_transaction(&pool, &info, &tx, &ValidationContext::default())
        .unwrap_err();
    assert_eq!(prepared.0[0].operation_index, Some(1));
    assert_eq!(prepared.0[0].code, "OBJECT_NAME_NOT_FOUND");
}
