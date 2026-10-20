use ag_iso_stack::object_pool::{object::Object, vt_version::VtVersion, ObjectId};
use ag_iso_terminal_designer::EditorProject;

#[test]
fn imports_string_fields_in_wire_order_without_losing_record_alignment() {
    // Input String followed by Output String and Number Variable. Distinct
    // field values expose reordered reads; the final record checks alignment.
    let bytes = vec![
        1, 0, 8, 120, 0, 30, 0, 4, 10, 0, 255, 255, 0, 255, 255, 0, 3, b'A', b'B', b'C', 1, 0, 2,
        0, 11, 90, 0, 20, 0, 5, 10, 0, 0, 255, 255, 0, 2, 0, b'D', b'E', 0, 3, 0, 21, 42, 0, 0, 0,
    ];
    for version in 2..=6 {
        let project =
            EditorProject::import_iop(bytes.clone(), VtVersion::try_from(version).unwrap())
                .unwrap();
        let pool = project.get_pool();
        assert_eq!(pool.size(), 3);
        let input = pool
            .input_string_object_by_id(ObjectId::new(1).unwrap())
            .unwrap();
        assert_eq!(
            (input.width, input.height, input.background_colour),
            (120, 30, 4)
        );
        assert_eq!(u16::from(input.font_attributes), 10);
        assert!(input.enabled);
        assert_eq!(input.value.wire_bytes(), b"ABC");
        let output = pool
            .output_string_object_by_id(ObjectId::new(2).unwrap())
            .unwrap();
        assert_eq!(
            (output.width, output.height, output.background_colour),
            (90, 20, 5)
        );
        assert_eq!(u16::from(output.font_attributes), 10);
        assert_eq!(output.value.wire_bytes(), b"DE");
        assert!(
            matches!(pool.object_by_id(ObjectId::new(3).unwrap()), Some(Object::NumberVariable(value)) if value.value == 42)
        );
    }
}
