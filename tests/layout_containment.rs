use ag_iso_stack::object_pool::{
    object::Object,
    object_attributes::{ObjectRef, Point},
    ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::{
    default_object, ChildPlacement, EditorProject, ObjectTarget, Operation, OperationTransaction,
};
use serde_json::json;

fn object_with_id(object_type: ObjectType, id: u16) -> Object {
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    object
}

fn add_child(pool: &mut ObjectPool, parent_id: u16, child_id: u16, x: i16, y: i16) {
    let parent = pool
        .object_mut_by_id(ObjectId::new(parent_id).unwrap())
        .unwrap();
    let references = match parent {
        Object::DataMask(mask) => &mut mask.object_refs,
        Object::AlarmMask(mask) => &mut mask.object_refs,
        Object::Container(container) => &mut container.object_refs,
        _ => panic!("object {parent_id} cannot contain positioned children"),
    };
    references.push(ObjectRef {
        id: ObjectId::new(child_id).unwrap(),
        offset: Point { x, y },
    });
}

fn sized_button(id: u16, width: u16, height: u16) -> Object {
    let mut object = object_with_id(ObjectType::Button, id);
    if let Object::Button(button) = &mut object {
        button.width = width;
        button.height = height;
    }
    object
}

fn sized_container(id: u16, width: u16, height: u16) -> Object {
    let mut object = object_with_id(ObjectType::Container, id);
    if let Object::Container(container) = &mut object {
        container.width = width;
        container.height = height;
    }
    object
}

fn property(object_id: u16, property: &str, value: serde_json::Value) -> Operation {
    Operation::SetProperty {
        object: ObjectTarget::id(object_id),
        property: property.into(),
        value,
    }
}

fn transaction(operations: impl IntoIterator<Item = Operation>) -> OperationTransaction {
    let mut transaction = OperationTransaction::new(Some("containment test".into()));
    transaction.add_operations(operations.into_iter().collect());
    transaction
}

fn diagnostics_code(project: &EditorProject, transaction: &OperationTransaction) -> String {
    project
        .prepare_transaction(transaction)
        .expect_err("invalid geometry must fail during preparation")
        .0[0]
        .code
        .clone()
}

#[test]
fn property_resize_rejects_overflow_atomically_for_editor_and_proposal_preparation() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 100, 60));
    pool.add(sized_button(3, 20, 20));
    add_child(&mut pool, 1, 2, 0, 0);
    add_child(&mut pool, 2, 3, 80, 10);
    let mut project = EditorProject::from(pool);

    let invalid = transaction([property(3, "width", json!(21))]);
    assert_eq!(
        diagnostics_code(&project, &invalid),
        "OBJECT_OUTSIDE_PARENT"
    );

    let before = project.get_pool().as_iop();
    let revision = project.revision();
    let undo = project
        .operation_history
        .borrow()
        .undo_description()
        .map(str::to_owned);
    assert!(project.execute_transaction(invalid).is_err());
    assert_eq!(project.get_pool().as_iop(), before);
    assert_eq!(project.revision(), revision);
    assert_eq!(
        project.operation_history.borrow().undo_description(),
        undo.as_deref()
    );
}

#[test]
fn exact_boundary_fit_is_valid_and_combined_move_and_resize_uses_final_geometry() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 100, 60));
    pool.add(sized_button(3, 20, 20));
    add_child(&mut pool, 1, 2, 0, 0);
    add_child(&mut pool, 2, 3, 80, 10);
    let mut project = EditorProject::from(pool);

    project
        .execute_transaction(transaction([property(2, "width", json!(80))]))
        .expect_err("shrinking around a child should be rejected");

    let combined = transaction([
        property(2, "width", json!(80)),
        Operation::SetChildPosition {
            parent_id: 2,
            child_id: 3,
            x: 60,
            y: 10,
        },
    ]);
    project
        .execute_transaction(combined)
        .expect("the final child bounds fit after the coordinated edit");
    assert!(matches!(
        project.get_pool().object_by_id(ObjectId::new(2).unwrap()),
        Some(Object::Container(container)) if container.width == 80
    ));
}

#[test]
fn one_shared_object_must_fit_each_parent_and_each_mask_placement() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(object_with_id(ObjectType::AlarmMask, 2));
    pool.add(sized_container(3, 100, 60));
    pool.add(sized_container(4, 50, 60));
    pool.add(sized_button(5, 20, 20));
    add_child(&mut pool, 1, 3, 0, 0);
    add_child(&mut pool, 3, 5, 0, 0);
    add_child(&mut pool, 2, 4, 0, 0);
    add_child(&mut pool, 4, 5, 0, 0);
    let project = EditorProject::from(pool);

    let resize = transaction([property(5, "width", json!(51))]);
    assert_eq!(diagnostics_code(&project, &resize), "OBJECT_OUTSIDE_PARENT");
}

#[test]
fn existing_overflow_can_be_reduced_and_unchanged_by_unrelated_edits() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 60, 20));
    add_child(&mut pool, 1, 2, 0, 0);
    add_child(&mut pool, 2, 3, 0, 0);
    let mut project = EditorProject::from(pool);

    project
        .execute_transaction(transaction([property(3, "width", json!(55))]))
        .expect("reducing existing right-edge overflow must remain available as a repair");
    project
        .execute_transaction(transaction([property(3, "background_colour", json!(3))]))
        .expect("an unrelated edit must preserve, not increase, remaining overflow");
    assert_eq!(
        diagnostics_code(&project, &transaction([property(3, "width", json!(56))])),
        "OBJECT_OUTSIDE_PARENT"
    );
}

#[test]
fn mask_size_reduction_rejects_new_overflow_without_changing_project_state() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_button(2, 20, 20));
    add_child(&mut pool, 1, 2, 480, 20);
    let mut project = EditorProject::from(pool);
    project.mask_size = 500;

    let revision = project.revision();
    let undo = project
        .operation_history
        .borrow()
        .undo_description()
        .map(str::to_owned);
    assert_eq!(
        project
            .set_design_dimensions(490, (80, 96))
            .unwrap_err()
            .code,
        "OBJECT_OUTSIDE_PARENT"
    );
    assert_eq!(project.mask_size, 500);
    assert_eq!(project.soft_key_size(), (60, 60));
    assert_eq!(project.revision(), revision);
    assert_eq!(
        project.operation_history.borrow().undo_description(),
        undo.as_deref()
    );
}

#[test]
fn growing_parent_can_repair_existing_child_overflow() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 60, 20));
    add_child(&mut pool, 1, 2, 0, 0);
    add_child(&mut pool, 2, 3, 0, 0);
    let mut project = EditorProject::from(pool);
    project.mask_size = 100;

    project
        .execute_transaction(transaction([property(2, "width", json!(55))]))
        .expect("reducing the existing overflow from ten to five pixels is a repair");
}

#[test]
fn object_reordering_does_not_increase_legacy_child_overflow() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 60, 20));
    pool.add(sized_button(4, 10, 10));
    add_child(&mut pool, 1, 2, 0, 0);
    add_child(&mut pool, 2, 3, 0, 0);
    let mut project = EditorProject::from(pool);

    project
        .execute_transaction(transaction([Operation::ReorderObjects {
            object_ids: vec![4, 3, 2, 1],
        }]))
        .expect("changing serialization order must not make existing overflow worse");
}

#[test]
fn replacing_a_legacy_overflowing_occurrence_with_a_new_one_is_rejected() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 60, 20));
    add_child(&mut pool, 1, 2, 0, 0);
    add_child(&mut pool, 2, 3, -10, 0);
    let project = EditorProject::from(pool);

    let replace = transaction([
        Operation::AddChild {
            parent: ObjectTarget::id(2),
            child: ObjectTarget::id(3),
            x: -5,
            y: 0,
        },
        Operation::RemoveChild {
            parent_id: 2,
            child_id: 3,
        },
    ]);
    assert_eq!(
        diagnostics_code(&project, &replace),
        "OBJECT_OUTSIDE_PARENT"
    );
}

#[test]
fn set_children_cannot_hide_new_overflow_behind_a_repaired_duplicate_occurrence() {
    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 60, 20));
    add_child(&mut pool, 1, 2, 0, 0);
    add_child(&mut pool, 2, 3, -10, 0);
    let project = EditorProject::from(pool);

    let changed = transaction([Operation::SetChildren {
        parent: ObjectTarget::id(2),
        children: vec![
            ChildPlacement {
                child: ObjectTarget::id(3),
                x: -5,
                y: 0,
            },
            ChildPlacement {
                child: ObjectTarget::id(3),
                x: 0,
                y: 0,
            },
        ],
    }]);
    assert_eq!(
        diagnostics_code(&project, &changed),
        "OBJECT_OUTSIDE_PARENT"
    );
}

#[test]
fn adding_and_reordering_a_contained_duplicate_preserves_the_legacy_survivor() {
    let mut pool = ObjectPool::default();
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 20, 20));
    add_child(&mut pool, 2, 3, 40, 0);
    let mut project = EditorProject::from(pool);

    project
        .execute_transaction(transaction([
            Operation::AddChild {
                parent: ObjectTarget::id(2),
                child: ObjectTarget::id(3),
                x: 0,
                y: 0,
            },
            Operation::SetChildren {
                parent: ObjectTarget::id(2),
                children: vec![
                    ChildPlacement {
                        child: ObjectTarget::id(3),
                        x: 0,
                        y: 0,
                    },
                    ChildPlacement {
                        child: ObjectTarget::id(3),
                        x: 40,
                        y: 0,
                    },
                ],
            },
        ]))
        .expect("reordering must preserve the old placement's overflow allowance");
    assert!(project.undo_operation());
    assert!(project.redo_operation());
}

#[test]
fn repairing_a_survivor_does_not_grant_its_overflow_allowance_to_an_added_reference() {
    let mut pool = ObjectPool::default();
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 20, 20));
    add_child(&mut pool, 2, 3, 40, 0);
    let project = EditorProject::from(pool);

    let changed = transaction([
        Operation::SetChildPosition {
            parent_id: 2,
            child_id: 3,
            x: 20,
            y: 0,
        },
        Operation::AddChild {
            parent: ObjectTarget::id(2),
            child: ObjectTarget::id(3),
            x: 40,
            y: 0,
        },
    ]);
    assert_eq!(
        diagnostics_code(&project, &changed),
        "OBJECT_OUTSIDE_PARENT"
    );
}

#[test]
fn renumbering_objects_preserves_legacy_overflow_and_undo() {
    let mut pool = ObjectPool::default();
    pool.add(sized_container(2, 50, 50));
    pool.add(sized_button(3, 60, 20));
    add_child(&mut pool, 2, 3, 0, 0);
    let mut project = EditorProject::from(pool);
    let before = project.get_pool().as_iop();
    project
        .execute_transaction(transaction([
            Operation::ChangeObjectId {
                old_id: 2,
                new_id: 12,
            },
            Operation::ChangeObjectId {
                old_id: 3,
                new_id: 13,
            },
        ]))
        .expect("renumbering must not count unchanged geometry as new overflow");
    assert_eq!(
        diagnostics_code(&project, &transaction([property(13, "width", json!(61))])),
        "OBJECT_OUTSIDE_PARENT"
    );
    assert!(project.undo_operation());
    assert_eq!(project.get_pool().as_iop(), before);
}
