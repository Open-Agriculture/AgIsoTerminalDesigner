use ag_iso_stack::object_pool::{
    object::Object, vt_version::VtVersion, Colour, NullableObjectId, ObjectId, ObjectType,
};
use ag_iso_terminal_designer::{
    ai::{self, visual_intent as visual},
    default_object, effective_colour as colours, AiDesignState, OperationTransaction,
};
use serde_json::{json, Value};
fn state() -> AiDesignState {
    let mut s = AiDesignState {
        canonical_pool: None,
        components: Default::default(),
        graphics: Default::default(),
        revision: 1,
        pool: Default::default(),
        object_info: Default::default(),
        vt_version: VtVersion::Version6,
        mask_size: 480,
        selected: None,
    };
    add(&mut s, ObjectType::WorkingSet, 1);
    add(&mut s, ObjectType::DataMask, 2);
    if let Some(Object::WorkingSet(w)) = s.pool.object_mut_by_id(id(1)) {
        w.active_mask = id(2);
    }
    s
}
fn id(n: u16) -> ObjectId {
    ObjectId::new(n).unwrap()
}
fn add(s: &mut AiDesignState, ty: ObjectType, n: u16) {
    let mut o = default_object(ty);
    o.mut_id().set_value(n).unwrap();
    s.pool.add(o);
}
fn set(n: u16, p: &str, v: Value) -> Value {
    json!({"type":"SetProperty","data":{"object":{"id":n},"property":p,"value":v}})
}
fn prepare(s: &AiDesignState, ops: Vec<Value>) -> ai::AiProposal {
    let (v,_)=visual::compile(s,json!({"schema_version":ag_iso_terminal_designer::CURRENT_OPERATION_SCHEMA_VERSION,"description":"Semantic styling","operations":ops})).unwrap();
    ai::propose_transaction(
        s,
        serde_json::from_value::<OperationTransaction>(v).unwrap(),
    )
    .unwrap()
}
fn custom(s: &mut AiDesignState) {
    add(s, ObjectType::ColourPalette, 500);
    add(s, ObjectType::ColourMap, 501);
    add(s, ObjectType::WorkingSetSpecialControls, 502);
    if let Some(Object::ColourPalette(p)) = s.pool.object_mut_by_id(id(500)) {
        p.colours = vec![Colour::BLACK; 256];
        p.colours[4] = Colour::NAVY;
        p.colours[6] = Colour::YELLOW;
        p.colours[1] = Colour::WHITE;
        p.colours[3] = Colour::CYAN;
    }
    if let Some(Object::ColourMap(m)) = s.pool.object_mut_by_id(id(501)) {
        m.colour_map = (0..=255).collect();
    }
    if let Some(Object::WorkingSetSpecialControls(c)) = s.pool.object_mut_by_id(id(502)) {
        c.id_of_colour_palette = NullableObjectId(Some(id(500)));
        c.id_of_colour_map = NullableObjectId(Some(id(501)));
    }
}
#[test]
fn modern_screen_regression_and_raw_index() {
    let mut s = state();
    custom(&mut s);
    let theme=visual::resolve_theme(&s.pool,&json!({"background":{"name":"dark_blue"},"accent":{"name":"cyan"},"primary_text":{"name":"white"}})).unwrap();
    assert_eq!(theme["background"]["index"], 4);
    let p = prepare(
        &s,
        vec![set(2, "background_colour", json!({"name":"dark navy"}))],
    );
    assert!(visual::theme_mismatches(&p, &theme).is_empty());
    let wrong = prepare(&s, vec![set(2, "background_colour", json!(6))]);
    assert_eq!(
        visual::theme_mismatches(&wrong, &theme)[0]["code"],
        "STYLE_INTENT_MISMATCH"
    );
    assert_eq!(
        colours::colour(&wrong.prepared.pool, 6).as_rgb(),
        Colour::YELLOW.as_rgb()
    );
}
#[test]
fn custom_palette_map_read_and_resolver_agree() {
    let mut s = state();
    custom(&mut s);
    if let Some(Object::ColourMap(m)) = s.pool.object_mut_by_id(id(501)) {
        m.colour_map.swap(4, 20);
    }
    assert_eq!(colours::resolve_name(&s.pool, "navy").unwrap().index, 20);
    assert_eq!(colours::colour(&s.pool, 20).as_rgb(), Colour::NAVY.as_rgb());
    let p = prepare(&s, vec![set(2, "background_colour", json!(6))]);
    s.pool = p.prepared.pool;
    let objects = ai::get_objects(
        &s,
        ai::GetObjectsRequest {
            ids: vec![2],
            ..Default::default()
        },
    )
    .unwrap();
    let v = serde_json::to_value(objects).unwrap();
    assert_eq!(
        v[0]["properties"]["background_colour"],
        json!({"index":6,"name":"yellow","rgb":"#FFFF00"})
    );
}
#[test]
fn deterministic_rgb_unknown_and_property_awareness() {
    let s = state();
    let a = visual::resolve(&s.pool, &json!({"rgb":"#06162E"})).unwrap();
    assert_eq!(a.index, 17);
    assert_eq!(
        a,
        visual::resolve(&s.pool, &json!({"rgb":"#06162E"})).unwrap()
    );
    assert_eq!(
        visual::resolve(&s.pool, &json!({"name":"ultraviolet midnight neon"})).unwrap_err()["code"],
        "AI_COLOUR_UNKNOWN"
    );
    let (v, _) = visual::compile(
        &s,
        json!({"operations":[set(2,"not_a_colour",json!({"name":"cyan"}))]}),
    )
    .unwrap();
    assert_eq!(v["operations"][0]["data"]["value"], json!({"name":"cyan"}));
    assert_eq!(
        colours::resolve_name(&s.pool, "grey"),
        colours::resolve_name(&s.pool, "gray")
    );
    assert_eq!(
        colours::resolve_name(&s.pool, "aqua"),
        colours::resolve_name(&s.pool, "cyan")
    );
}
#[test]
fn standard_icons_are_primitive_valid_deterministic_and_render() {
    let s = state();
    for icon in ai::icon_recipes::ICONS {
        let d = json!({"icon":icon,"name":"icon","width":24,"height":24,"foreground":{"name":"cyan"},"transparent":true,"parent":{"id":2},"x":20,"y":20});
        assert_eq!(
            ai::icon_recipes::compile(&s.pool, &d).unwrap(),
            ai::icon_recipes::compile(&s.pool, &d).unwrap()
        );
        let p = prepare(&s, vec![json!({"type":"CreateIcon","data":d})]);
        assert!(p.transaction.operations.iter().any(|o| matches!(
            o,
            ag_iso_terminal_designer::Operation::CreateObject {
                object_type: ObjectType::PictureGraphic,
                ..
            }
        )));
        assert!(ai::render_proposal_preview(&s, &p).is_ok());
        let ctx = egui::Context::default();
        let rendered = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ai::render_preview_ui(ui, &s, &p).unwrap();
            });
        });
        assert!(!rendered.textures_delta.set.is_empty());
        assert!(rendered
            .textures_delta
            .set
            .iter()
            .any(|(_, delta)| match &delta.image {
                egui::ImageData::Color(image) =>
                    image.pixels.contains(&egui::Color32::from_rgb(0, 255, 255)),
            }));
        let picture = p.prepared.pool.picture_graphic_objects()[0];
        assert_eq!(picture.data.len(), 576);
        assert!(picture.data.contains(&11));
        assert!(picture.data.iter().any(|v| *v != 11));
    }
}
fn labels(s: &mut AiDesignState) {
    add(s, ObjectType::FontAttributes, 23);
    for n in [24, 25] {
        add(s, ObjectType::OutputString, n);
        if let Some(Object::OutputString(o)) = s.pool.object_mut_by_id(id(n)) {
            o.font_attributes = id(23);
            o.width = 80;
            o.height = 24;
            o.value = "Status".into();
            o.options.transparent = true;
        }
        if let Some(Object::DataMask(m)) = s.pool.object_mut_by_id(id(2)) {
            m.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
                id: id(n),
                offset: ag_iso_stack::object_pool::object_attributes::Point {
                    x: 0,
                    y: (n - 24) as i16 * 30,
                },
            });
        }
    }
}
#[test]
fn contrast_and_shared_font_warnings_preserve_colours() {
    let mut s = state();
    labels(&mut s);
    let p = prepare(
        &s,
        vec![
            set(23, "font_colour", json!({"name":"white"})),
            set(2, "background_colour", json!({"name":"white"})),
        ],
    );
    assert_eq!(
        ai::visual_diagnostics::shared_style_impact(&s, &p)[0]["affected_objects_count"],
        2
    );
    let analysis = ai::analyze_design(&s, Some(&p), ai::DesignAnalysisScope::Proposal).unwrap();
    assert!(analysis
        .diagnostics
        .iter()
        .any(|d| d.code == "LOW_TEXT_CONTRAST"));
    let p = prepare(
        &s,
        vec![
            set(23, "font_colour", json!({"name":"white"})),
            set(2, "background_colour", json!({"name":"dark_blue"})),
        ],
    );
    assert!(
        !ai::analyze_design(&s, Some(&p), ai::DesignAnalysisScope::Proposal)
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code.starts_with("LOW_"))
    );
    assert_eq!(colours::contrast([255; 3], [255; 3]), 1.);
}
#[test]
fn accent_variation_and_existing_style_reuse() {
    let mut s = state();
    add(&mut s, ObjectType::Button, 15);
    let p = prepare(
        &s,
        vec![set(15, "background_colour", json!({"name":"cyan"}))],
    );
    s.pool = p.prepared.pool;
    let v = serde_json::to_value(
        ai::get_objects(
            &s,
            ai::GetObjectsRequest {
                ids: vec![15],
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    let existing = &v[0]["properties"]["background_colour"];
    assert_eq!(existing["name"], "cyan");
    let p = prepare(
        &s,
        vec![set(15, "background_colour", json!({"name":"red"}))],
    );
    assert!(visual::theme_mismatches(
        &p,
        &visual::resolve_theme(&s.pool, &json!({"accent":{"name":"cyan"}})).unwrap()
    )
    .is_empty());
}

#[test]
fn simultaneous_custom_map_edit_resolves_final_palette() {
    let mut s = state();
    custom(&mut s);
    let mut map: Vec<u8> = (0..=255).collect();
    map.swap(4, 20);
    let p = prepare(
        &s,
        vec![
            set(2, "background_colour", json!({"name":"navy"})),
            set(501, "colour_map", json!(map)),
        ],
    );
    let actual = ag_iso_terminal_designer::object_properties::get_object_property(
        p.prepared.pool.object_by_id(id(2)).unwrap(),
        "background_colour",
    )
    .unwrap();
    assert_eq!(actual, 20);
    assert_eq!(
        colours::colour(&p.prepared.pool, 20).as_rgb(),
        Colour::NAVY.as_rgb()
    );
}
#[test]
fn explicit_roles_compile_away_and_check_text_intent() {
    let mut s = state();
    add(&mut s, ObjectType::FontAttributes, 23);
    let mut op = set(23, "font_colour", json!({"name":"yellow"}));
    op["data"]["theme_role"] = json!("primary_text");
    let (v, metadata) = visual::compile(&s, json!({"operations":[op]})).unwrap();
    assert!(v["operations"][0]["data"].get("theme_role").is_none());
    let theme = visual::resolve_theme(&s.pool, &json!({"primary_text":{"name":"white"}})).unwrap();
    assert_eq!(
        visual::explicit_theme_mismatches(&s.pool, &json!(metadata), &theme)[0]["code"],
        "STYLE_INTENT_MISMATCH"
    );
}
#[test]
fn context_region_and_compact_styles_all_enrich_colours() {
    let mut s = state();
    labels(&mut s);
    let context = serde_json::to_value(
        ai::get_design_context(
            &s,
            ai::DesignContextRequest {
                include_properties: true,
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(context["objects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["properties"]["background_colour"]["rgb"].is_string()));
    let objects = serde_json::to_value(
        ai::get_objects(
            &s,
            ai::GetObjectsRequest {
                ids: vec![2],
                include_nearby_style: true,
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    let example = &objects[0]["nearby_style"]["examples"][0];
    assert!(example["background_colour"]["rgb"].is_string());
    assert!(example["font_attributes"]["font_colour"]["rgb"].is_string());
    let region = serde_json::to_value(
        ai::design_inspection::inspect_design_region(
            &s,
            serde_json::from_value(json!({"root_id":2,"region":"all"})).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(region["objects"][0]["style"]["background_colour"]["rgb"].is_string());
    assert!(region["objects"][0]["style"]["font_attributes"]["font_colour"]["rgb"].is_string());
}
#[test]
fn button_and_status_contrast_are_configurable_warnings() {
    let mut s = state();
    labels(&mut s);
    add(&mut s, ObjectType::Button, 15);
    if let Some(Object::DataMask(m)) = s.pool.object_mut_by_id(id(2)) {
        m.object_refs.retain(|r| r.id != id(24));
        m.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
            id: id(15),
            offset: ag_iso_stack::object_pool::object_attributes::Point { x: 100, y: 100 },
        });
    }
    if let Some(Object::Button(b)) = s.pool.object_mut_by_id(id(15)) {
        b.width = 100;
        b.height = 40;
        b.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
            id: id(24),
            offset: ag_iso_stack::object_pool::object_attributes::Point { x: 0, y: 0 },
        });
    }
    let mut info = ag_iso_terminal_designer::ObjectInfo::new(s.pool.object_by_id(id(25)).unwrap());
    info.set_name("Important status".into());
    s.object_info.insert(id(25), info);
    let p = prepare(
        &s,
        vec![
            set(23, "font_colour", json!({"name":"white"})),
            set(2, "background_colour", json!({"name":"white"})),
            set(15, "background_colour", json!({"name":"white"})),
        ],
    );
    let diagnostics = ai::analyze_design(&s, Some(&p), ai::DesignAnalysisScope::Proposal)
        .unwrap()
        .diagnostics;
    let relaxed = ai::design_analysis::analyze_design_with_thresholds(
        &s,
        Some(&p),
        ai::DesignAnalysisScope::Proposal,
        ai::visual_diagnostics::ContrastThresholds {
            text: 1.,
            button: 1.,
            status: 1.,
        },
    )
    .unwrap();
    assert!(!relaxed
        .diagnostics
        .iter()
        .any(|d| d.code.starts_with("LOW_")));
    for code in ["LOW_BUTTON_CONTRAST", "LOW_STATUS_CONTRAST"] {
        let d = diagnostics.iter().find(|d| d.code == code).unwrap();
        assert_eq!(d.context.as_ref().unwrap()["contrast_ratio"], 1.);
    }
}

#[test]
fn custom_palette_renders_and_invalidates_picture_texture_cache() {
    let mut s = state();
    custom(&mut s);
    let p = prepare(
        &s,
        vec![
            set(2, "background_colour", json!({"name":"yellow"})),
            json!({"type":"CreateIcon","data":{"icon":"wifi","name":"wifi","width":24,"height":24,"foreground":{"name":"cyan"},"transparent":true,"parent":{"id":2}}}),
        ],
    );
    let ctx = egui::Context::default();
    let render = |p: &ai::AiProposal| {
        ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ai::render_preview_ui(ui, &s, p).unwrap();
            });
        })
    };
    let first = render(&p);
    assert!(first
        .shapes
        .iter()
        .any(|s| matches!(&s.shape,egui::Shape::Rect(r) if r.fill==egui::Color32::YELLOW)));
    let mut changed = p.clone();
    if let Some(Object::ColourPalette(p)) = changed.prepared.pool.object_mut_by_id(id(500)) {
        p.colours[3] = Colour::RED;
    }
    let next = render(&changed);
    assert!(next
        .textures_delta
        .set
        .iter()
        .any(|(_, delta)| match &delta.image {
            egui::ImageData::Color(image) => image.pixels.contains(&egui::Color32::RED),
        }));
}
