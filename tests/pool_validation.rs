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
