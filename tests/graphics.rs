use ag_iso_stack::object_pool::{
    object::{Object, PictureGraphic},
    ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::{
    default_object,
    graphics::{integration::GraphicService, GraphicAssetId},
    EditorProject,
};

fn png(pixels: &[[u8; 4]]) -> Vec<u8> {
    let image = image::RgbaImage::from_raw(
        pixels.len() as u32,
        1,
        pixels.iter().flatten().copied().collect(),
    )
    .unwrap();
    let mut bytes = std::io::Cursor::new(vec![]);
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn fixture() -> EditorProject {
    let mut pool = ObjectPool::default();
    for (id, object_type) in [
        (120, ObjectType::PictureGraphic),
        (500, ObjectType::PictureGraphic),
    ] {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        pool.add(object);
    }
    EditorProject::from(pool)
}

fn upload(project: &mut EditorProject, name: &str) -> GraphicAssetId {
    GraphicService::upload(project, name, png(&[[255, 0, 0, 255]; 16])).unwrap()
}

fn picture(project: &EditorProject, id: u16) -> PictureGraphic {
    match project
        .get_pool()
        .object_by_id(ObjectId::new(id).unwrap())
        .unwrap()
    {
        Object::PictureGraphic(picture) => picture.clone(),
        _ => panic!("expected PictureGraphic"),
    }
}

#[test]
fn converting_into_existing_picture_graphic_is_atomic_and_undoable() {
    let mut project = fixture();
    let asset = upload(&mut project, "WiFi.png");
    let before = picture(&project, 500);

    GraphicService::convert_asset_into_picture_graphic(&mut project, asset, 500).unwrap();
    assert!(!project.graphic_library().assets.contains_key(&asset));
    assert_ne!(picture(&project, 500), before);
    let converted = picture(&project, 500);

    assert!(project.undo_operation());
    assert!(project.graphic_library().assets.contains_key(&asset));
    assert_eq!(picture(&project, 500), before);
    assert!(project.redo_operation());
    assert!(!project.graphic_library().assets.contains_key(&asset));
    assert_eq!(picture(&project, 500), converted);
}

#[test]
fn save_reload_keeps_only_unconsumed_uploads() {
    let mut project = fixture();
    let wifi = upload(&mut project, "WiFi.png");
    let lock = upload(&mut project, "Lock.png");
    let warning = upload(&mut project, "Warning.png");

    GraphicService::convert_asset_into_picture_graphic(&mut project, wifi, 120).unwrap();
    let restored = EditorProject::load_project(project.save_project().unwrap()).unwrap();

    assert!(!restored.graphic_library().assets.contains_key(&wifi));
    assert_eq!(
        restored.graphic_library().assets[&lock].download().1,
        png(&[[255, 0, 0, 255]; 16])
    );
    assert_eq!(
        restored.graphic_library().assets[&warning].download().1,
        png(&[[255, 0, 0, 255]; 16])
    );
    assert!(matches!(
        restored
            .get_pool()
            .object_by_id(ObjectId::new(120).unwrap()),
        Some(Object::PictureGraphic(_))
    ));
}

#[test]
fn deleting_picture_recovers_current_image_and_is_one_undo_step() {
    use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat;
    use ag_iso_terminal_designer::graphics::preview;
    let mut pool = fixture().get_pool().clone();
    let Object::PictureGraphic(source) =
        pool.object_mut_by_id(ObjectId::new(120).unwrap()).unwrap()
    else {
        unreachable!()
    };
    source.width = 2;
    source.actual_width = 2;
    source.actual_height = 1;
    source.format = PictureGraphicFormat::EightBit;
    source.options.transparent = true;
    source.transparency_colour = 2;
    source.data = vec![2, 3];
    let mut map = default_object(ObjectType::ColourMap);
    *map.mut_id() = ObjectId::new(600).unwrap();
    if let Object::ColourMap(map) = &mut map {
        map.colour_map = (0..=255).collect();
        map.colour_map[3] = 2;
    }
    pool.add(map);
    let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
    *controls.mut_id() = ObjectId::new(601).unwrap();
    if let Object::WorkingSetSpecialControls(controls) = &mut controls {
        controls.id_of_colour_map = ObjectId::new(600).unwrap().into();
    }
    pool.add(controls);
    let mut project = EditorProject::from(pool);
    assert!(project.rename_object(ObjectId::new(120).unwrap(), "Warning".into()));
    let before = picture(&project, 120);
    // Equal resolved colours retain distinct transparency by source index.
    let decoded = preview::decode_picture(&before, project.get_pool());
    assert_eq!(decoded.pixels[0].a(), 0);
    assert_eq!(decoded.pixels[1].a(), 255);
    let expected = preview::picture_png(&before, project.get_pool()).unwrap();
    let png = image::load_from_memory(&expected).unwrap().to_rgba8();
    assert_eq!(png.get_pixel(0, 0).0[3], 0);
    assert_eq!(png.get_pixel(1, 0).0[3], 255);
    let other = upload(&mut project, "Keep.png");
    assert!(project.delete_object(ObjectId::new(120).unwrap()));
    assert!(project
        .get_pool()
        .object_by_id(ObjectId::new(120).unwrap())
        .is_none());
    assert_eq!(project.graphic_library().assets.len(), 2);
    let recovered = project
        .graphic_library()
        .assets
        .values()
        .find(|asset| asset.id != other)
        .unwrap()
        .clone();
    assert_eq!(recovered.name, "Warning");
    assert_eq!(recovered.original_bytes, expected);
    assert_eq!(recovered.format, "png");
    assert!(project.undo_operation());
    assert_eq!(picture(&project, 120), before);
    assert!(!project.graphic_library().assets.contains_key(&recovered.id));
    assert!(project.graphic_library().assets.contains_key(&other));
    assert!(project.redo_operation());
    assert_eq!(project.graphic_library().assets[&recovered.id], recovered);
    let restored = EditorProject::load_project(project.save_project().unwrap()).unwrap();
    assert_eq!(restored.graphic_library().assets[&recovered.id], recovered);
}

#[test]
fn format_conversion_maps_colour_indices_and_packs_each_odd_width_row() {
    use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat;
    let mut picture = match default_object(ObjectType::PictureGraphic) {
        Object::PictureGraphic(picture) => picture,
        _ => unreachable!(),
    };
    picture.actual_width = 3;
    picture.actual_height = 2;
    picture.format = PictureGraphicFormat::EightBit;
    picture.options.transparent = false;
    picture.data = vec![1, 2, 3, 3, 2, 1];
    let mut palette = [[0, 0, 0]; 256];
    palette[1] = [255, 0, 0];
    palette[2] = [0, 255, 0];
    palette[3] = [0, 0, 255];

    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut picture,
        PictureGraphicFormat::FourBit,
        &palette,
    )
    .unwrap();

    assert_eq!(picture.data, vec![0x12, 0x30, 0x32, 0x10]);
    assert_eq!(picture.format, PictureGraphicFormat::FourBit);
}

#[test]
fn conversions_cover_packed_formats_and_high_standard_palette_indices() {
    use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat as Format;
    let mut palette = [[0, 0, 0]; 256];
    palette[0] = [0, 0, 0];
    palette[1] = [255, 255, 255];
    palette[2] = [0, 255, 0];
    palette[4] = [128, 0, 0];
    palette[12] = [255, 0, 0];
    palette[196] = [255, 0, 0]; // standard palette's red cube entry

    let convert = |format, width, data: Vec<u8>| {
        let mut picture = match default_object(ObjectType::PictureGraphic) {
            Object::PictureGraphic(picture) => picture,
            _ => unreachable!(),
        };
        picture.actual_width = width;
        picture.actual_height = 1;
        picture.format = format;
        picture.options.transparent = false;
        picture.data = data;
        picture
    };

    let mut eight = convert(Format::EightBit, 1, vec![196]);
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut eight,
        Format::FourBit,
        &palette,
    )
    .unwrap();
    assert_eq!(eight.data, vec![0xC0]); // index 12, not masked index 4

    let mut four = convert(Format::FourBit, 2, vec![0x10]);
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut four,
        Format::Monochrome,
        &palette,
    )
    .unwrap();
    assert_eq!(four.data, vec![0x80]); // MSB first

    let mut mono = convert(Format::Monochrome, 2, vec![0x40]);
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut mono,
        Format::FourBit,
        &palette,
    )
    .unwrap();
    assert_eq!(mono.data, vec![0x01]);

    let mut mono = convert(Format::Monochrome, 2, vec![0x40]);
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut mono,
        Format::EightBit,
        &palette,
    )
    .unwrap();
    assert_eq!(mono.data, vec![0, 1]);

    let mut four = convert(Format::FourBit, 2, vec![0x12]);
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut four,
        Format::EightBit,
        &palette,
    )
    .unwrap();
    assert_eq!(four.data, vec![1, 2]);

    let mut rows = convert(Format::EightBit, 10, vec![1; 20]);
    rows.actual_height = 2;
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut rows,
        Format::Monochrome,
        &palette,
    )
    .unwrap();
    assert_eq!(rows.data, vec![0xFF, 0xC0, 0xFF, 0xC0]);
}

#[test]
fn monochrome_conversion_uses_msb_first_and_preserves_transparency() {
    use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat;
    let mut picture = match default_object(ObjectType::PictureGraphic) {
        Object::PictureGraphic(picture) => picture,
        _ => unreachable!(),
    };
    picture.actual_width = 2;
    picture.actual_height = 1;
    picture.format = PictureGraphicFormat::EightBit;
    picture.options.transparent = true;
    picture.transparency_colour = 2;
    picture.data = vec![1, 2];
    let mut palette = [[0, 0, 0]; 256];
    palette[0] = [0, 0, 0];
    palette[1] = [255, 255, 255];
    palette[2] = [20, 20, 20];

    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut picture,
        PictureGraphicFormat::Monochrome,
        &palette,
    )
    .unwrap();

    assert_eq!(picture.data, vec![0x80]);
    assert_eq!(picture.transparency_colour, 0);
    assert_eq!(
        picture.options.data_code_type,
        ag_iso_stack::object_pool::object_attributes::DataCodeType::Raw
    );
}

#[test]
fn opaque_imported_black_and_white_keeps_both_monochrome_colours() {
    use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat as Format;
    let mut picture = match default_object(ObjectType::PictureGraphic) {
        Object::PictureGraphic(picture) => picture,
        _ => unreachable!(),
    };
    ag_iso_terminal_designer::graphics::apply_image_bytes_to_picture_graphic(
        &mut picture,
        &png(&[[0, 0, 0, 255], [255, 255, 255, 255]]),
    )
    .unwrap();
    assert!(picture.options.transparent);
    assert_eq!(picture.transparency_colour, 1);
    let palette = ag_iso_terminal_designer::effective_colour::colours(
        &ag_iso_stack::object_pool::ObjectPool::default(),
    )
    .map(|colour| colour.as_rgb());

    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut picture,
        Format::Monochrome,
        &palette,
    )
    .unwrap();

    assert_eq!(picture.data, vec![0x40]);
    assert!(!picture.options.transparent);
}

#[test]
fn fill_pattern_conversion_enforces_row_padding_restrictions() {
    use ag_iso_stack::object_pool::{object_attributes::PictureGraphicFormat, NullableObjectId};
    let mut pool = fixture().get_pool().clone();
    let picture_id = ObjectId::new(120).unwrap();
    let mut fill = default_object(ObjectType::FillAttributes);
    fill.mut_id().set_value(700).unwrap();
    if let Object::FillAttributes(fill) = &mut fill {
        fill.fill_type = 3;
        fill.fill_pattern = NullableObjectId::new(picture_id.value());
    }
    pool.add(fill);

    assert!(ag_iso_terminal_designer::graphics::validate_picture_format(
        &pool,
        picture_id,
        10,
        PictureGraphicFormat::Monochrome,
    )
    .is_err());
    assert!(ag_iso_terminal_designer::graphics::validate_picture_format(
        &pool,
        picture_id,
        10,
        PictureGraphicFormat::FourBit,
    )
    .is_ok());

    // Fill Pattern is ignored while Fill Type is not pattern fill.
    if let Some(Object::FillAttributes(fill)) = pool.object_mut_by_id(ObjectId::new(700).unwrap()) {
        fill.fill_type = 2;
    }
    assert!(ag_iso_terminal_designer::graphics::validate_picture_format(
        &pool,
        picture_id,
        10,
        PictureGraphicFormat::Monochrome,
    )
    .is_ok());
}

#[test]
fn conversion_uses_rle_only_when_smaller_and_reports_current_sizes() {
    use ag_iso_stack::object_pool::object_attributes::{
        DataCodeType, PictureGraphicFormat as Format,
    };
    let mut palette = [[0, 0, 0]; 256];
    palette[1] = [20, 20, 20];
    let mut repeated = match default_object(ObjectType::PictureGraphic) {
        Object::PictureGraphic(picture) => picture,
        _ => unreachable!(),
    };
    repeated.actual_width = 8;
    repeated.actual_height = 1;
    repeated.format = Format::EightBit;
    repeated.options.transparent = false;
    repeated.data = vec![1; 8];
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut repeated,
        Format::FourBit,
        &palette,
    )
    .unwrap();
    assert_eq!(repeated.data, vec![4, 0x11]);
    assert_eq!(repeated.options.data_code_type, DataCodeType::RunLength);

    let mut tied = repeated.clone();
    tied.actual_width = 2;
    tied.format = Format::EightBit;
    tied.options.data_code_type = DataCodeType::Raw;
    tied.data = vec![1, 1];
    ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
        &mut tied,
        Format::FourBit,
        &palette,
    )
    .unwrap();
    assert_eq!(tied.data, vec![0x11]);
    assert_eq!(tied.options.data_code_type, DataCodeType::Raw);

    tied.actual_width = 3;
    tied.actual_height = 2;
    tied.format = Format::FourBit;
    tied.data = vec![0x12, 0x30, 0x32, 0x10];
    assert_eq!(
        ag_iso_terminal_designer::graphics::picture_data_sizes(&tied),
        (6, 4, 4)
    );
}

#[test]
fn invalid_conversion_leaves_the_picture_unchanged() {
    use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat as Format;
    let mut picture = match default_object(ObjectType::PictureGraphic) {
        Object::PictureGraphic(picture) => picture,
        _ => unreachable!(),
    };
    picture.actual_width = 2;
    picture.actual_height = 1;
    picture.format = Format::EightBit;
    picture.data = vec![1];
    let before = picture.clone();
    assert!(
        ag_iso_terminal_designer::graphics::convert_picture_graphic_format(
            &mut picture,
            Format::FourBit,
            &[[0, 0, 0]; 256],
        )
        .is_err()
    );
    assert_eq!(picture, before);
}
