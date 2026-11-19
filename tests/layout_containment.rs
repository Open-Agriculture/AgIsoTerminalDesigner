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
        property: property.parse().unwrap(),
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
fn hierarchy_parent_moves_preserve_position_and_reject_invalid_destinations_atomically() {
    use ag_iso_terminal_designer::canvas::{
        edits::{position, PlacementTarget},
        CanvasState,
    };

    let mut pool = ObjectPool::default();
    pool.add(object_with_id(ObjectType::DataMask, 1));
    pool.add(sized_container(2, 100, 60));
    pool.add(sized_button(3, 20, 20));
    pool.add(sized_container(4, 10, 10));
    add_child(&mut pool, 1, 2, 100, 50);
    add_child(&mut pool, 1, 3, 120, 70);
    add_child(&mut pool, 1, 4, 200, 200);
    let mut project = EditorProject::from(pool);
    let canvas = CanvasState::default();
    let source = PlacementTarget::Primitive {
        parent_id: 1,
        child_index: 1,
    };
    let before_preview = project.save_project().unwrap();
    canvas
        .validate_hierarchy_drop(&project, &source, 1, &[0], 1, &[0, 0], 2, Some(0))
        .unwrap();
    assert_eq!(project.save_project().unwrap(), before_preview);
    assert!(!project.undo_available());

    let moved = canvas
        .reparent_in_hierarchy_at(&mut project, &source, 1, &[0], 1, &[0, 0], 2, Some(0))
        .unwrap();
    assert_eq!(position(&project, &moved), Some((20, 20)));
    let before_invalid = project.save_project().unwrap();
    // The destination is too small; neither the source nor history may change.
    assert!(canvas
        .reparent_in_hierarchy_at(&mut project, &moved, 1, &[0, 0], 1, &[0, 1], 4, None)
        .is_err());
    assert_eq!(project.save_project().unwrap(), before_invalid);
    // A parent cannot become its own child.
    let container = PlacementTarget::Primitive {
        parent_id: 1,
        child_index: 0,
    };
    assert!(canvas
        .reparent_in_hierarchy_at(&mut project, &container, 1, &[0], 1, &[0, 0], 2, None)
        .is_err());
    assert_eq!(project.save_project().unwrap(), before_invalid);

    assert!(project.undo_operation());
    assert_eq!(project.save_project().unwrap(), before_preview);
    assert!(!project.undo_available());
    assert!(project.redo_operation());
    assert_eq!(project.save_project().unwrap(), before_invalid);
}

#[test]
fn property_resize_rejects_overflow_atomically_during_execution_and_preparation() {
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
