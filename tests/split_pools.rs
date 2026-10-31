use ag_iso_stack::object_pool::{object::Object, vt_version::VtVersion, ObjectPool, ObjectType};
use ag_iso_terminal_designer::{default_object, EditorProject, ObjectIdFormat};

fn object_with_id(object_type: ObjectType, id: u16) -> Object {
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    object
}

fn pool_bytes(objects: &[(ObjectType, u16)]) -> Vec<u8> {
    let mut pool = ObjectPool::default();
    for (object_type, id) in objects {
        pool.add(object_with_id(*object_type, *id));
    }
    pool.as_iop()
}

fn import(parts: Vec<(&str, Vec<u8>)>) -> Result<EditorProject, String> {
    EditorProject::import_iop_parts_with_id_format(
        parts
            .into_iter()
            .map(|(name, bytes)| (name.to_owned(), bytes))
            .collect(),
        VtVersion::Version4,
        ObjectIdFormat::Hexadecimal,
    )
}

fn ids(project: &EditorProject) -> Vec<u16> {
    project
        .get_pool()
        .objects()
        .iter()
        .map(|object| object.id().value())
        .collect()
}

#[test]
fn parts_with_whole_objects_load_as_one_pool() {
    let first = pool_bytes(&[
        (ObjectType::NumberVariable, 1),
        (ObjectType::NumberVariable, 2),
    ]);
    let second = pool_bytes(&[(ObjectType::NumberVariable, 3)]);

    let project = import(vec![("a.iop", first), ("b.iop", second)]).unwrap();

    assert_eq!(ids(&project), vec![1, 2, 3]);
}

#[test]
fn a_part_may_end_inside_an_object() {
    let whole = pool_bytes(&[
        (ObjectType::NumberVariable, 1),
        (ObjectType::NumberVariable, 2),
        (ObjectType::NumberVariable, 3),
    ]);
    let (first, rest) = whole.split_at(5);
    let (second, third) = rest.split_at(7);

    let project = import(vec![
        ("1.iop", first.to_vec()),
        ("2.iop", second.to_vec()),
        ("3.iop", third.to_vec()),
    ])
    .unwrap();

    assert_eq!(ids(&project), vec![1, 2, 3]);
}

#[test]
fn an_object_a_later_part_defines_again_replaces_the_earlier_one() {
    let first = pool_bytes(&[
        (ObjectType::NumberVariable, 0x10),
        (ObjectType::NumberVariable, 1),
    ]);
    let mut replacement = object_with_id(ObjectType::NumberVariable, 0x10);
    if let Object::NumberVariable(variable) = &mut replacement {
        variable.value = 7;
    }
    let mut pool = ObjectPool::default();
    pool.add(replacement);

    let project = import(vec![("a.iop", first), ("b.iop", pool.as_iop())]).unwrap();

    assert_eq!(ids(&project), vec![1, 0x10]);
    let Some(Object::NumberVariable(variable)) = project
        .get_pool()
        .object_by_id(ag_iso_stack::object_pool::ObjectId::new(0x10).unwrap())
    else {
        panic!("expected the Number Variable");
    };
    assert_eq!(variable.value, 7);
}

#[test]
fn errors_name_the_part_and_its_byte() {
    let first = pool_bytes(&[(ObjectType::NumberVariable, 1)]);
    let truncated = pool_bytes(&[(ObjectType::NumberVariable, 2)])[..4].to_vec();

    let error = import(vec![("a.iop", first), ("b.iop", truncated)])
        .err()
        .unwrap();

    assert!(error.contains("b.iop"), "{error}");
}
