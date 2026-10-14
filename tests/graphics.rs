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
fn available_asset_conversion_creates_a_named_picture_graphic_and_consumes_the_asset() {
    let mut project = fixture();
    let asset = upload(&mut project, "Warning.png");

    let object_id = GraphicService::create_picture_graphic_from_asset(&mut project, asset).unwrap();

    assert!(!project.graphic_library().assets.contains_key(&asset));
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(object_id).unwrap())
        .unwrap();
    assert!(matches!(object, Object::PictureGraphic(_)));
    assert_eq!(
        project.get_object_info(object).name.as_deref(),
        Some("Warning")
    );
    assert_eq!(picture(&project, object_id).actual_width, 16);
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
fn graphics_panel_lists_every_picture_and_previews_every_source() {
    use ag_iso_terminal_designer::graphics::{preview, ui};
    let mut project = fixture();
    let first = upload(&mut project, "first.png");
    let second = upload(&mut project, "second.png");
    let ids: Vec<_> = ui::picture_graphics(&project)
        .iter()
        .map(|object| object.id().value())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&120) && ids.contains(&500));
    assert_eq!(project.graphic_library().assets.len(), 2);
    let ctx = egui::Context::default();
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| ui::show(ui, &mut project));
    });
    assert!(!output.shapes.is_empty());
    // Rendering the panel has already populated both available-image caches.
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        for id in [first, second] {
            assert!(preview::asset_texture(ctx, &project.graphic_library().assets[&id]).is_some());
        }
    });
    assert!(output.textures_delta.set.is_empty());
}

#[test]
fn create_is_one_undo_step_and_assign_uses_authoritative_conversion() {
    use ag_iso_terminal_designer::graphics::{apply_image_bytes_to_picture_graphic, ui};
    let mut project = fixture();
    let asset = upload(&mut project, "Warning.png");
    let mut expected = picture(&project, 120);
    apply_image_bytes_to_picture_graphic(
        &mut expected,
        &project.graphic_library().assets[&asset].original_bytes,
    )
    .unwrap();
    GraphicService::convert_asset_into_picture_graphic(&mut project, asset, 120).unwrap();
    assert_eq!(picture(&project, 120), expected);
    assert!(project.undo_operation());
    let id = GraphicService::create_picture_graphic_from_asset(&mut project, asset).unwrap();
    assert_eq!(ui::picture_graphics(&project).len(), 3);
    assert!(!project.graphic_library().assets.contains_key(&asset));
    ui::go_to_picture_graphic(&project, ObjectId::new(id).unwrap());
    assert!(project.update_selected());
    assert_eq!(project.get_selected().0.unwrap().value(), id);
    assert!(project.undo_operation());
    assert_eq!(ui::picture_graphics(&project).len(), 2);
    assert!(project.graphic_library().assets.contains_key(&asset));
    assert!(project.redo_operation());
    assert_eq!(ui::picture_graphics(&project).len(), 3);
}

#[test]
fn download_original_and_delete_only_available_asset() {
    let mut project = fixture();
    let first = upload(&mut project, "original.png");
    let second = upload(&mut project, "keep.png");
    let before = picture(&project, 120);
    let (filename, bytes) = project.graphic_library().assets[&first].download();
    assert_eq!(filename, "original.png");
    assert_eq!(bytes, png(&[[255, 0, 0, 255]; 16]));
    GraphicService::delete_available_asset(&mut project, first).unwrap();
    assert!(!project.graphic_library().assets.contains_key(&first));
    assert!(project.graphic_library().assets.contains_key(&second));
    assert_eq!(picture(&project, 120), before);
    assert!(project.undo_operation());
    assert!(project.graphic_library().assets.contains_key(&first));
}

#[test]
fn export_and_texture_cache_follow_current_picture_and_source_bytes() {
    use ag_iso_stack::object_pool::object_attributes::{DataCodeType, PictureGraphicFormat};
    use ag_iso_terminal_designer::graphics::preview;
    let mut project = fixture();
    let asset = upload(&mut project, "source.png");
    let mut source = project.graphic_library().assets[&asset].clone();
    let ctx = egui::Context::default();
    let original_texture = preview::asset_texture(&ctx, &source).unwrap();
    assert_eq!(
        original_texture.id(),
        preview::asset_texture(&ctx, &source).unwrap().id()
    );
    source.original_bytes = png(&[[0, 255, 0, 255]]);
    assert_ne!(
        original_texture.id(),
        preview::asset_texture(&ctx, &source).unwrap().id()
    );

    let mut picture = picture(&project, 120);
    picture.actual_width = 1;
    picture.actual_height = 1;
    picture.format = PictureGraphicFormat::EightBit;
    picture.options.data_code_type = DataCodeType::Raw;
    picture.options.transparent = false;
    picture.data = vec![2];
    let pool = project.get_pool();
    let first = preview::picture_texture(&ctx, &picture, pool).unwrap();
    assert_eq!(
        first.id(),
        preview::picture_texture(&ctx, &picture, pool).unwrap().id()
    );
    picture.data = vec![3];
    let changed = preview::picture_texture(&ctx, &picture, pool).unwrap();
    assert_ne!(first.id(), changed.id());
    let decoded = image::load_from_memory(&preview::picture_png(&picture, pool).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(
        decoded.get_pixel(0, 0).0[..3],
        ag_iso_terminal_designer::effective_colour::colour(pool, 3).as_rgb()
    );
    picture.options.transparent = true;
    picture.transparency_colour = 3;
    assert_ne!(
        changed.id(),
        preview::picture_texture(&ctx, &picture, pool).unwrap().id()
    );
    let decoded = image::load_from_memory(&preview::picture_png(&picture, pool).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(decoded.get_pixel(0, 0)[3], 0);
}

#[test]
fn picture_cache_tracks_format_and_effective_palette() {
    use ag_iso_stack::object_pool::{
        object_attributes::{DataCodeType, PictureGraphicFormat},
        Colour, NullableObjectId,
    };
    use ag_iso_terminal_designer::graphics::preview;
    let mut pool = ObjectPool::default();
    let mut palette = default_object(ObjectType::ColourPalette);
    palette.mut_id().set_value(10).unwrap();
    if let Object::ColourPalette(palette) = &mut palette {
        palette.colours = vec![Colour::RED; 256];
    }
    pool.add(palette);
    let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
    controls.mut_id().set_value(11).unwrap();
    if let Object::WorkingSetSpecialControls(controls) = &mut controls {
        controls.id_of_colour_palette = NullableObjectId(Some(ObjectId::new(10).unwrap()));
    }
    pool.add(controls);
    let Object::PictureGraphic(mut picture) = default_object(ObjectType::PictureGraphic) else {
        panic!()
    };
    picture.actual_width = 1;
    picture.actual_height = 1;
    picture.data = vec![0];
    picture.options.data_code_type = DataCodeType::Raw;
    picture.options.transparent = false;
    picture.format = PictureGraphicFormat::EightBit;
    let ctx = egui::Context::default();
    let first = preview::picture_texture(&ctx, &picture, &pool).unwrap();
    picture.format = PictureGraphicFormat::FourBit;
    let second = preview::picture_texture(&ctx, &picture, &pool).unwrap();
    assert_ne!(first.id(), second.id());
    if let Some(Object::ColourPalette(palette)) = pool.object_mut_by_id(ObjectId::new(10).unwrap())
    {
        palette.colours[0] = Colour::BLUE;
    }
    let third = preview::picture_texture(&ctx, &picture, &pool).unwrap();
    assert_ne!(second.id(), third.id());
    let decoded = image::load_from_memory(&preview::picture_png(&picture, &pool).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 255, 255]);
}

#[test]
fn picture_decode_discards_packed_row_padding_and_compares_transparency_by_rgb() {
    use ag_iso_stack::object_pool::object_attributes::{DataCodeType, PictureGraphicFormat};
    use ag_iso_terminal_designer::{effective_colour, graphics::preview};

    let pool = ObjectPool::default();
    let Object::PictureGraphic(mut picture) = default_object(ObjectType::PictureGraphic) else {
        panic!()
    };
    picture.actual_width = 3;
    picture.actual_height = 2;
    picture.options.data_code_type = DataCodeType::Raw;
    picture.format = PictureGraphicFormat::FourBit;
    picture.data = vec![0x12, 0x30, 0x45, 0x60];
    let image = preview::decode_picture(&picture, &pool);
    let expected: Vec<_> = [1, 2, 3, 4, 5, 6]
        .map(|index| effective_colour::colour(&pool, index).as_rgba())
        .into_iter()
        .collect();
    let actual: Vec<_> = image.pixels.iter().map(|color| color.to_array()).collect();
    assert_eq!(actual, expected);

    picture.format = PictureGraphicFormat::Monochrome;
    picture.data = vec![0b1011_0000, 0b0101_0000];
    let image = preview::decode_picture(&picture, &pool);
    let expected: Vec<_> = [1, 0, 1, 0, 1, 0]
        .map(|index| effective_colour::colour(&pool, index).as_rgba())
        .into_iter()
        .collect();
    let actual: Vec<_> = image.pixels.iter().map(|color| color.to_array()).collect();
    assert_eq!(actual, expected);

    use ag_iso_stack::object_pool::{Colour, NullableObjectId};
    let mut duplicate_rgb_pool = ObjectPool::default();
    let mut palette = default_object(ObjectType::ColourPalette);
    palette.mut_id().set_value(10).unwrap();
    if let Object::ColourPalette(palette) = &mut palette {
        palette.colours = vec![Colour::RED; 256];
    }
    duplicate_rgb_pool.add(palette);
    let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
    controls.mut_id().set_value(11).unwrap();
    if let Object::WorkingSetSpecialControls(controls) = &mut controls {
        controls.id_of_colour_palette = NullableObjectId(Some(ObjectId::new(10).unwrap()));
    }
    duplicate_rgb_pool.add(controls);

    picture.actual_width = 2;
    picture.actual_height = 1;
    picture.format = PictureGraphicFormat::FourBit;
    picture.data = vec![0x01];
    picture.options.transparent = true;
    picture.transparency_colour = 0;
    let image = preview::decode_picture(&picture, &duplicate_rgb_pool);
    assert_eq!(image.pixels[0].a(), 0);
    assert_eq!(image.pixels[1].a(), 0);
}

#[test]
fn picture_texture_cache_separates_pools_with_same_picture_id() {
    use ag_iso_stack::object_pool::{object_attributes::DataCodeType, Colour, NullableObjectId};
    use ag_iso_terminal_designer::graphics::preview;

    fn pool_with_palette(colour: Colour) -> ObjectPool {
        let mut pool = ObjectPool::default();
        let mut palette = default_object(ObjectType::ColourPalette);
        palette.mut_id().set_value(10).unwrap();
        if let Object::ColourPalette(palette) = &mut palette {
            palette.colours = vec![colour; 256];
        }
        pool.add(palette);
        let mut controls = default_object(ObjectType::WorkingSetSpecialControls);
        controls.mut_id().set_value(11).unwrap();
        if let Object::WorkingSetSpecialControls(controls) = &mut controls {
            controls.id_of_colour_palette = NullableObjectId(Some(ObjectId::new(10).unwrap()));
        }
        pool.add(controls);
        pool
    }

    let first_pool = pool_with_palette(Colour::RED);
    let second_pool = pool_with_palette(Colour::BLUE);
    let Object::PictureGraphic(mut first_picture) = default_object(ObjectType::PictureGraphic)
    else {
        panic!()
    };
    first_picture.actual_width = 1;
    first_picture.actual_height = 1;
    first_picture.data = vec![3];
    first_picture.options.data_code_type = DataCodeType::Raw;
    let mut second_picture = first_picture.clone();
    second_picture.data = vec![4];

    let ctx = egui::Context::default();
    let first = preview::picture_texture(&ctx, &first_picture, &first_pool).unwrap();
    let second = preview::picture_texture(&ctx, &second_picture, &second_pool).unwrap();
    assert_ne!(first.id(), second.id());
    assert_eq!(
        preview::decode_picture(&first_picture, &first_pool).pixels[0].to_array(),
        [255, 0, 0, 255]
    );
    assert_eq!(
        preview::decode_picture(&second_picture, &second_pool).pixels[0].to_array(),
        [0, 0, 255, 255]
    );
}

#[test]
fn effective_colour_uses_lowest_controls_and_falls_back_for_missing_references() {
    use ag_iso_stack::object_pool::{Colour, NullableObjectId};
    use ag_iso_terminal_designer::effective_colour;

    let mut pool = ObjectPool::default();
    let mut selected_palette = default_object(ObjectType::ColourPalette);
    selected_palette.mut_id().set_value(10).unwrap();
    if let Object::ColourPalette(palette) = &mut selected_palette {
        palette.colours = vec![Colour::BLUE, Colour::RED];
    }
    pool.add(selected_palette);
    let mut other_palette = default_object(ObjectType::ColourPalette);
    other_palette.mut_id().set_value(20).unwrap();
    if let Object::ColourPalette(palette) = &mut other_palette {
        palette.colours = vec![Colour::GREEN; 256];
    }
    pool.add(other_palette);
    let mut map = default_object(ObjectType::ColourMap);
    map.mut_id().set_value(30).unwrap();
    if let Object::ColourMap(map) = &mut map {
        map.colour_map = vec![5]; // Index 1 falls back to index 1.
    }
    pool.add(map);

    let mut selected = default_object(ObjectType::WorkingSetSpecialControls);
    selected.mut_id().set_value(11).unwrap();
    if let Object::WorkingSetSpecialControls(controls) = &mut selected {
        controls.id_of_colour_map = NullableObjectId(Some(ObjectId::new(30).unwrap()));
        controls.id_of_colour_palette = NullableObjectId(Some(ObjectId::new(10).unwrap()));
    }
    pool.add(selected);
    let mut higher_id = default_object(ObjectType::WorkingSetSpecialControls);
    higher_id.mut_id().set_value(12).unwrap();
    if let Object::WorkingSetSpecialControls(controls) = &mut higher_id {
        controls.id_of_colour_palette = NullableObjectId(Some(ObjectId::new(20).unwrap()));
    }
    pool.add(higher_id);

    assert_eq!(
        effective_colour::colour(&pool, 1).as_rgb(),
        Colour::RED.as_rgb()
    );
    if let Some(Object::WorkingSetSpecialControls(controls)) =
        pool.object_mut_by_id(ObjectId::new(11).unwrap())
    {
        controls.id_of_colour_map = NullableObjectId(Some(ObjectId::new(99).unwrap()));
    }
    assert_eq!(
        effective_colour::colour(&pool, 1).as_rgb(),
        Colour::RED.as_rgb()
    );
    if let Some(Object::WorkingSetSpecialControls(controls)) =
        pool.object_mut_by_id(ObjectId::new(11).unwrap())
    {
        controls.id_of_colour_palette = NullableObjectId(Some(ObjectId::new(99).unwrap()));
    }
    assert_eq!(
        effective_colour::colour(&pool, 1).as_rgb(),
        pool.color_by_index(1).as_rgb()
    );
}

#[test]
fn create_preserves_uploaded_semantic_metadata() {
    let mut project = fixture();
    let asset = upload(&mut project, "Warning.png");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&project.save_project().unwrap()).unwrap();
    saved["graphics"]["assets"][asset.to_string()]["tags"] =
        serde_json::json!(["warning", "safety"]);
    saved["graphics"]["assets"][asset.to_string()]["description"] =
        serde_json::json!("Operator warning");
    let mut project = EditorProject::load_project(serde_json::to_vec(&saved).unwrap()).unwrap();
    let id = GraphicService::create_picture_graphic_from_asset(&mut project, asset).unwrap();
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(id).unwrap())
        .unwrap();
    let info = project.get_object_info(object);
    assert_eq!(info.name.as_deref(), Some("Warning"));
    assert_eq!(info.description.as_deref(), Some("Operator warning"));
    assert_eq!(info.tags, ["warning", "safety"]);
    assert!(project.undo_operation());
    assert_eq!(
        project.graphic_library().assets[&asset].tags,
        ["warning", "safety"]
    );
}

#[test]
fn deleting_picture_recovers_current_image_and_is_one_undo_step() {
    use ag_iso_terminal_designer::graphics::preview;
    let mut project = fixture();
    let asset = upload(&mut project, "Warning.png");
    GraphicService::convert_asset_into_picture_graphic(&mut project, asset, 120).unwrap();
    let before = picture(&project, 120);
    let expected = preview::picture_png(&before, project.get_pool()).unwrap();
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
fn inspector_available_assignment_consumes_asset_and_undo_restores_both() {
    let mut project = fixture();
    let asset = upload(&mut project, "WiFi.png");
    let before = picture(&project, 500);
    project.request_graphic_assignment(asset, ObjectId::new(500).unwrap());
    assert_eq!(picture(&project, 500), before);
    assert!(project.commit_graphic_assignment());
    assert!(!project.commit_graphic_assignment());
    assert!(!project.graphic_library().assets.contains_key(&asset));
    assert_ne!(picture(&project, 500), before);
    assert!(project.undo_operation());
    assert_eq!(picture(&project, 500), before);
    assert!(project.graphic_library().assets.contains_key(&asset));
}

#[test]
fn failed_deletion_does_not_recover_an_asset_and_empty_graphics_can_be_deleted() {
    use ag_iso_terminal_designer::{Operation, OperationTransaction};
    let mut project = fixture();
    let asset = upload(&mut project, "WiFi.png");
    GraphicService::convert_asset_into_picture_graphic(&mut project, asset, 120).unwrap();
    let mut transaction = OperationTransaction::new(None);
    transaction.add_operation(Operation::DeleteObject { object_id: 120 });
    transaction.add_operation(Operation::DeleteObject { object_id: 999 });
    assert!(project.execute_transaction(transaction).is_err());
    assert!(project.graphic_library().assets.is_empty());
    assert_eq!(picture(&project, 120).actual_width, 16);
    assert!(project.delete_object(ObjectId::new(500).unwrap()));
    assert!(project.graphic_library().assets.is_empty());
}

#[test]
fn inspector_assignment_to_missing_target_keeps_available_asset() {
    let mut project = fixture();
    let asset = upload(&mut project, "WiFi.png");
    project.request_graphic_assignment(asset, ObjectId::new(999).unwrap());
    assert!(!project.commit_graphic_assignment());
    assert!(project.take_operation_error().is_some());
    assert!(project.graphic_library().assets.contains_key(&asset));
}
