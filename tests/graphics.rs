use ag_iso_stack::object_pool::{
    object::{Object, PictureGraphic},
    object_attributes::{DataCodeType, PictureGraphicFormat},
    ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::{
    components::*,
    graphics::{integration::*, *},
    *,
};
use serde_json::json;
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
    for (id, ty) in [
        (2, ObjectType::DataMask),
        (120, ObjectType::PictureGraphic),
        (500, ObjectType::PictureGraphic),
    ] {
        let mut object = default_object(ty);
        object.mut_id().set_value(id).unwrap();
        pool.add(object);
    }
    EditorProject::from(pool)
}
fn picture(pool: &ObjectPool, id: u16) -> PictureGraphic {
    match pool.object_by_id(ObjectId::new(id).unwrap()).unwrap() {
        Object::PictureGraphic(p) => p.clone(),
        _ => panic!(),
    }
}
fn upload(project: &mut EditorProject, name: &str) -> GraphicAssetId {
    GraphicService::upload(project, name, png(&[[255, 0, 0, 255]; 16])).unwrap()
}
#[test]
fn unused_assets_and_exact_original_bytes_survive_save_reload() {
    let mut p = fixture();
    let wifi = upload(&mut p, "WiFi.png");
    let lock = upload(&mut p, "Lock.png");
    let warning = upload(&mut p, "Warning.png");
    GraphicService::assign(&mut p, 120, Some(wifi)).unwrap();
    let before = picture(p.get_pool(), 120);
    let restored = EditorProject::load_project(p.save_project().unwrap()).unwrap();
    assert_eq!(restored.graphic_library(), p.graphic_library());
    assert_eq!(restored.graphic_library().assets.len(), 3);
    assert!(restored.picture_for_graphic(wifi).unwrap().is_some());
    for id in [lock, warning] {
        assert!(restored.picture_for_graphic(id).unwrap().is_none());
    }
    assert_eq!(picture(restored.get_pool(), 120), before);
    assert_eq!(
        restored.graphic_library().assets[&wifi].download(),
        ("WiFi.png".into(), png(&[[255, 0, 0, 255]; 16]).as_slice())
    );
}
#[test]
fn assignments_are_atomic_unique_and_removable_without_touching_raw_picture() {
    let mut p = fixture();
    let asset = upload(&mut p, "WiFi.png");
    let raw = picture(p.get_pool(), 120);
    GraphicService::assign(&mut p, 120, Some(asset)).unwrap();
    let bytes = p.save_project().unwrap();
    let revision = p.revision();
    let err = GraphicService::assign(&mut p, 500, Some(asset)).unwrap_err();
    assert_eq!(err.code, "GRAPHIC_ALREADY_ASSIGNED");
    assert!(err.message.contains("120") && err.message.contains("500"));
    assert_eq!(p.save_project().unwrap(), bytes);
    assert_eq!(p.revision(), revision);
    assert_eq!(
        GraphicService::delete(&mut p, asset).unwrap_err().code,
        "GRAPHIC_ASSET_ASSIGNED"
    );
    let mut manual = raw.clone();
    apply_image_bytes_to_picture_graphic(
        &mut manual,
        &p.graphic_library().assets[&asset].original_bytes,
    )
    .unwrap();
    assert_eq!(picture(&p.materialize().unwrap().pool, 120), manual);
    assert_eq!(picture(p.get_pool(), 120), raw);
    GraphicService::assign(&mut p, 120, None).unwrap();
    assert_eq!(picture(&p.materialize().unwrap().pool, 120), raw);
    GraphicService::assign(&mut p, 500, Some(asset)).unwrap();
}
#[test]
fn source_history_preserves_ids_metadata_and_ownership() {
    let mut p = fixture();
    p.mark_saved();
    let revision = p.revision();
    let asset = upload(&mut p, "WiFi.png");
    assert!(p.is_dirty());
    assert!(p.revision() > revision);
    p.undo();
    assert!(p.graphic_library().assets.is_empty());
    assert!(!p.is_dirty());
    p.redo();
    assert!(p.graphic_library().assets.contains_key(&asset));
    GraphicService::rename(&mut p, asset, "Wireless".into()).unwrap();
    p.undo();
    assert_eq!(p.graphic_library().assets[&asset].name, "WiFi");
    p.redo();
    assert_eq!(p.graphic_library().assets[&asset].name, "Wireless");
    GraphicService::assign(&mut p, 120, Some(asset)).unwrap();
    p.undo();
    assert!(p.picture_for_graphic(asset).unwrap().is_none());
    p.redo();
    assert!(p.picture_for_graphic(asset).unwrap().is_some());
    GraphicService::assign(&mut p, 120, None).unwrap();
    GraphicService::delete(&mut p, asset).unwrap();
    assert!(!p.graphic_library().assets.contains_key(&asset));
    p.undo();
    assert!(p.graphic_library().assets.contains_key(&asset));
}
#[test]
fn legacy_files_and_invalid_uploads() {
    let p = fixture();
    let mut json: serde_json::Value = serde_json::from_slice(&p.save_project().unwrap()).unwrap();
    json.as_object_mut().unwrap().remove("graphics");
    let mut loaded = EditorProject::load_project(serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(loaded.graphic_library(), &GraphicLibrary::default());
    let revision = loaded.revision();
    assert!(GraphicService::upload(&mut loaded, "bad.png", vec![0, 1]).is_err());
    assert_eq!(loaded.revision(), revision);
}
#[test]
fn conversion_preserves_geometry_transparency_and_selects_smallest_encoding() {
    let mut p = picture(fixture().get_pool(), 120);
    p.width = 0;
    apply_image_bytes_to_picture_graphic(&mut p, &png(&[[255, 0, 0, 255]; 300])).unwrap();
    assert_eq!((p.width, p.actual_width, p.actual_height), (300, 300, 1));
    assert_eq!(p.options.data_code_type, DataCodeType::RunLength);
    assert_eq!(p.data, vec![255, 196, 45, 196]);
    apply_image_bytes_to_picture_graphic(
        &mut p,
        &png(&[[99, 99, 99, 0], [255, 0, 0, 128], [0, 0, 255, 255]]),
    )
    .unwrap();
    assert_eq!(p.width, 300);
    assert_eq!(p.actual_width, 3);
    assert_eq!(p.format, PictureGraphicFormat::EightBit);
    assert!(p.options.transparent);
    assert_eq!(p.transparency_colour, 1);
    assert_eq!(p.options.data_code_type, DataCodeType::Raw);
    assert_eq!(p.data, vec![1, 196, 21]);
}
#[test]
fn legacy_format_packing_regressions() {
    let cases = [
        (
            PictureGraphicFormat::EightBit,
            PictureGraphicFormat::FourBit,
            vec![1, 2, 15, 0, 7],
            vec![0x12, 0xf0],
        ),
        (
            PictureGraphicFormat::EightBit,
            PictureGraphicFormat::Monochrome,
            vec![0, 1, 0, 2, 0, 3, 0, 4, 9],
            vec![0xaa],
        ),
        (
            PictureGraphicFormat::FourBit,
            PictureGraphicFormat::EightBit,
            vec![0x12, 0xf0],
            vec![1, 2, 15, 0],
        ),
        (
            PictureGraphicFormat::FourBit,
            PictureGraphicFormat::Monochrome,
            vec![0, 1, 2, 3],
            vec![0xe4],
        ),
        (
            PictureGraphicFormat::Monochrome,
            PictureGraphicFormat::EightBit,
            vec![0xa5],
            vec![1, 0, 1, 0, 0, 1, 0, 1],
        ),
        (
            PictureGraphicFormat::Monochrome,
            PictureGraphicFormat::FourBit,
            vec![0xa5],
            vec![1, 0, 0, 0],
        ),
    ];
    for (from, to, input, expected) in cases {
        let mut p = picture(fixture().get_pool(), 120);
        p.format = from;
        p.data = input;
        p.options.data_code_type = DataCodeType::Raw;
        convert_picture_graphic_format(&mut p, to);
        assert_eq!(p.data, expected);
        assert_eq!(p.format, to);
        assert_eq!(p.options.data_code_type, DataCodeType::Raw);
    }
}
fn definition(asset: GraphicAssetId) -> ComponentDefinition {
    let node = ComponentNodeId::default();
    ComponentDefinition {
        schema_version: 1,
        id: ComponentId::default(),
        name: "Graphic".into(),
        description: String::new(),
        category_path: vec![],
        tags: vec![],
        revision: 0,
        root_node: node,
        nodes: vec![ComponentNode {
            id: node,
            name: "Icon".into(),
            object_type: ObjectType::PictureGraphic,
            properties: Default::default(),
            children: vec![],
            references: Default::default(),
            macro_references: vec![],
            object_list: None,
        }],
        parameters: vec![ComponentParameter {
            id: ComponentParameterId::default(),
            name: "Icon".into(),
            description: None,
            target_node: node,
            property: "graphic".into(),
            kind: ComponentParameterKind::Graphic,
            default_value: json!(asset),
        }],
    }
}
#[test]
fn components_cannot_multiply_ownership_and_overrides_are_atomic() {
    let mut p = fixture();
    let a = upload(&mut p, "WiFi.png");
    let b = upload(&mut p, "Lock.png");
    let d = definition(a);
    let cid = d.id;
    let param = d.parameters[0].id;
    ComponentService::edit_component_definition(&mut p, d).unwrap();
    let i =
        ComponentService::instantiate_component(&mut p, cid, 2, 0, 0, Default::default()).unwrap();
    assert!(matches!(
        p.picture_for_graphic(a).unwrap(),
        Some(PictureGraphicSource::ComponentNode { .. })
    ));
    let rev = p.revision();
    assert_eq!(
        ComponentService::instantiate_component(&mut p, cid, 2, 0, 0, Default::default())
            .unwrap_err()
            .code,
        "GRAPHIC_ALREADY_ASSIGNED"
    );
    assert_eq!(p.revision(), rev);
    ComponentService::set_component_override(&mut p, i, param, json!(b)).unwrap();
    assert!(p.picture_for_graphic(a).unwrap().is_none());
    let j =
        ComponentService::instantiate_component(&mut p, cid, 2, 0, 0, Default::default()).unwrap();
    assert_eq!(
        ComponentService::reset_component_override(&mut p, i, param)
            .unwrap_err()
            .code,
        "GRAPHIC_ALREADY_ASSIGNED"
    );
    let loaded = EditorProject::load_project(p.save_project().unwrap()).unwrap();
    assert_eq!(loaded.component_source(), p.component_source());
    assert_ne!(i, j);
}
#[test]
fn ai_catalog_proposals_and_duplicate_assignment_enforcement() {
    let mut p = fixture();
    let a = upload(&mut p, "WiFi.png");
    let available = catalog(
        p.component_source(),
        p.graphic_library(),
        "wifi",
        "Available",
        20,
    )
    .unwrap();
    assert_eq!(available["assets"][0]["status"], "available");
    assert!(available["assets"][0].get("original_bytes").is_none());
    let state = p.ai_design_state();
    let proposal = ai::proposal::propose_component_intents(
        &state,
        vec![ComponentIntent::AssignGraphicAsset {
            object_id: 120,
            asset_id: a,
        }],
    )
    .unwrap();
    assert!(p.picture_for_graphic(a).unwrap().is_none());
    p.apply_ai_proposal(proposal).unwrap();
    let assigned = catalog(p.component_source(), p.graphic_library(), "wifi", "all", 20).unwrap();
    assert_eq!(assigned["assets"][0]["picture_graphic"]["object_id"], 120);
    assert_eq!(assigned["assets"][0]["status"], "assigned");
    let err = ai::proposal::propose_component_intents(
        &p.ai_design_state(),
        vec![ComponentIntent::AssignGraphicAsset {
            object_id: 500,
            asset_id: a,
        }],
    )
    .unwrap_err();
    assert!(format!("{err:?}").contains("GRAPHIC_ALREADY_ASSIGNED"));
    assert!(ai::backend::SYSTEM_INSTRUCTIONS.contains("Never assign an asset to a second source"));
}

#[test]
fn assigned_graphic_can_be_reused_by_links_and_component_references() {
    let mut p = fixture();
    let a = upload(&mut p, "WiFi.png");
    GraphicService::assign(&mut p, 120, Some(a)).unwrap();
    GraphicService::reuse_child(&mut p, a, 2).unwrap();
    let mut d = definition(a);
    d.parameters.clear();
    d.nodes[0].object_type = ObjectType::ObjectPointer;
    d.nodes[0].references.insert(
        "value".into(),
        ComponentReference::ExternalObject(GraphicService::reuse_target(&p, a).unwrap()),
    );
    let cid = d.id;
    ComponentService::edit_component_definition(&mut p, d).unwrap();
    ComponentService::instantiate_component(&mut p, cid, 2, 0, 0, Default::default()).unwrap();
    ComponentService::instantiate_component(&mut p, cid, 2, 10, 10, Default::default()).unwrap();
    assert_eq!(
        assignments(p.component_source(), p.graphic_library())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        p.picture_for_graphic(a).unwrap(),
        Some(PictureGraphicSource::Primitive { object_id: 120 })
    );
}
#[test]
fn detach_moves_ownership_to_ordinary_picture_and_preserves_raw_fallback() {
    let mut p = fixture();
    let a = upload(&mut p, "WiFi.png");
    let d = definition(a);
    let cid = d.id;
    let node = d.root_node;
    ComponentService::edit_component_definition(&mut p, d).unwrap();
    let i =
        ComponentService::instantiate_component(&mut p, cid, 2, 0, 0, Default::default()).unwrap();
    let id = p.component_source().instances[0].compiled_ids[&node];
    let effective = picture(&p.materialize().unwrap().pool, id);
    assert_eq!(
        GraphicService::reuse_target(&p, a).unwrap_err().code,
        "GRAPHIC_REUSE_NOT_SUPPORTED"
    );
    ComponentService::detach_component_instance(&mut p, i).unwrap();
    assert_eq!(
        p.picture_for_graphic(a).unwrap(),
        Some(PictureGraphicSource::Primitive { object_id: id })
    );
    assert_eq!(picture(&p.materialize().unwrap().pool, id), effective);
    assert_ne!(picture(p.get_pool(), id), effective);
    p.undo();
    assert!(matches!(
        p.picture_for_graphic(a).unwrap(),
        Some(PictureGraphicSource::ComponentNode { .. })
    ));
    p.redo();
    GraphicService::assign(&mut p, id, None).unwrap();
    assert_eq!(
        picture(&p.materialize().unwrap().pool, id),
        picture(p.get_pool(), id)
    );
}
#[test]
fn bound_format_conversion_preserves_original_and_survives_history_and_save() {
    let mut p = fixture();
    let a = upload(&mut p, "WiFi.png");
    let raw = picture(p.get_pool(), 120);
    GraphicService::assign(&mut p, 120, Some(a)).unwrap();
    let mut expected = picture(&p.materialize().unwrap().pool, 120);
    convert_picture_graphic_format(&mut expected, PictureGraphicFormat::FourBit);
    GraphicService::set_format(&mut p, 120, PictureGraphicFormat::FourBit).unwrap();
    assert_eq!(picture(&p.materialize().unwrap().pool, 120), expected);
    assert_eq!(picture(p.get_pool(), 120), raw);
    p.undo();
    assert_eq!(
        picture(&p.materialize().unwrap().pool, 120).format,
        PictureGraphicFormat::EightBit
    );
    p.redo();
    let mut p = EditorProject::load_project(p.save_project().unwrap()).unwrap();
    assert_eq!(picture(&p.materialize().unwrap().pool, 120), expected);
    GraphicService::assign(&mut p, 120, None).unwrap();
    assert_eq!(picture(&p.materialize().unwrap().pool, 120), raw);
}
#[test]
fn malformed_saved_bindings_are_rejected_and_available_defaults_can_be_deleted() {
    let mut p = fixture();
    let a = upload(&mut p, "WiFi.png");
    let d = definition(a);
    let cid = d.id;
    ComponentService::edit_component_definition(&mut p, d).unwrap();
    GraphicService::delete(&mut p, a).unwrap();
    assert_eq!(
        ComponentService::instantiate_component(&mut p, cid, 2, 0, 0, Default::default())
            .unwrap_err()
            .code,
        "GRAPHIC_ASSET_NOT_FOUND"
    );
    p.undo();
    GraphicService::assign(&mut p, 120, Some(a)).unwrap();
    let mut saved: serde_json::Value = serde_json::from_slice(&p.save_project().unwrap()).unwrap();
    saved["graphics"]["picture_graphic_bindings"]["500"] =
        saved["graphics"]["picture_graphic_bindings"]["120"].clone();
    let error = EditorProject::load_project(serde_json::to_vec(&saved).unwrap())
        .err()
        .unwrap();
    assert!(error.contains("GRAPHIC_ALREADY_ASSIGNED"));
}
#[test]
fn stale_graphic_proposal_does_not_overwrite_new_source_edits() {
    let mut p = fixture();
    let a = upload(&mut p, "WiFi.png");
    let proposal = ai::proposal::propose_component_intents(
        &p.ai_design_state(),
        vec![ComponentIntent::AssignGraphicAsset {
            object_id: 120,
            asset_id: a,
        }],
    )
    .unwrap();
    GraphicService::rename(&mut p, a, "Wireless".into()).unwrap();
    assert!(p.apply_ai_proposal(proposal).is_err());
    assert!(p.picture_for_graphic(a).unwrap().is_none());
}
