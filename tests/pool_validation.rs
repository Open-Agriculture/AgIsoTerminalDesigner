use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectId};
use ag_iso_terminal_designer::default_object;
use ag_iso_terminal_designer::pool_validation::{
    validate_complete_pool_state_for_version, validate_pool_state_for_version,
};
use std::collections::HashSet;

#[test]
fn complete_validation_checks_pool_scaffolding_and_typed_references() {
    use ag_iso_stack::object_pool::{ObjectPool, ObjectType};
    let mut pool = ObjectPool::new();
    let mut ws = default_object(ObjectType::WorkingSet);
    *ws.mut_id() = ObjectId::new(1).unwrap();
    let mut ws2 = default_object(ObjectType::WorkingSet);
    *ws2.mut_id() = ObjectId::new(2).unwrap();
    pool.add(ws);
    pool.add(ws2);
    let mut rect = default_object(ObjectType::OutputRectangle);
    *rect.mut_id() = ObjectId::new(3).unwrap();
    let wrong_type_target = default_object(ObjectType::OutputString);
    let mut wrong_type_target = wrong_type_target;
    *wrong_type_target.mut_id() = ObjectId::new(4).unwrap();
    if let ag_iso_stack::object_pool::object::Object::OutputRectangle(rect) = &mut rect {
        rect.line_attributes = ObjectId::new(4).unwrap().into();
    }
    pool.add(rect);
    pool.add(wrong_type_target);
    for id in [5, 6] {
        let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
        *controls.mut_id() = ObjectId::new(id).unwrap();
        pool.add(controls);
    }

    let mut polygon = default_object(ObjectType::OutputPolygon);
    *polygon.mut_id() = ObjectId::new(7).unwrap();
    if let ag_iso_stack::object_pool::object::Object::OutputPolygon(polygon) = &mut polygon {
        polygon.polygon_type = 3;
        polygon.points = vec![Default::default(); 2];
    }
    pool.add(polygon);
    let mut colour_map = default_object(ObjectType::ColourMap);
    *colour_map.mut_id() = ObjectId::new(8).unwrap();
    if let ag_iso_stack::object_pool::object::Object::ColourMap(map) = &mut colour_map {
        map.colour_map = vec![0; 3];
    }
    pool.add(colour_map);
    let mut palette = default_object(ObjectType::ColourPalette);
    *palette.mut_id() = ObjectId::new(9).unwrap();
    if let ag_iso_stack::object_pool::object::Object::ColourPalette(palette) = &mut palette {
        palette.colours = vec![Default::default(); 257];
    }
    pool.add(palette);
    let mut key_group = default_object(ObjectType::KeyGroup);
    *key_group.mut_id() = ObjectId::new(10).unwrap();
    if let ag_iso_stack::object_pool::object::Object::KeyGroup(group) = &mut key_group {
        group.objects = (20..25).map(|id| ObjectId::new(id).unwrap()).collect();
    }
    pool.add(key_group);
    for id in 20..25 {
        let mut key = default_object(ObjectType::Key);
        *key.mut_id() = ObjectId::new(id).unwrap();
        pool.add(key);
    }
    let mut input_number = default_object(ObjectType::InputNumber);
    *input_number.mut_id() = ObjectId::new(12).unwrap();
    if let ag_iso_stack::object_pool::object::Object::InputNumber(number) = &mut input_number {
        number.nr_of_decimals = 8;
    }
    pool.add(input_number);
    let mut number = default_object(ObjectType::OutputNumber);
    *number.mut_id() = ObjectId::new(11).unwrap();
    if let ag_iso_stack::object_pool::object::Object::OutputNumber(number) = &mut number {
        number.nr_of_decimals = 8;
    }
    pool.add(number);

    let incremental = validate_pool_state_for_version(&pool, VtVersion::Version6);
    assert!(!incremental.iter().any(|d| {
        matches!(
            d.code.as_str(),
            "WORKING_SET_CARDINALITY"
                | "WORKING_SET_IDENTIFIER_MISSING"
                | "WORKING_SET_SPECIAL_CONTROLS_CARDINALITY"
                | "PROPERTY_REFERENCE_WRONG_TYPE"
        )
    }));
    let complete = validate_complete_pool_state_for_version(&pool, VtVersion::Version6);
    let codes: HashSet<_> = complete.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains("WORKING_SET_CARDINALITY"));
    assert!(codes.contains("WORKING_SET_IDENTIFIER_MISSING"));
    assert!(codes.contains("WORKING_SET_SPECIAL_CONTROLS_CARDINALITY"));
    assert!(codes.contains("PROPERTY_REFERENCE_WRONG_TYPE"));
    assert!(codes.contains("POLYGON_TOO_FEW_POINTS"));
    assert!(codes.contains("COLOUR_MAP_INVALID_SIZE"));
    assert!(codes.contains("COLOUR_PALETTE_TOO_LARGE"));
    assert!(codes.contains("KEY_GROUP_INVALID_SIZE"));
    assert!(codes.contains("PROPERTY_OUT_OF_RANGE"));

    let has_code = |pool: &ObjectPool, code: &str| {
        validate_complete_pool_state_for_version(pool, VtVersion::Version6)
            .iter()
            .any(|diagnostic| diagnostic.code == code)
    };
    for size in [2, 16, 256] {
        if let Some(ag_iso_stack::object_pool::object::Object::ColourMap(map)) =
            pool.object_mut_by_id(ObjectId::new(8).unwrap())
        {
            map.colour_map = vec![0; size];
        }
        assert!(!has_code(&pool, "COLOUR_MAP_INVALID_SIZE"));
    }
    for size in [0, 256] {
        if let Some(ag_iso_stack::object_pool::object::Object::ColourPalette(palette)) =
            pool.object_mut_by_id(ObjectId::new(9).unwrap())
        {
            palette.colours = vec![Default::default(); size];
        }
        assert!(!has_code(&pool, "COLOUR_PALETTE_TOO_LARGE"));
    }
    if let Some(ag_iso_stack::object_pool::object::Object::KeyGroup(group)) =
        pool.object_mut_by_id(ObjectId::new(10).unwrap())
    {
        group.objects.clear();
    }
    assert!(has_code(&pool, "KEY_GROUP_INVALID_SIZE"));
    for count in [1, 4] {
        if let Some(ag_iso_stack::object_pool::object::Object::KeyGroup(group)) =
            pool.object_mut_by_id(ObjectId::new(10).unwrap())
        {
            group.objects = (20..20 + count)
                .map(|id| ObjectId::new(id).unwrap())
                .collect();
        }
        assert!(!has_code(&pool, "KEY_GROUP_INVALID_SIZE"));
    }
    if let Some(ag_iso_stack::object_pool::object::Object::OutputPolygon(polygon)) =
        pool.object_mut_by_id(ObjectId::new(7).unwrap())
    {
        polygon.polygon_type = 3;
        polygon.points = vec![Default::default(); 2];
    }
    assert!(has_code(&pool, "POLYGON_TOO_FEW_POINTS"));
    if let Some(ag_iso_stack::object_pool::object::Object::OutputPolygon(polygon)) =
        pool.object_mut_by_id(ObjectId::new(7).unwrap())
    {
        polygon.points = vec![Default::default(); 3];
    }
    assert!(!has_code(&pool, "POLYGON_TOO_FEW_POINTS"));
}

#[test]
fn object_id_zero_is_valid_and_resolved_as_a_pool_reference() {
    use ag_iso_stack::object_pool::{object::Object, ObjectPool, ObjectType};
    let mut pool = ObjectPool::new();
    let mut target = default_object(ObjectType::NumberVariable);
    *target.mut_id() = ObjectId::new(0).unwrap();
    pool.add(target);
    let mut reference = default_object(ObjectType::OutputNumber);
    *reference.mut_id() = ObjectId::new(1).unwrap();
    if let Object::OutputNumber(number) = &mut reference {
        number.variable_reference = ObjectId::new(0).unwrap().into();
    }
    pool.add(reference);
    let diagnostics = validate_pool_state_for_version(&pool, VtVersion::Version6);
    assert!(!diagnostics
        .iter()
        .any(|d| d.code == "E007_OBJECT_NOT_FOUND"));

    let diagnostics = validate_complete_pool_state_for_version(&pool, VtVersion::Version6);
    assert!(!diagnostics
        .iter()
        .any(|d| d.code == "E007_OBJECT_NOT_FOUND"));

    let mut missing_pool = ObjectPool::new();
    let mut missing = default_object(ObjectType::OutputNumber);
    *missing.mut_id() = ObjectId::new(2).unwrap();
    if let Object::OutputNumber(number) = &mut missing {
        number.variable_reference = ObjectId::new(0).unwrap().into();
    }
    missing_pool.add(missing);
    let diagnostics = validate_pool_state_for_version(&missing_pool, VtVersion::Version6);
    assert!(!diagnostics
        .iter()
        .any(|d| d.code == "E007_OBJECT_NOT_FOUND"));
    let diagnostics = validate_complete_pool_state_for_version(&missing_pool, VtVersion::Version6);
    assert!(diagnostics
        .iter()
        .any(|d| { d.code == "E007_OBJECT_NOT_FOUND" && d.affected_objects == Some(vec![0]) }));
}

#[test]
fn iso_attribute_domains_are_checked_for_complete_pools() {
    use ag_iso_stack::object_pool::{
        object::{CharacterRange, CodePlane, Object},
        ObjectPool, ObjectType,
    };

    let mut pool = ObjectPool::new();
    let mut meter = default_object(ObjectType::OutputMeter);
    *meter.mut_id() = ObjectId::new(1).unwrap();
    if let Object::OutputMeter(meter) = &mut meter {
        meter.start_angle = 181;
        meter.variable_reference = ObjectId::new(2).unwrap().into();
    }
    pool.add(meter);

    let mut variable = default_object(ObjectType::NumberVariable);
    *variable.mut_id() = ObjectId::new(2).unwrap();
    if let Object::NumberVariable(variable) = &mut variable {
        variable.value = 65_536;
    }
    pool.add(variable);

    let mut extended = default_object(ObjectType::ExtendedInputAttributes);
    *extended.mut_id() = ObjectId::new(3).unwrap();
    if let Object::ExtendedInputAttributes(extended) = &mut extended {
        extended.code_planes = vec![CodePlane {
            number: 17,
            character_ranges: vec![CharacterRange {
                first_character: 2,
                last_character: 1,
            }],
        }];
    }
    pool.add(extended);
    let mut empty_extended = default_object(ObjectType::ExtendedInputAttributes);
    *empty_extended.mut_id() = ObjectId::new(8).unwrap();
    pool.add(empty_extended);
    let mut working_set = default_object(ObjectType::WorkingSet);
    *working_set.mut_id() = ObjectId::new(4).unwrap();
    if let Object::WorkingSet(working_set) = &mut working_set {
        working_set.language_codes = vec!["éx".into()];
    }
    pool.add(working_set);

    let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
    *controls.mut_id() = ObjectId::new(5).unwrap();
    if let Object::WorkingSetSpecialControls(controls) = &mut controls {
        controls.language_pairs = vec![("eng".into(), "US".into())];
    }
    pool.add(controls);

    let mut graphics = default_object(ObjectType::GraphicsContext);
    *graphics.mut_id() = ObjectId::new(6).unwrap();
    if let Object::GraphicsContext(graphics) = &mut graphics {
        graphics.canvas_width = 32_768;
    }
    pool.add(graphics);

    let diagnostics = validate_complete_pool_state_for_version(&pool, VtVersion::Version6);
    let affected = |code: &str, id: u16| {
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == code
                && diagnostic
                    .affected_objects
                    .as_ref()
                    .is_some_and(|objects| objects.contains(&id))
        })
    };
    assert!(affected("PROPERTY_OUT_OF_RANGE", 1)); // Meter angle.
    assert!(affected("ISO_PROPERTY_INVALID", 2)); // Referenced Number Variable.
    assert!(affected("ISO_PROPERTY_INVALID", 3)); // Code plane and character range.
    assert!(affected("ISO_PROPERTY_INVALID", 4)); // Working Set language code.
    assert!(affected("ISO_PROPERTY_INVALID", 5)); // Special Controls language pair.
    assert!(affected("PROPERTY_OUT_OF_RANGE", 6)); // Graphics Context dimensions.
    assert!(affected("ISO_PROPERTY_INVALID", 8)); // Empty authoring default blocks export.

    if let Some(Object::NumberVariable(variable)) = pool.object_mut_by_id(ObjectId::new(2).unwrap())
    {
        variable.value = 65_535;
    }
    assert!(
        !validate_complete_pool_state_for_version(&pool, VtVersion::Version6)
            .iter()
            .any(|diagnostic| diagnostic.code == "ISO_PROPERTY_INVALID"
                && diagnostic
                    .affected_objects
                    .as_ref()
                    .is_some_and(|ids| ids.contains(&2)))
    );
}

#[test]
fn iso_attribute_edits_validate_domains_and_allow_defined_boundaries() {
    use ag_iso_stack::object_pool::{object::Object, ObjectPool, ObjectType};
    use ag_iso_terminal_designer::object_properties::{validate_property, PropertyKey};
    use serde_json::json;

    let mut pool = ObjectPool::new();
    let mut variable = default_object(ObjectType::NumberVariable);
    *variable.mut_id() = ObjectId::new(1).unwrap();
    if let Object::NumberVariable(variable) = &mut variable {
        variable.value = 65_536;
    }
    pool.add(variable);
    let mut meter = default_object(ObjectType::OutputMeter);
    *meter.mut_id() = ObjectId::new(2).unwrap();
    if let Object::OutputMeter(meter) = &mut meter {
        meter.variable_reference = ObjectId::new(1).unwrap().into();
    }
    pool.add(meter);
    let mut bar = default_object(ObjectType::OutputLinearBarGraph);
    *bar.mut_id() = ObjectId::new(6).unwrap();
    pool.add(bar);
    let mut working_set = default_object(ObjectType::WorkingSet);
    *working_set.mut_id() = ObjectId::new(3).unwrap();
    pool.add(working_set);
    let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
    *controls.mut_id() = ObjectId::new(4).unwrap();
    pool.add(controls);
    let mut extended = default_object(ObjectType::ExtendedInputAttributes);
    *extended.mut_id() = ObjectId::new(5).unwrap();
    pool.add(extended);
    let mut graphics = default_object(ObjectType::GraphicsContext);
    *graphics.mut_id() = ObjectId::new(7).unwrap();
    pool.add(graphics);

    let reject = |id, property, value| {
        validate_property(&pool, ObjectId::new(id).unwrap(), property, &value).is_err()
    };
    assert!(reject(1, PropertyKey::Value, json!(65_537)));
    assert!(reject(
        2,
        PropertyKey::VariableReference,
        json!(ObjectId::new(1).unwrap())
    ));
    assert!(reject(
        6,
        PropertyKey::TargetValueVariableReference,
        json!(ObjectId::new(1).unwrap())
    ));
    assert!(reject(2, PropertyKey::StartAngle, json!(181)));
    assert!(reject(7, PropertyKey::ViewportWidth, json!(32768)));
    assert!(reject(7, PropertyKey::CanvasHeight, json!(32768)));
    assert!(reject(3, PropertyKey::LanguageCodes, json!(["en", "éx"])));
    assert!(reject(
        4,
        PropertyKey::LanguagePairs,
        json!([["eng", "US"]])
    ));
    assert!(reject(5, PropertyKey::CodePlanes, json!([])));
    assert!(reject(
        5,
        PropertyKey::CodePlanes,
        json!(vec![
            json!({"number":0,"character_ranges":[{"first_character":0,"last_character":0}]});
            18
        ])
    ));
    assert!(reject(
        5,
        PropertyKey::CodePlanes,
        json!([{"number":17,"character_ranges":[{"first_character":0,"last_character":0}]}])
    ));
    assert!(reject(
        5,
        PropertyKey::CodePlanes,
        json!([{"number":0,"character_ranges":[]}])
    ));
    assert!(reject(
        5,
        PropertyKey::CodePlanes,
        json!([{"number":0,"character_ranges":[{"first_character":8,"last_character":7}]}])
    ));

    assert!(validate_property(
        &pool,
        ObjectId::new(2).unwrap(),
        PropertyKey::EndAngle,
        &json!(180)
    )
    .is_ok());
    assert!(validate_property(
        &pool,
        ObjectId::new(7).unwrap(),
        PropertyKey::ViewportWidth,
        &json!(32767)
    )
    .is_ok());
    assert!(validate_property(
        &pool,
        ObjectId::new(7).unwrap(),
        PropertyKey::CanvasHeight,
        &json!(32767)
    )
    .is_ok());
    assert!(validate_property(
        &pool,
        ObjectId::new(6).unwrap(),
        PropertyKey::TargetValueVariableReference,
        &json!(null)
    )
    .is_ok());
    assert!(validate_property(
        &pool,
        ObjectId::new(3).unwrap(),
        PropertyKey::LanguageCodes,
        &json!(["en", "EN"])
    )
    .is_ok());
    assert!(validate_property(
        &pool,
        ObjectId::new(4).unwrap(),
        PropertyKey::LanguagePairs,
        &json!([["en", "  "]])
    )
    .is_ok());
    assert!(validate_property(&pool, ObjectId::new(5).unwrap(), PropertyKey::CodePlanes, &json!([{"number":16,"character_ranges":[{"first_character":65_535,"last_character":65_535}]}])).is_ok());
    if let Some(Object::NumberVariable(variable)) = pool.object_mut_by_id(ObjectId::new(1).unwrap())
    {
        variable.value = 65_535;
    }
    assert!(validate_property(
        &pool,
        ObjectId::new(1).unwrap(),
        PropertyKey::Value,
        &json!(65_535)
    )
    .is_ok());
    assert!(validate_property(
        &pool,
        ObjectId::new(6).unwrap(),
        PropertyKey::TargetValueVariableReference,
        &json!(ObjectId::new(1).unwrap())
    )
    .is_ok());
}

#[test]
fn complete_validation_rejects_malformed_graphics_and_macros() {
    use ag_iso_stack::object_pool::{
        object::Object,
        object_attributes::{DataCodeType, PictureGraphicFormat},
        ObjectPool, ObjectType,
    };
    let mut pool = ObjectPool::new();

    let mut text = default_object(ObjectType::OutputString);
    *text.mut_id() = ObjectId::new(1).unwrap();
    if let Object::OutputString(text) = &mut text {
        text.font_attributes = Default::default();
    }
    pool.add(text);

    let mut picture = default_object(ObjectType::PictureGraphic);
    *picture.mut_id() = ObjectId::new(2).unwrap();
    if let Object::PictureGraphic(picture) = &mut picture {
        picture.actual_width = 7;
        picture.actual_height = 1;
        picture.format = PictureGraphicFormat::Monochrome;
        picture.options.data_code_type = DataCodeType::Raw;
        picture.data = vec![];
    }
    pool.add(picture);
    let mut fill = default_object(ObjectType::FillAttributes);
    *fill.mut_id() = ObjectId::new(3).unwrap();
    if let Object::FillAttributes(fill) = &mut fill {
        fill.fill_type = 3;
        fill.fill_pattern = ObjectId::new(2).unwrap().into();
    }
    pool.add(fill);

    let mut empty_macro = default_object(ObjectType::Macro);
    *empty_macro.mut_id() = ObjectId::new(4).unwrap();
    pool.add(empty_macro);
    let mut recursive_macro = default_object(ObjectType::Macro);
    *recursive_macro.mut_id() = ObjectId::new(5).unwrap();
    if let Object::Macro(macro_object) = &mut recursive_macro {
        macro_object.commands = vec![0xBE, 5, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    pool.add(recursive_macro);
    let mut missing_target_macro = default_object(ObjectType::Macro);
    *missing_target_macro.mut_id() = ObjectId::new(7).unwrap();
    if let Object::Macro(macro_object) = &mut missing_target_macro {
        macro_object.commands = vec![0xBE, 90, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    pool.add(missing_target_macro);
    let mut wrong_target_macro = default_object(ObjectType::Macro);
    *wrong_target_macro.mut_id() = ObjectId::new(8).unwrap();
    if let Object::Macro(macro_object) = &mut wrong_target_macro {
        macro_object.commands = vec![0xB6, 2, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    pool.add(wrong_target_macro);

    let mut graphic_data = default_object(ObjectType::GraphicData);
    *graphic_data.mut_id() = ObjectId::new(6).unwrap();
    if let Object::GraphicData(data) = &mut graphic_data {
        data.data = b"not a png".to_vec();
    }
    pool.add(graphic_data);

    let diagnostics = validate_complete_pool_state_for_version(&pool, VtVersion::Version6);
    let codes: HashSet<_> = diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains("REQUIRED_REFERENCE_NULL"));
    assert!(codes.contains("PICTURE_DATA_TRUNCATED"));
    assert!(codes.contains("FILL_PATTERN_NOT_BYTE_ALIGNED"));
    assert!(codes.contains("MACRO_EMPTY"));
    assert!(codes.contains("MACRO_EXECUTION_CYCLE"));
    assert!(codes.contains("MACRO_TARGET_NOT_FOUND"));
    assert!(codes.contains("MACRO_TARGET_WRONG_TYPE"));
    assert!(codes.contains("GRAPHIC_DATA_PNG_INVALID"));
}

#[test]
fn picture_validation_accepts_excess_bytes_and_zero_rle_runs() {
    use ag_iso_stack::object_pool::{
        object::Object,
        object_attributes::{DataCodeType, PictureGraphicFormat},
        ObjectPool, ObjectType,
    };
    let mut pool = ObjectPool::new();
    let mut picture = default_object(ObjectType::PictureGraphic);
    *picture.mut_id() = ObjectId::new(1).unwrap();
    if let Object::PictureGraphic(picture) = &mut picture {
        picture.actual_width = 8;
        picture.actual_height = 1;
        picture.format = PictureGraphicFormat::Monochrome;
        picture.options.data_code_type = DataCodeType::RunLength;
        picture.data = vec![0, 0x7F, 1, 0x80];
    }
    pool.add(picture);

    assert!(
        !validate_complete_pool_state_for_version(&pool, VtVersion::Version6)
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("PICTURE_DATA_"))
    );

    if let Some(Object::PictureGraphic(picture)) = pool.object_mut_by_id(ObjectId::new(1).unwrap())
    {
        picture.options.data_code_type = DataCodeType::Raw;
        picture.data = vec![0; 3];
    }
    assert!(
        !validate_complete_pool_state_for_version(&pool, VtVersion::Version6)
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("PICTURE_DATA_"))
    );
}

#[test]
fn scaled_graphic_accepts_graphic_pointer_chains_and_rejects_invalid_endpoints() {
    use ag_iso_stack::object_pool::{object::Object, NullableObjectId, ObjectPool, ObjectType};
    let mut pool = ObjectPool::new();
    let mut graphic = default_object(ObjectType::ScaledGraphic);
    *graphic.mut_id() = ObjectId::new(1).unwrap();
    if let Object::ScaledGraphic(graphic) = &mut graphic {
        graphic.value = NullableObjectId::new(2);
    }
    let mut pointer = default_object(ObjectType::ObjectPointer);
    *pointer.mut_id() = ObjectId::new(2).unwrap();
    let mut picture = default_object(ObjectType::PictureGraphic);
    *picture.mut_id() = ObjectId::new(3).unwrap();
    let mut number = default_object(ObjectType::NumberVariable);
    *number.mut_id() = ObjectId::new(4).unwrap();
    pool.add(graphic);
    pool.add(pointer);
    pool.add(picture);
    pool.add(number);
    for (target, expected) in [
        (u16::MAX, None),
        (3, None),
        (4, Some("SCALED_GRAPHIC_TARGET_INVALID")),
        (2, Some("SCALED_GRAPHIC_POINTER_CYCLE")),
    ] {
        if let Some(Object::ObjectPointer(pointer)) =
            pool.object_mut_by_id(ObjectId::new(2).unwrap())
        {
            pointer.value = NullableObjectId::new(target);
        }
        let diagnostics = validate_complete_pool_state_for_version(&pool, VtVersion::Version6);
        let scaled: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.code.starts_with("SCALED_GRAPHIC_"))
            .map(|d| d.code.as_str())
            .collect();
        assert_eq!(scaled, expected.into_iter().collect::<Vec<_>>());
    }
}
