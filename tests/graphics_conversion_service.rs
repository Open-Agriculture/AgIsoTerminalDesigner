use ag_iso_stack::object_pool::{
    object::Object, object_attributes::PictureGraphicFormat, ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::{
    default_object,
    graphics::{conversion::ConversionOptions, integration::GraphicService, GraphicAssetId},
    EditorProject,
};

fn png(pixels: &[[u8; 4]]) -> Vec<u8> {
    let image = image::RgbaImage::from_raw(
        pixels.len() as u32,
        1,
        pixels.iter().flatten().copied().collect(),
    )
    .unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn fixture() -> EditorProject {
    let mut pool = ObjectPool::default();
    let mut object = default_object(ObjectType::PictureGraphic);
    object.mut_id().set_value(120).unwrap();
    let Object::PictureGraphic(picture) = &mut object else {
        unreachable!();
    };
    picture.width = 20;
    picture.actual_width = 2;
    picture.actual_height = 1;
    picture.format = PictureGraphicFormat::EightBit;
    picture.options.flashing = true;
    picture.data = vec![2, 3];
    pool.add(object);
    EditorProject::from(pool)
}

fn picture(project: &EditorProject) -> ag_iso_stack::object_pool::object::PictureGraphic {
    match project
        .get_pool()
        .object_by_id(ObjectId::new(120).unwrap())
        .unwrap()
    {
        Object::PictureGraphic(picture) => picture.clone(),
        _ => unreachable!(),
    }
}

fn upload(project: &mut EditorProject) -> GraphicAssetId {
    GraphicService::upload(
        project,
        "replacement.png",
        png(&[[255, 0, 0, 255], [0, 255, 0, 255]]),
    )
    .unwrap()
}

#[test]
fn configured_asset_replacement_and_undo_are_one_transaction() {
    let mut project = fixture();
    let asset = upload(&mut project);
    let before = picture(&project);
    let mut options = ConversionOptions::suggested(PictureGraphicFormat::FourBit);
    options.transparency =
        ag_iso_terminal_designer::graphics::conversion::TransparencyMode::Flatten;

    GraphicService::convert_asset_into_picture_graphic_with_options(
        &mut project,
        asset,
        120,
        &options,
    )
    .unwrap();

    let converted = picture(&project);
    assert_eq!(converted.format, PictureGraphicFormat::FourBit);
    assert_eq!((converted.actual_width, converted.actual_height), (2, 1));
    assert_eq!(converted.width, before.width);
    assert!(converted.options.flashing);
    assert!(!project.graphic_library().assets.contains_key(&asset));

    assert!(project.undo_operation());
    assert_eq!(picture(&project), before);
    assert!(project.graphic_library().assets.contains_key(&asset));
    assert!(project.redo_operation());
    assert_eq!(picture(&project), converted);
    assert!(!project.graphic_library().assets.contains_key(&asset));
}

#[test]
fn failed_configured_conversion_keeps_picture_and_available_asset() {
    let mut project = fixture();
    let asset = upload(&mut project);
    let before = picture(&project);
    let mut options = ConversionOptions::suggested(PictureGraphicFormat::FourBit);
    options.transparent_index = Some(16);

    assert!(
        GraphicService::convert_asset_into_picture_graphic_with_options(
            &mut project,
            asset,
            120,
            &options,
        )
        .is_err()
    );

    assert_eq!(picture(&project), before);
    assert!(project.graphic_library().assets.contains_key(&asset));
}
