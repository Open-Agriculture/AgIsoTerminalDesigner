use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::object_attributes::ObjectLabel;
use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::{NullableObjectId, ObjectId, ObjectPool, ObjectType};
use ag_iso_terminal_designer::{
    default_object, EditorProject, Operation, OperationError, OperationTransaction,
};

fn object_with_id(object_type: ObjectType, id: u16) -> Object {
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    object
}

fn vt4_project() -> EditorProject {
    let mut pool = ObjectPool::default();
    for (object_type, id) in [
        (ObjectType::ObjectLabelReferenceList, 1),
        (ObjectType::Button, 2),
        (ObjectType::StringVariable, 3),
        (ObjectType::PictureGraphic, 4),
        (ObjectType::Container, 5),
    ] {
        pool.add(object_with_id(object_type, id));
    }
    let mut project = EditorProject::from(pool);
    project.set_vt_version(VtVersion::Version4).unwrap();
    project
}

fn set_labels(project: &mut EditorProject, labels: Vec<ObjectLabel>) -> Result<(), OperationError> {
    let mut transaction = OperationTransaction::new(None);
    transaction.add_operation(Operation::SetObjectLabels {
        object_id: 1,
        labels,
    });
    project.execute_transaction(transaction).map(|_| ())
}

fn label(id: u16, text: Option<u16>, graphic: Option<u16>) -> ObjectLabel {
    let nullable = |id: Option<u16>| NullableObjectId(id.map(|id| ObjectId::new(id).unwrap()));
    ObjectLabel {
        id: ObjectId::new(id).unwrap(),
        string_variable_reference: nullable(text),
        font_type: 0,
        graphic_representation: nullable(graphic),
    }
}

fn labels(project: &EditorProject) -> Vec<ObjectLabel> {
    match project.get_pool().object_by_id(ObjectId::new(1).unwrap()) {
        Some(Object::ObjectLabelReferenceList(list)) => list.object_labels.clone(),
        _ => unreachable!(),
    }
}

#[test]
fn a_vt4_label_takes_its_text_from_a_string_variable_and_its_graphic_from_a_picture() {
    let mut project = vt4_project();
    let button_label = label(2, Some(3), Some(4));

    set_labels(&mut project, vec![button_label.clone()]).unwrap();

    assert_eq!(labels(&project), vec![button_label]);
}

#[test]
fn label_text_and_graphic_must_have_their_own_types() {
    let mut project = vt4_project();

    assert!(set_labels(&mut project, vec![label(2, Some(4), None)]).is_err());
    assert!(set_labels(&mut project, vec![label(2, None, Some(3))]).is_err());
    assert!(set_labels(&mut project, vec![label(3, None, None)]).is_err());
    assert!(labels(&project).is_empty());
}

#[test]
fn a_container_can_be_labelled_and_be_the_graphic_of_another_label() {
    let mut project = vt4_project();

    set_labels(
        &mut project,
        vec![label(5, Some(3), None), label(2, None, Some(5))],
    )
    .unwrap();

    assert_eq!(labels(&project).len(), 2);
}
