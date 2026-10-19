use ag_iso_stack::object_pool::{
    object::Object, vt_version::VtVersion, Colour, NullableObjectId, ObjectId, ObjectType,
};
use ag_iso_terminal_designer::{cli, default_object, effective_colour as colours, AiDesignState};
use serde_json::json;
fn state() -> AiDesignState {
    let mut s = AiDesignState {
        components: Default::default(),
        graphics: Default::default(),
        object_id_ranges: Default::default(),
        revision: 1,
        pool: Default::default(),
        object_info: Default::default(),
        vt_version: VtVersion::Version6,
        mask_size: 480,
        soft_key_size: (60, 60),
        soft_key_layout: Default::default(),
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
fn custom_palette_map_read_and_resolver_agree() {
    let mut s = state();
    custom(&mut s);
    if let Some(Object::ColourMap(m)) = s.pool.object_mut_by_id(id(501)) {
        m.colour_map.swap(4, 20);
    }
    assert_eq!(colours::resolve_name(&s.pool, "navy").unwrap().index, 20);
    assert_eq!(colours::colour(&s.pool, 20).as_rgb(), Colour::NAVY.as_rgb());
    if let Some(Object::DataMask(mask)) = s.pool.object_mut_by_id(id(2)) {
        mask.background_colour = 6;
    }
    let objects = cli::get_objects(
        &s,
        cli::GetObjectsRequest {
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
    let a = colours::resolve(&s.pool, "#06162E").unwrap();
    assert_eq!(a.index, 17);
    assert_eq!(a, colours::resolve(&s.pool, "#06162E").unwrap());
    assert!(colours::resolve(&s.pool, "ultraviolet midnight neon")
        .unwrap_err()
        .contains("Unknown colour"));
    assert_eq!(
        colours::resolve_name(&s.pool, "grey"),
        colours::resolve_name(&s.pool, "gray")
    );
    assert_eq!(
        colours::resolve_name(&s.pool, "aqua"),
        colours::resolve_name(&s.pool, "cyan")
    );
}
fn labels(s: &mut AiDesignState) {
    add(s, ObjectType::FontAttributes, 23);
    for n in [24, 25] {
        add(s, ObjectType::OutputString, n);
        if let Some(Object::OutputString(o)) = s.pool.object_mut_by_id(id(n)) {
            o.font_attributes = id(23).into();
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
fn context_region_and_compact_styles_all_enrich_colours() {
    let mut s = state();
    labels(&mut s);
    let context = serde_json::to_value(
        cli::get_design_context(
            &s,
            cli::DesignContextRequest {
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
        cli::get_objects(
            &s,
            cli::GetObjectsRequest {
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
        cli::design_inspection::inspect_design_region(
            &s,
            serde_json::from_value(json!({"root_id":2,"region":"all"})).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(region["objects"][0]["style"]["background_colour"]["rgb"].is_string());
    assert!(region["objects"][0]["style"]["font_attributes"]["font_colour"]["rgb"].is_string());
}
