use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool, ObjectType};
use ag_iso_terminal_designer::{
    default_object, EditorProject, ObjectTarget, Operation, OperationTransaction,
    CURRENT_OPERATION_SCHEMA_VERSION,
};

fn fixture() -> (EditorProject, u16, u16, u16) {
    let mut pool = ObjectPool::default();
    for (id, object_type) in [
        (10, ObjectType::ObjectPointer),
        (20, ObjectType::Button),
        (30, ObjectType::Button),
        (40, ObjectType::ObjectPointer),
    ] {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        pool.add(object);
    }
    (EditorProject::from(pool), 10, 20, 30)
}

fn transact(project: &mut EditorProject, operations: Vec<Operation>) -> bool {
    project
        .execute_transaction(OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some("Object pointer references test".into()),
            operations,
        })
        .is_ok()
}

fn allowed(pointer: u16, ids: &[u16]) -> Operation {
    Operation::SetObjectPointerAllowedReferences {
        object: ObjectTarget::id(pointer),
        references: ids.iter().copied().map(ObjectTarget::id).collect(),
    }
}

fn object_id(id: u16) -> ObjectId {
    ObjectId::new(id).unwrap()
}

fn allowed_ids(project: &EditorProject, pointer_id: u16) -> Vec<u16> {
    let pointer = project
        .get_pool()
        .object_by_id(object_id(pointer_id))
        .unwrap();
    project
        .get_object_info(pointer)
        .allowed_references
        .iter()
        .map(|id| id.value())
        .collect()
}

#[test]
fn allowed_references_round_trip_and_legacy_metadata_defaults_empty() {
    let (mut project, pointer, first, second) = fixture();
    assert!(transact(
        &mut project,
        vec![allowed(pointer, &[second, first])]
    ));

    let saved = project.save_project().unwrap();
    let loaded = EditorProject::load_project(saved.clone()).unwrap();
    assert_eq!(allowed_ids(&loaded, pointer), vec![second, first]);
    assert_eq!(loaded.export_iop().unwrap(), project.export_iop().unwrap());

    // Older .aitp metadata has no allowed_references property.
    let mut legacy: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    let metadata = legacy["object_metadata"].as_object_mut().unwrap();
    for object_metadata in metadata.values_mut() {
        object_metadata
            .as_object_mut()
            .unwrap()
            .remove("allowed_references");
    }
    let legacy = EditorProject::load_project(serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(allowed_ids(&legacy, pointer).is_empty());
}

#[test]
fn list_only_edit_is_dirty_undoable_and_does_not_change_iop() {
    let (mut project, pointer, first, second) = fixture();
    project.mark_saved();
    let before = project.export_iop().unwrap();

    assert!(transact(
        &mut project,
        vec![allowed(pointer, &[first, second])]
    ));
    assert!(project.is_dirty());
    assert_eq!(project.export_iop().unwrap(), before);
    assert_eq!(allowed_ids(&project, pointer), vec![first, second]);

    assert!(project.undo_operation());
    assert!(allowed_ids(&project, pointer).is_empty());
    assert!(!project.is_dirty());
    assert!(project.redo_operation());
    assert_eq!(allowed_ids(&project, pointer), vec![first, second]);
    assert!(project.is_dirty());
}

#[test]
fn excluding_active_target_clears_it_and_one_undo_restores_both() {
    let (mut project, pointer, active, other) = fixture();
    assert!(transact(
        &mut project,
        vec![Operation::SetObjectReference {
            object: ObjectTarget::id(pointer),
            property: "value".into(),
            reference: Some(ObjectTarget::id(active)),
        }],
    ));
    assert!(transact(
        &mut project,
        vec![allowed(pointer, &[active, other])]
    ));

    assert!(transact(&mut project, vec![allowed(pointer, &[other])]));
    let Object::ObjectPointer(pointer_object) =
        project.get_pool().object_by_id(object_id(pointer)).unwrap()
    else {
        panic!("fixture pointer has the wrong type");
    };
    assert!(pointer_object.value.0.is_none());
    assert_eq!(allowed_ids(&project, pointer), vec![other]);

    assert!(project.undo_operation());
    let Object::ObjectPointer(pointer_object) =
        project.get_pool().object_by_id(object_id(pointer)).unwrap()
    else {
        panic!("fixture pointer has the wrong type");
    };
    assert_eq!(pointer_object.value.0, Some(object_id(active)));
    assert_eq!(allowed_ids(&project, pointer), vec![active, other]);
}

#[test]
fn disallowed_reference_assignment_is_rejected_without_mutation() {
    let (mut project, pointer, allowed_target, denied_target) = fixture();
    assert!(transact(
        &mut project,
        vec![allowed(pointer, &[allowed_target])]
    ));
    let before = project.export_iop().unwrap();

    assert!(!transact(
        &mut project,
        vec![Operation::SetObjectReference {
            object: ObjectTarget::id(pointer),
            property: "value".into(),
            reference: Some(ObjectTarget::id(denied_target)),
        }],
    ));
    assert_eq!(project.export_iop().unwrap(), before);
}

#[test]
fn allowed_target_and_none_switching_work_and_empty_list_preserves_active_target() {
    let (mut project, pointer, first, second) = fixture();
    assert!(transact(
        &mut project,
        vec![allowed(pointer, &[first, second])]
    ));

    for reference in [Some(first), None, Some(second)] {
        assert!(transact(
            &mut project,
            vec![Operation::SetObjectReference {
                object: ObjectTarget::id(pointer),
                property: "value".into(),
                reference: reference.map(ObjectTarget::id),
            }],
        ));
        let Object::ObjectPointer(pointer_object) =
            project.get_pool().object_by_id(object_id(pointer)).unwrap()
        else {
            panic!("fixture pointer has the wrong type");
        };
        assert_eq!(pointer_object.value.0, reference.map(object_id));
    }

    // Removing all restrictions keeps the current value and restores unrestricted edits.
    assert!(transact(&mut project, vec![allowed(pointer, &[])]));
    let Object::ObjectPointer(pointer_object) =
        project.get_pool().object_by_id(object_id(pointer)).unwrap()
    else {
        panic!("fixture pointer has the wrong type");
    };
    assert_eq!(pointer_object.value.0, Some(object_id(second)));
    assert!(transact(
        &mut project,
        vec![Operation::SetObjectReference {
            object: ObjectTarget::id(pointer),
            property: "value".into(),
            reference: Some(ObjectTarget::id(first)),
        }],
    ));
}

#[test]
fn allowed_reference_lists_reject_duplicate_self_non_pointer_and_cycle_entries() {
    let (mut project, pointer, first, _) = fixture();
    assert!(!transact(
        &mut project,
        vec![allowed(pointer, &[first, first])]
    ));
    assert!(!transact(&mut project, vec![allowed(pointer, &[pointer])]));
    assert!(!transact(&mut project, vec![allowed(first, &[pointer])]));

    // The reverse edge makes the proposed allowed target circular.
    assert!(transact(
        &mut project,
        vec![Operation::SetObjectReference {
            object: ObjectTarget::id(40),
            property: "value".into(),
            reference: Some(ObjectTarget::id(pointer)),
        }],
    ));
    assert!(!transact(&mut project, vec![allowed(pointer, &[40])]));
}

#[test]
fn narrowing_list_keeps_a_newly_circular_target_unselectable() {
    let (mut project, pointer_a, _, other) = fixture();
    let target_b = 40;
    assert!(transact(
        &mut project,
        vec![allowed(pointer_a, &[target_b, other])]
    ));
    assert!(transact(
        &mut project,
        vec![Operation::SetObjectReference {
            object: ObjectTarget::id(target_b),
            property: "value".into(),
            reference: Some(ObjectTarget::id(pointer_a)),
        }],
    ));

    let selection = Operation::SetObjectReference {
        object: ObjectTarget::id(pointer_a),
        property: "value".into(),
        reference: Some(ObjectTarget::id(target_b)),
    };
    assert!(!transact(&mut project, vec![selection.clone()]));

    // Keeping the existing B entry while removing C is valid, even though B
    // became cyclic after the other pointer was changed.
    assert!(transact(
        &mut project,
        vec![allowed(pointer_a, &[target_b])]
    ));
    assert_eq!(allowed_ids(&project, pointer_a), vec![target_b]);
    assert!(!transact(&mut project, vec![selection]));
}

#[test]
fn changing_target_id_remaps_allowed_list_and_undo_restores_it() {
    let (mut project, pointer, target, _) = fixture();
    assert!(transact(&mut project, vec![allowed(pointer, &[target])]));
    assert!(transact(
        &mut project,
        vec![Operation::ChangeObjectId {
            old_id: target,
            new_id: 25,
        }],
    ));
    assert_eq!(allowed_ids(&project, pointer), vec![25]);

    assert!(project.undo_operation());
    assert_eq!(allowed_ids(&project, pointer), vec![target]);
}

#[test]
fn deleting_an_object_in_an_allowed_list_is_blocked() {
    let (mut project, pointer, target, _) = fixture();
    assert!(transact(&mut project, vec![allowed(pointer, &[target])]));
    assert!(!project.delete_object(object_id(target)));
    assert!(project.get_pool().object_by_id(object_id(target)).is_some());
}
