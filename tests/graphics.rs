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
