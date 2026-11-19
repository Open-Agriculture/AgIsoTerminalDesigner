use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_terminal_designer::macro_id_remap::{remap_macro_object_ids, MacroAttributeValueKind};
use std::collections::HashMap;

fn mapping(pairs: &[(u16, u16)]) -> HashMap<u16, u16> {
    pairs.iter().copied().collect()
}

fn numeric(_: u16, _: Option<u8>) -> MacroAttributeValueKind {
    MacroAttributeValueKind::Numeric
}

#[test]
fn remaps_ids_in_fixed_variable_and_graphics_packets_only() {
    let mut bytes = vec![0xA0, 10, 0, 1, 2, 3, 4, 0xFF];
    // A 2-byte string command is padded to an 8-byte record.
    bytes.extend_from_slice(&[0xB3, 11, 0, 2, 0, b'h', b'i', 0xFF]);
    // Graphics Context draw-object subcommand (18), with ID in bytes 5-6.
    bytes.extend_from_slice(&[0xB8, 12, 0, 18, 13, 0, 0xFF, 0xFF]);
    // A numeric Change Attribute value that happens to equal an old ID.
    bytes.extend_from_slice(&[0xAF, 14, 0, 1, 10, 0, 0, 0]);

    let result = remap_macro_object_ids(
        &bytes,
        &mapping(&[(10, 110), (11, 111), (12, 112), (13, 113), (14, 114)]),
        VtVersion::Version4,
        numeric,
    )
    .unwrap();

    assert_eq!(&result[..3], &[0xA0, 110, 0]);
    assert_eq!(&result[9..11], &[111, 0]);
    assert_eq!(&result[17..20], &[112, 0, 18]);
    assert_eq!(&result[20..22], &[113, 0]);
    assert_eq!(&result[25..27], &[114, 0]);
    assert_eq!(&result[28..30], &[10, 0]);
}

#[test]
fn change_attribute_remaps_reference_value_but_preserves_numeric_value() {
    let bytes = [0xAF, 5, 0, 9, 7, 0, 0, 0];
    let result = remap_macro_object_ids(
        &bytes,
        &mapping(&[(5, 50), (7, 70)]),
        VtVersion::Version4,
        |_, aid| {
            if aid == Some(9) {
                MacroAttributeValueKind::LocalObjectId16 { value_offset: 0 }
            } else {
                MacroAttributeValueKind::Numeric
            }
        },
    )
    .unwrap();
    assert_eq!(result, [0xAF, 50, 0, 9, 70, 0, 0, 0]);

    let foreign_reference = [0xAF, 5, 0, 10, 7, 0, 0, 0];
    let result = remap_macro_object_ids(
        &foreign_reference,
        &mapping(&[(5, 50), (7, 70)]),
        VtVersion::Version4,
        |_, aid| {
            if aid == Some(10) {
                MacroAttributeValueKind::ForeignReference
            } else {
                MacroAttributeValueKind::Numeric
            }
        },
    )
    .unwrap();
    assert_eq!(result, [0xAF, 50, 0, 10, 7, 0, 0, 0]);
}

#[test]
fn resolves_change_attribute_type_using_original_id_during_a_swap() {
    let bytes = [0xAF, 1, 0, 9, 2, 0, 0, 0];
    let mut seen_target = None;
    let result = remap_macro_object_ids(
        &bytes,
        &mapping(&[(1, 2), (2, 1)]),
        VtVersion::Version4,
        |target_id, aid| {
            seen_target = Some((target_id, aid));
            if target_id == 1 && aid == Some(9) {
                MacroAttributeValueKind::LocalObjectId16 { value_offset: 0 }
            } else {
                MacroAttributeValueKind::Numeric
            }
        },
    )
    .unwrap();
    assert_eq!(seen_target, Some((1, Some(9))));
    assert_eq!(result, [0xAF, 2, 0, 9, 1, 0, 0, 0]);
}

#[test]
fn honors_short_string_padding_and_long_string_length() {
    let mut bytes = vec![0xB3, 1, 0, 2, 0, b'h', b'i', 0xFF];
    // Eight payload bytes make this a 13-byte command with no padding.
    bytes.extend_from_slice(&[0xB3, 2, 0, 8, 0, 0xA0, 20, 0, 0xFF, 0xAF, 1, 2, 3]);
    // This command starts at 21; the apparent command bytes above belong
    // to the string and must not be parsed or rewritten.
    bytes.extend_from_slice(&[0xA0, 3, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);

    let result = remap_macro_object_ids(
        &bytes,
        &mapping(&[(1, 11), (2, 12), (3, 13), (20, 21)]),
        VtVersion::Version4,
        numeric,
    )
    .unwrap();

    assert_eq!(&result[1..3], &[11, 0]);
    assert_eq!(result[7], 0xFF);
    assert_eq!(&result[8..13], &[0xB3, 12, 0, 8, 0]);
    assert_eq!(&result[13..21], &[0xA0, 20, 0, 0xFF, 0xAF, 1, 2, 3]);
    assert_eq!(&result[22..24], &[13, 0]);
}

#[test]
fn malformed_variable_padding_is_rejected() {
    let bytes = [0xB3, 1, 0, 2, 0, b'h', b'i', 0];
    let result = remap_macro_object_ids(&bytes, &mapping(&[(1, 2)]), VtVersion::Version4, numeric);
    assert!(result.unwrap_err().contains("invalid command padding"));
    assert_eq!(bytes[1], 1);
}

#[test]
fn execute_macro_promotes_on_vt5_and_rejects_on_older_versions_atomically() {
    let bytes = [0xBE, 7, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    let ids = mapping(&[(7, 300)]);
    let promoted = remap_macro_object_ids(&bytes, &ids, VtVersion::Version5, numeric).unwrap();
    assert_eq!(&promoted[..3], &[0xBC, 44, 1]);
    assert!(remap_macro_object_ids(&bytes, &ids, VtVersion::Version4, numeric).is_err());
    assert_eq!(bytes[0], 0xBE);
}
