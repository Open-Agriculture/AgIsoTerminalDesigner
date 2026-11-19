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
    for (kind, id) in [
        (ObjectType::WorkingSet, 65000),
        (ObjectType::DataMask, 65001),
        (ObjectType::PictureGraphic, 65002),
    ] {
        let mut object = default_object(kind);
        *object.mut_id() = object_id(id);
        match &mut object {
            Object::WorkingSet(ws) => {
                ws.active_mask = object_id(65001);
                ws.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
                    id: object_id(65002),
                    offset: Default::default(),
                });
            }
            Object::PictureGraphic(picture) => {
                picture.width = 8;
                picture.actual_width = 8;
                picture.actual_height = 1;
                picture.data = vec![0];
            }
            _ => {}
        }
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
            property: ag_iso_terminal_designer::object_properties::PropertyKey::Value,
            reference: Some(ObjectTarget::id(pointer)),
        }],
    ));
    assert!(!transact(&mut project, vec![allowed(pointer, &[40])]));
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
