use ag_iso_stack::object_pool::{
    object::{Object, StringVariable, VtString},
    vt_version::VtVersion,
    ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::{
    default_object, EditorProject, ObjectTarget, Operation, OperationTransaction,
    CURRENT_OPERATION_SCHEMA_VERSION,
};
use serde_json::{json, Value};

fn object_id(value: u16) -> ObjectId {
    ObjectId::new(value).unwrap()
}

#[test]
fn availability_labels_include_introductions_and_vt2_only_objects() {
    use ag_iso_terminal_designer::vt_compatibility::{
        availability_description, supported_object_type,
    };
    for (object_type, expected) in [
        (ObjectType::OutputList, "Available from VT4"),
        (ObjectType::Animation, "Available from VT5"),
        (ObjectType::ColourPalette, "Available from VT6"),
        (ObjectType::AuxiliaryFunctionType1, "Available only in VT2"),
    ] {
        assert_eq!(
            availability_description(|version| supported_object_type(object_type, version))
                .as_deref(),
            Some(expected)
        );
    }
    assert_eq!(availability_description(|_| false), None);
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

fn with_object(object_type: ObjectType, id: u16) -> ObjectPool {
    let mut pool = ObjectPool::default();
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    pool.add(object);
    pool
}

fn set_property(object: u16, property: &str, value: Value) -> Operation {
    Operation::SetProperty {
        object: ObjectTarget::id(object),
        property: property.parse().unwrap(),
        value,
    }
}

fn apply(project: &mut EditorProject, operations: Vec<Operation>) -> bool {
    project
        .execute_transaction(OperationTransaction {
            schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some("VT version regression".into()),
            operations,
        })
        .is_ok()
}

#[test]
fn version_settings_are_dirty_noop_and_undoable() {
    let mut project = EditorProject::default();
    project.mark_saved();

    for version in [
        VtVersion::Version2,
        VtVersion::Version3,
        VtVersion::Version4,
        VtVersion::Version5,
        VtVersion::Version6,
    ] {
        project.set_vt_version(version).unwrap();
        assert_eq!(project.vt_version(), version);
    }
    assert!(!project.is_dirty());

    project.set_vt_version(VtVersion::Version5).unwrap();
    assert!(project.is_dirty());
    project.set_vt_version(VtVersion::Version5).unwrap();
    assert!(project.is_dirty());
    assert!(project.undo_operation());
    assert_eq!(project.vt_version(), VtVersion::Version6);
    assert!(!project.is_dirty());
    assert!(project.redo_operation());
    assert_eq!(project.vt_version(), VtVersion::Version5);
    assert!(project.is_dirty());
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
fn incompatible_flag_repair_allows_other_edits_and_survives_v2_save() {
    let mut project = EditorProject::from(with_object(ObjectType::Button, 10));
    let mut saved: Value = serde_json::from_slice(&project.save_project().unwrap()).unwrap();
    saved["settings"]["vt_version"] = json!(2);
    let mut button_options = saved["objects"][0]["properties"]["options"].clone();
    button_options["suppress_border"] = json!(true);
    saved["objects"][0]["properties"]["options"] = button_options;
    project = EditorProject::load_project(serde_json::to_vec(&saved).unwrap()).unwrap();

    assert!(!project.compatibility_diagnostics().unwrap().is_empty());
    assert!(apply(
        &mut project,
        vec![set_property(10, "width", json!(20))]
    ));
    let options = project
        .get_pool()
        .object_by_id(object_id(10))
        .and_then(|object| match object {
            Object::Button(button) => Some(serde_json::to_value(button.options).unwrap()),
            _ => None,
        })
        .unwrap();
    let mut repaired_options = options;
    repaired_options["suppress_border"] = json!(false);
    assert!(apply(
        &mut project,
        vec![set_property(10, "options", repaired_options)]
    ));
    assert!(project.compatibility_diagnostics().unwrap().is_empty());

    // A repair draft with an unsupported object remains lossless in .aitp v2.
    let mut unsupported = EditorProject::from(with_object(ObjectType::OutputList, 30));
    let mut saved: Value = serde_json::from_slice(&unsupported.save_project().unwrap()).unwrap();
    saved["settings"]["vt_version"] = json!(2);
    unsupported = EditorProject::load_project(serde_json::to_vec(&saved).unwrap()).unwrap();
    let round_trip = EditorProject::load_project(unsupported.save_project().unwrap()).unwrap();
    assert_eq!(round_trip.vt_version(), VtVersion::Version2);
    assert!(matches!(
        round_trip.get_pool().object_by_id(object_id(30)),
        Some(Object::OutputList(_))
    ));
    assert!(round_trip.export_iop().is_err());

    let mut fonts = with_object(ObjectType::FontAttributes, 40);
    let Some(Object::FontAttributes(font)) = fonts.object_mut_by_id(object_id(40)) else {
        unreachable!()
    };
    let fixed_size = serde_json::to_value(font.font_size).unwrap();
    font.font_size = ag_iso_stack::object_pool::object_attributes::FontSize::Proportional(16);
    font.font_style.proportional = true;
    let mut saved: Value =
        serde_json::from_slice(&EditorProject::from(fonts).save_project().unwrap()).unwrap();
    saved["settings"]["vt_version"] = json!(2);
    let mut project = EditorProject::load_project(serde_json::to_vec(&saved).unwrap()).unwrap();
    assert!(project.export_iop().is_err());
    assert!(apply(
        &mut project,
        vec![set_property(40, "font_size", fixed_size)]
    ));
    assert!(project.compatibility_diagnostics().unwrap().is_empty());
    assert_eq!(project.export_iop().unwrap()[6] & 0x80, 0);
}

#[test]
fn legacy_strings_accept_ascii_and_reject_unrepresentable_edits() {
    let mut pool = ObjectPool::default();
    pool.add(Object::StringVariable(StringVariable {
        id: object_id(10),
        value: VtString::Text("A".repeat(200)),
    }));
    let project = EditorProject::from(pool);
    let mut saved: Value = serde_json::from_slice(&project.save_project().unwrap()).unwrap();
    saved["settings"]["vt_version"] = json!(2);
    let mut project = EditorProject::load_project(serde_json::to_vec(&saved).unwrap()).unwrap();
    assert!(project.export_iop().is_ok());

    assert!(!apply(
        &mut project,
        vec![set_property(10, "value", json!("😀"))]
    ));
    assert!(project.export_iop().is_ok());
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
        loaded.export_iop().unwrap(),
        [11, 0, 47, 5, 0, 255, 255, 255, 255, 0]
    );
}

#[test]
fn raw_iop_rejects_truncated_records_and_unknown_object_types() {
    assert!(EditorProject::import_iop(vec![37, 1, 0], VtVersion::Version6).is_err());
    assert!(EditorProject::import_iop(vec![0xFE], VtVersion::Version6).is_err());
}

#[test]
fn configuration_ui_hides_button_options_until_the_specified_version() {
    // Table B.14 introduces Suppress Border at VT4. Exercise the actual
    // property editor checkbox, rather than only its compatibility helper.
    for number in 2..=6 {
        let mut project = EditorProject::from(with_object(ObjectType::Button, 10));
        project
            .set_vt_version(VtVersion::try_from(number).unwrap())
            .unwrap();
        // Annex A.1 introduction boundaries, exercised through authoring.
        for (object_type, first_version) in [
            (ObjectType::AuxiliaryFunctionType2, 3),
            (ObjectType::AuxiliaryControlDesignatorType2, 3),
            (ObjectType::WindowMask, 4),
            (ObjectType::GraphicsContext, 4),
            (ObjectType::OutputList, 4),
            (ObjectType::Animation, 5),
            (ObjectType::ExternalObjectPointer, 5),
            (ObjectType::ColourPalette, 6),
            (ObjectType::ScaledGraphic, 6),
        ] {
            let mut sample = project.clone();
            assert_eq!(
                sample
                    .create_object(object_type, "Version boundary".into())
                    .is_some(),
                number >= first_version,
                "{object_type:?} at VT{number}"
            );
        }
        let mut sample = project.clone();
        assert_eq!(
            sample
                .create_object(
                    ObjectType::AuxiliaryFunctionType1,
                    "Legacy auxiliary".into()
                )
                .is_some(),
            number == 2,
            "Type 1 auxiliary at VT{number}"
        );

        let mut sample = project.clone();
        let font = sample
            .create_object(ObjectType::FontAttributes, "Font".into())
            .unwrap();
        assert_eq!(
            apply(
                &mut sample,
                vec![set_property(font.value(), "font_type", json!("Greek"))]
            ),
            number >= 4,
            "Greek encoding at VT{number}"
        );
        assert_eq!(
            apply(
                &mut sample,
                vec![set_property(
                    font.value(),
                    "font_size",
                    json!({"Proportional": 16})
                )]
            ),
            number >= 4,
            "Proportional font at VT{number}"
        );
        let boolean = sample
            .create_object(ObjectType::InputBoolean, "Boolean".into())
            .unwrap();
        assert_eq!(
            apply(
                &mut sample,
                vec![set_property(boolean.value(), "value", json!(2))]
            ),
            number >= 4,
            "Boolean byte value at VT{number}"
        );
        let context = egui::Context::default();
        let mut position = None;
        for pressed in [None, Some(true), Some(false)] {
            let events = pressed.map_or_else(Vec::new, |pressed| {
                let position = position.unwrap();
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            });
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 1600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        ag_iso_terminal_designer::render_property_editor(
                            ui,
                            project.get_pool().object_by_id(object_id(10)).unwrap(),
                            &project,
                        );
                    });
                },
            );
            if pressed.is_none() {
                position = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.text().eq_ignore_ascii_case("Suppress Border") =>
                    {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                });
                assert_eq!(
                    position.is_some(),
                    number >= 4,
                    "checkbox visibility for VT{number}"
                );
                if number < 4 {
                    break;
                }
            }
        }
        assert_eq!(project.update_pool(), number >= 4, "VT{number}");
        let Some(Object::Button(button)) = project.get_pool().object_by_id(object_id(10)) else {
            unreachable!()
        };
        assert_eq!(button.options.suppress_border, number >= 4, "VT{number}");
    }
}

#[test]
fn hierarchy_checks_allow_key_labels_and_follow_container_and_pointer_chains() {
    use ag_iso_stack::object_pool::object_attributes::{ObjectRef, Point};
    let reference = |id| ObjectRef {
        id: object_id(id),
        offset: Point::default(),
    };
    let mut pool = with_object(ObjectType::SoftKeyMask, 10);
    if let Some(Object::SoftKeyMask(mask)) = pool.object_mut_by_id(object_id(10)) {
        mask.objects.push(object_id(11));
    }
    let mut key = default_object(ObjectType::Key);
    key.mut_id().set_value(11).unwrap();
    if let Object::Key(key) = &mut key {
        key.object_refs.push(reference(12));
    }
    pool.add(key);
    let mut label = default_object(ObjectType::OutputString);
    label.mut_id().set_value(12).unwrap();
    pool.add(label);
    let mut project = EditorProject::from(pool);
    project.set_vt_version(VtVersion::Version2).unwrap();
    assert!(project.compatibility_diagnostics().unwrap().is_empty());

    let mut pool = with_object(ObjectType::Button, 20);
    if let Some(Object::Button(button)) = pool.object_mut_by_id(object_id(20)) {
        button.object_refs.push(reference(21));
    }
    let mut container = default_object(ObjectType::Container);
    container.mut_id().set_value(21).unwrap();
    if let Object::Container(container) = &mut container {
        container.object_refs.push(reference(22));
    }
    pool.add(container);
    let mut input = default_object(ObjectType::InputNumber);
    input.mut_id().set_value(22).unwrap();
    pool.add(input);
    let project = EditorProject::from(pool.clone());
    assert!(project
        .compatibility_diagnostics()
        .unwrap()
        .iter()
        .any(|d| d.code == "VT_INVALID_CHILD_TYPE"));

    if let Some(Object::Container(container)) = pool.object_mut_by_id(object_id(21)) {
        container.object_refs = vec![reference(23)];
    }
    for (id, target) in [(23, 24), (24, 22)] {
        let mut pointer = default_object(ObjectType::ObjectPointer);
        pointer.mut_id().set_value(id).unwrap();
        if let Object::ObjectPointer(pointer) = &mut pointer {
            pointer.value = object_id(target).into();
        }
        pool.add(pointer);
    }
    let project = EditorProject::from(pool);
    assert!(project
        .compatibility_diagnostics()
        .unwrap()
        .iter()
        .any(|d| d.code == "VT_INVALID_POINTER_TARGET_TYPE"));
    assert!(project.export_iop().is_err());
}
