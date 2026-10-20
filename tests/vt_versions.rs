use ag_iso_stack::object_pool::{
    object::{Object, StringVariable, VtString},
    object_attributes::FontType,
    vt_version::VtVersion,
    ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::{
    default_object,
    object_properties::{get_property, get_property_descriptors, validate_property, PropertyKey},
    vt_compatibility::supported_font_type,
    EditorProject, ObjectTarget, Operation, OperationTransaction,
};
use serde_json::{json, Value};

fn object_id(value: u16) -> ObjectId {
    ObjectId::new(value).unwrap()
}

#[test]
fn font_type_choices_authoring_and_export_follow_version_boundaries() {
    let cases: Vec<_> = [(0, 2), (1, 2), (2, 4), (4, 4), (5, 4), (7, 4)]
        .into_iter()
        .chain((240..=254).map(|code| (code, 4)))
        .chain([(255, 2)])
        .map(|(code, since)| (FontType::from(code), since))
        .collect();
    let pool = with_object(ObjectType::FontAttributes, 10);
    let choices = get_property_descriptors(ObjectType::FontAttributes)
        .into_iter()
        .find(|d| d.name == PropertyKey::FontType)
        .unwrap()
        .enum_choices;
    let expected: Vec<_> = cases
        .iter()
        .map(|(font, _)| serde_json::to_value(font).unwrap())
        .collect();
    assert_eq!(choices, expected);
    for (font, since) in cases {
        let value = serde_json::to_value(font).unwrap();
        validate_property(&pool, object_id(10), PropertyKey::FontType, &value).unwrap();
        for number in 2..=6 {
            let version = VtVersion::try_from(number).unwrap();
            let available = number >= since;
            assert_eq!(
                supported_font_type(font, version),
                available,
                "{font:?} at VT{number}"
            );
            let mut project = EditorProject::from(pool.clone());
            project.set_vt_version(version).unwrap();
            let mut edit = OperationTransaction::new(None);
            edit.add_operation(set_property(10, "font_type", value.clone()));
            let applied = project.execute_transaction(edit);
            assert_eq!(applied.is_ok(), available, "{font:?} at VT{number}");
            if available {
                // Valid edits retain serializable inverse transactions.
                serde_json::to_value(applied.unwrap().create_inverse()).unwrap();
                let imported =
                    EditorProject::import_iop(project.export_iop().unwrap(), version).unwrap();
                assert_eq!(
                    get_property(imported.get_pool(), object_id(10), PropertyKey::FontType)
                        .unwrap(),
                    value
                );
            }
        }
    }
}

#[test]
fn signed_numeric_offsets_survive_project_and_iop_round_trips() {
    let mut pool = ObjectPool::default();
    for (kind, id) in [
        (ObjectType::FontAttributes, 1),
        (ObjectType::InputNumber, 2),
        (ObjectType::OutputNumber, 3),
    ] {
        let mut object = default_object(kind);
        object.mut_id().set_value(id).unwrap();
        match &mut object {
            Object::InputNumber(number) => {
                number.font_attributes = object_id(1).into();
                number.max_value = 1000;
            }
            Object::OutputNumber(number) => number.font_attributes = object_id(1).into(),
            _ => {}
        }
        pool.add(object);
    }
    add_pool_scaffolding(&mut pool);
    let mut project = EditorProject::from(pool);
    let mut transaction = OperationTransaction::new(None);
    for id in [2, 3] {
        for (property, value) in [
            ("offset", json!(-100)),
            ("value", json!(250)),
            ("scale", json!(0.5)),
        ] {
            transaction.add_operation(Operation::SetProperty {
                object: ObjectTarget::id(id),
                property: property.parse().unwrap(),
                value,
            });
        }
    }
    project.execute_transaction(transaction).unwrap();
    let saved = EditorProject::load_project(project.save_project().unwrap()).unwrap();
    let imported =
        EditorProject::import_iop(project.export_iop().unwrap(), VtVersion::Version6).unwrap();
    for candidate in [&saved, &imported] {
        for id in [2, 3] {
            let object = candidate.get_pool().object_by_id(object_id(id)).unwrap();
            let (value, offset, scale) = match object {
                Object::InputNumber(number) => (number.value, number.offset, number.scale),
                Object::OutputNumber(number) => (number.value, number.offset, number.scale),
                _ => unreachable!(),
            };
            assert_eq!((value, offset, scale), (250, -100, 0.5));
            assert_eq!(
                (f64::from(value) + f64::from(offset)) * f64::from(scale),
                75.0
            );
        }
    }
}

#[test]
fn versioned_records_match_specification_bytes_and_round_trip() {
    use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef};
    let mut boolean = default_object(ObjectType::InputBoolean);
    let Object::InputBoolean(value) = &mut boolean else {
        unreachable!()
    };
    value.id = object_id(1);
    value.background_colour = 160;
    value.width = 0x1234;
    value.foreground_colour = object_id(2).into();
    value.value = 200;
    let bytes = boolean.try_write_for_version(VtVersion::Version4).unwrap();
    assert_eq!(bytes, [1, 0, 7, 160, 0x34, 0x12, 2, 0, 255, 255, 200, 1, 0]);
    assert_eq!(
        Object::read_for_version(&mut bytes.into_iter(), VtVersion::Version4).unwrap(),
        boolean
    );

    let mut container = default_object(ObjectType::Container);
    let Object::Container(value) = &mut container else {
        unreachable!()
    };
    value.id = object_id(3);
    value.macro_refs.push(MacroRef {
        macro_id: 7000,
        event_id: Event::OnHide,
    });
    let bytes = container
        .try_write_for_version(VtVersion::Version5)
        .unwrap();
    assert_eq!(&bytes[9..], [2, 255, 0x58, 4, 0x1B]);
    assert_eq!(
        Object::read_for_version(&mut bytes.into_iter(), VtVersion::Version5).unwrap(),
        container
    );
    assert!(container
        .try_write_for_version(VtVersion::Version4)
        .is_err());
    if let Object::Container(value) = &mut container {
        value.macro_refs = vec![
            MacroRef {
                macro_id: 7000,
                event_id: Event::OnHide
            };
            128
        ];
    }
    assert!(container
        .try_write_for_version(VtVersion::Version5)
        .is_err());
    if let Object::Container(value) = &mut container {
        value.macro_refs.clear();
        value.object_refs = vec![
            ag_iso_stack::object_pool::ObjectRef {
                id: object_id(2),
                offset: Default::default(),
            };
            256
        ];
    }
    assert!(container
        .try_write_for_version(VtVersion::Version6)
        .is_err());

    let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
    controls.mut_id().set_value(4).unwrap();
    let bytes = controls.try_write_for_version(VtVersion::Version6).unwrap();
    assert_eq!(bytes, [4, 0, 47, 5, 0, 255, 255, 255, 255, 0]);
    assert_eq!(
        Object::read_for_version(&mut bytes.into_iter(), VtVersion::Version6).unwrap(),
        controls
    );
}

// Export tests need a complete Working Set even when the record under test
// is otherwise independent of the mask tree.
fn add_pool_scaffolding(pool: &mut ObjectPool) {
    for (kind, id) in [
        (ObjectType::WorkingSet, 65000),
        (ObjectType::DataMask, 65001),
        (ObjectType::PictureGraphic, 65002),
    ] {
        let mut object = default_object(kind);
        *object.mut_id() = object_id(id);
        if let Object::WorkingSet(ws) = &mut object {
            ws.active_mask = object_id(65001);
            ws.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
                id: object_id(65002),
                offset: Default::default(),
            });
        }
        if let Object::PictureGraphic(picture) = &mut object {
            picture.width = 8;
            picture.actual_width = 8;
            picture.actual_height = 1;
            picture.data = vec![0];
        }
        pool.add(object);
    }
}

fn with_object(object_type: ObjectType, id: u16) -> ObjectPool {
    let mut pool = ObjectPool::default();
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    pool.add(object);
    add_pool_scaffolding(&mut pool);
    pool
}

fn set_property(object: u16, property: &str, value: Value) -> Operation {
    Operation::SetProperty {
        object: ObjectTarget::id(object),
        property: property.parse().unwrap(),
        value,
    }
}

#[test]
fn unavailable_object_blocks_version_and_dimension_change_atomically() {
    let mut project = EditorProject::from(with_object(ObjectType::OutputList, 10));
    project.mark_saved();
    let original_mask_size = project.mask_size;
    let before = project.export_iop().unwrap();
    let result = project.set_project_settings(400, (50, 50), VtVersion::Version2);

    assert!(result.is_err());
    assert_eq!(project.vt_version(), VtVersion::Version6);
    assert_eq!(project.mask_size, original_mask_size);
    assert_eq!(project.soft_key_size(), (60, 60));
    assert_eq!(project.export_iop().unwrap(), before);
    assert!(!project.is_dirty());
    assert!(!project.undo_operation());
}

#[test]
fn v1_encoded_string_bytes_and_v2_repair_drafts_round_trip() {
    let mut pool = ObjectPool::default();
    pool.add(Object::StringVariable(StringVariable {
        id: object_id(10),
        value: VtString::Encoded {
            encoded_bytes: vec![0x41, 0xE9],
        },
    }));
    add_pool_scaffolding(&mut pool);
    let legacy_iop = pool.as_iop();
    let mut legacy: Value =
        serde_json::from_slice(&EditorProject::from(pool).save_project().unwrap()).unwrap();
    legacy["version"] = json!(1);
    legacy["settings"]["vt_version"] = json!(2);
    legacy["object_pool_data"] = json!(legacy_iop);
    legacy.as_object_mut().unwrap().remove("objects");
    let loaded = EditorProject::load_project(serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(loaded.vt_version(), VtVersion::Version2);
    assert_eq!(loaded.export_iop().unwrap(), legacy_iop);

    // Historical v1 saves omitted the extensible-record length on this object.
    let controls = with_object(ObjectType::WorkingSetSpecialControls, 11);
    let mut legacy: Value =
        serde_json::from_slice(&EditorProject::from(controls).save_project().unwrap()).unwrap();
    legacy["version"] = json!(1);
    legacy["object_pool_data"] = json!([11, 0, 47, 255, 255, 255, 255, 0]);
    legacy.as_object_mut().unwrap().remove("objects");
    let loaded = EditorProject::load_project(serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(
        loaded.get_pool().as_iop(),
        [11, 0, 47, 5, 0, 255, 255, 255, 255, 0]
    );
}

#[test]
fn raw_iop_rejects_truncated_records_and_unknown_object_types() {
    assert!(EditorProject::import_iop(vec![37, 1, 0], VtVersion::Version6).is_err());
    assert!(EditorProject::import_iop(vec![0xFE], VtVersion::Version6).is_err());
}
