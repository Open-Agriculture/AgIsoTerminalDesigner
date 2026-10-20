use ag_iso_stack::object_pool::{
    object::Object,
    object_attributes::{ObjectRef, Point},
    ObjectId, ObjectPool, ObjectType,
};
use ag_iso_terminal_designer::{
    cli::zip as project_zip,
    components::{
        plan_component_capture, ComponentParameter, ComponentParameterId, ComponentParameterKind,
        ComponentParameterTarget, ComponentReference, ComponentService,
    },
    default_object,
    graphics::{integration::GraphicService, GraphicAssetId},
    object_properties::PropertyKey,
    EditorProject,
};
use serde_json::json;
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::{Command, Output},
};
use uuid::Uuid;
use zip::ZipArchive;

fn object_id(value: u16) -> ObjectId {
    ObjectId::new(value).unwrap()
}

fn object_with_id(object_type: ObjectType, id: u16) -> Object {
    let mut object = default_object(object_type);
    object.mut_id().set_value(id).unwrap();
    object
}

fn push_child(pool: &mut ObjectPool, parent: u16, child: u16, x: i16, y: i16) {
    let reference = ObjectRef {
        id: object_id(child),
        offset: Point { x, y },
    };
    match pool.object_mut_by_id(object_id(parent)).unwrap() {
        Object::DataMask(mask) => mask.object_refs.push(reference),
        Object::AlarmMask(mask) => mask.object_refs.push(reference),
        Object::Button(button) => button.object_refs.push(reference),
        Object::Container(container) => container.object_refs.push(reference),
        object => panic!(
            "{parent} cannot contain children ({:?})",
            object.object_type()
        ),
    }
}

fn source_project() -> (EditorProject, ComponentParameterId, GraphicAssetId) {
    let mut pool = ObjectPool::default();
    for (object_type, id) in [
        (ObjectType::WorkingSet, 256),
        (ObjectType::DataMask, 2),
        (ObjectType::AlarmMask, 3),
        (ObjectType::Button, 100),
        (ObjectType::OutputString, 101),
        (ObjectType::Container, 200),
        (ObjectType::OutputString, 201),
        (ObjectType::FontAttributes, 102),
        (ObjectType::FontAttributes, 103),
        (ObjectType::NumberVariable, 300), // intentionally unused
    ] {
        pool.add(object_with_id(object_type, id));
    }
    if let Some(Object::WorkingSet(working_set)) = pool.object_mut_by_id(object_id(256)) {
        working_set.active_mask = object_id(2);
        working_set.object_refs.push(ObjectRef {
            id: object_id(201),
            offset: Point::default(),
        });
        working_set.language_codes = vec!["en".into(), "nl".into(), "de".into()];
    }
    if let Some(Object::Button(button)) = pool.object_mut_by_id(object_id(100)) {
        button.width = 148;
        button.height = 60;
        button.background_colour = 12;
    }
    if let Some(Object::Container(container)) = pool.object_mut_by_id(object_id(200)) {
        container.width = 148;
        container.height = 60;
    }
    if let Some(Object::OutputString(label)) = pool.object_mut_by_id(object_id(101)) {
        label.width = 140;
        label.height = 20;
        label.value = "Primary\n\"caption\"".into();
        label.font_attributes = object_id(102).into();
    }
    if let Some(Object::OutputString(label)) = pool.object_mut_by_id(object_id(201)) {
        label.width = 140;
        label.height = 20;
        label.value = "Alarm".into();
        label.font_attributes = object_id(102).into();
    }
    push_child(&mut pool, 2, 100, 10, 20);
    push_child(&mut pool, 3, 200, 20, 20);
    push_child(&mut pool, 100, 101, 0, 8);
    push_child(&mut pool, 200, 201, 0, 8);

    let mut project = EditorProject::from(pool);
    project.mask_size = 500;
    for (id, name) in [(2, "Main"), (3, "Alarm")] {
        assert!(project.rename_object(object_id(id), name.to_owned()));
    }

    // Capture one placed object into a reusable definition, then add two
    // instances with stable compiled IDs, distinct overrides, and explicit
    // placement order. The second mask retains its ordinary object graph.
    let plan = plan_component_capture(&project, 100, "Primary Button".into()).unwrap();
    let component_id = plan.definition.id;
    let first =
        ComponentService::create_component_from_selection(&mut project, plan, false).unwrap();
    let definition = &project.component_source().definitions[&component_id];
    let text_node = definition
        .nodes
        .iter()
        .find(|node| node.object_type == ObjectType::OutputString)
        .unwrap()
        .id;
    let parameter_id = ComponentParameterId(Uuid::from_u128(0xabc));
    let value = PropertyKey::parse("value").unwrap();
    ComponentService::add_component_parameter(
        &mut project,
        component_id,
        ComponentParameter {
            id: parameter_id,
            name: "Caption".into(),
            description: Some("Primary button caption".into()),
            target_node: text_node,
            property: ComponentParameterTarget::Property(value),
            kind: ComponentParameterKind::Text,
            default_value: json!("Primary\n\"caption\""),
        },
    )
    .unwrap();
    let reference_parameter_id = ComponentParameterId(Uuid::from_u128(0xdef));
    let mut definition = project.component_source().definitions[&component_id].clone();
    let text_node = definition
        .nodes
        .iter_mut()
        .find(|node| node.id == text_node)
        .unwrap();
    let target_node = text_node.id;
    let font_attributes = PropertyKey::parse("font_attributes").unwrap();
    assert!(text_node
        .references
        .insert(
            font_attributes,
            ComponentReference::Parameter(reference_parameter_id),
        )
        .is_some());
    definition.parameters.push(ComponentParameter {
        id: reference_parameter_id,
        name: "Font attributes".into(),
        description: Some("Font attributes object".into()),
        target_node,
        property: ComponentParameterTarget::Property(font_attributes),
        kind: ComponentParameterKind::ObjectReference,
        default_value: json!(102),
    });
    ComponentService::edit_component_definition(&mut project, definition).unwrap();
    ComponentService::set_component_override(
        &mut project,
        first,
        parameter_id,
        json!("Ready\n\"now\""),
    )
    .unwrap();
    let second = ComponentService::instantiate_component(
        &mut project,
        component_id,
        2,
        170,
        20,
        std::collections::BTreeMap::from([
            (parameter_id, json!("Save")),
            (reference_parameter_id, json!(103)),
        ]),
    )
    .unwrap();
    let instances = &project.component_source().instances;
    let first_instance = instances
        .iter()
        .find(|instance| instance.id == first)
        .unwrap();
    let second_instance = instances
        .iter()
        .find(|instance| instance.id == second)
        .unwrap();
    assert!(first_instance.child_index.unwrap() < second_instance.child_index.unwrap());
    assert_eq!(
        first_instance.overrides.get(&reference_parameter_id),
        None,
        "the first instance uses the object-reference parameter default"
    );
    assert_eq!(
        second_instance.overrides.get(&reference_parameter_id),
        Some(&json!(103))
    );
    let source = GraphicService::upload(&mut project, "spare graphic.png", tiny_png()).unwrap();
    (project, parameter_id, source)
}

fn tiny_png() -> Vec<u8> {
    let image = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

fn temporary_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn extract(archive: &[u8], destination: &Path) {
    let mut zip = ZipArchive::new(Cursor::new(archive)).unwrap();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        let relative = entry.enclosed_name().unwrap().to_path_buf();
        let path = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(path).unwrap();
        } else {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            fs::write(path, bytes).unwrap();
        }
    }
}

fn run_build(script: &Path, cwd: &Path) -> Output {
    Command::new("bash")
        .arg(script)
        .current_dir(cwd)
        .env("AITD", env!("CARGO_BIN_EXE_aitd"))
        .output()
        .expect("run exported build.sh")
}

#[test]
fn exported_zip_rebuilds_components_masks_shared_resources_and_iop_from_another_directory() {
    let (mut project, parameter_id, _asset_id) = source_project();
    let fixed_id = object_id(300);
    assert!(project.set_object_id_overridden(fixed_id, true));
    let instance_source = project.component_source();
    assert!(instance_source.instances.len() >= 2);
    assert!(instance_source
        .instances
        .iter()
        .all(|instance| !instance.compiled_ids.is_empty()));
    assert!(instance_source.instances.iter().any(|instance| {
        instance.overrides.get(&parameter_id) == Some(&json!("Ready\n\"now\""))
    }));

    let archive = project_zip::generate(&project).expect("generate readable CLI ZIP");
    let bytes = archive.to_zip().unwrap();
    let repeated = project_zip::generate(&project).unwrap();
    assert_eq!(
        archive.files, repeated.files,
        "archive entries should be deterministic"
    );
    assert_eq!(
        bytes,
        repeated.to_zip().unwrap(),
        "ZIP serialization should be deterministic"
    );
    assert!(archive
        .files
        .keys()
        .any(|path| path.starts_with("masks/Main/")));
    assert!(archive
        .files
        .keys()
        .any(|path| path.starts_with("masks/Alarm/")));
    assert!(archive.files.keys().any(|path| path.starts_with("shared/")));
    assert!(archive
        .files
        .keys()
        .any(|path| path.starts_with("components/")));
    assert!(archive
        .files
        .contains_key("assets/graphics/spare_graphic.png"));

    let root = temporary_directory("aitd-cli-zip");
    let extracted = root.join("bundle");
    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&extracted).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    extract(&bytes, &extracted);
    let script = extracted.join("build.sh");
    let script_text = fs::read_to_string(&script).unwrap();
    let settings = std::str::from_utf8(&archive.files["shared/project_settings.aitd"]).unwrap();
    assert!(
        settings.contains("id_range."),
        "the initial settings file must install automatic ID ranges"
    );
    assert!(script_text.find("project_settings.aitd").is_some());

    let output = run_build(&script, &elsewhere);
    assert!(
        output.status.success(),
        "build.sh failed from another cwd\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let built_project = extracted.join("build/project.aitp");
    let built_iop = extracted.join("build/project.iop");
    assert!(built_project.is_file());
    assert!(built_iop.is_file());
    let rebuilt = EditorProject::load_project(fs::read(&built_project).unwrap()).unwrap();
    assert_eq!(
        rebuilt.get_pool().objects().len(),
        project.get_pool().objects().len()
    );
    assert_eq!(
        rebuilt.component_source().instances.len(),
        project.component_source().instances.len()
    );
    assert_eq!(
        rebuilt.component_source().definitions.len(),
        project.component_source().definitions.len()
    );
    assert!(rebuilt.get_pool().object_by_id(fixed_id).is_some());
    assert!(
        rebuilt
            .get_object_info(rebuilt.get_pool().object_by_id(fixed_id).unwrap())
            .id_overridden
    );
    assert_eq!(fs::read(&built_iop).unwrap(), rebuilt.export_iop().unwrap());

    // The script must keep existing build outputs intact on a second run.
    let existing_project = fs::read(&built_project).unwrap();
    let second_run = run_build(&script, &elsewhere);
    assert!(!second_run.status.success());
    assert_eq!(fs::read(&built_project).unwrap(), existing_project);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exported_zip_replays_numeric_graphic_enums() {
    use ag_iso_stack::object_pool::vt_version::VtVersion;

    let mut pool = ObjectPool::default();
    for (object_type, id) in [
        (ObjectType::OutputPolygon, 100),
        (ObjectType::OutputPolygon, 101),
        (ObjectType::OutputEllipse, 102),
        (ObjectType::FillAttributes, 103),
        (ObjectType::ScaledGraphic, 104),
        (ObjectType::LineAttributes, 105),
    ] {
        pool.add(object_with_id(object_type, id));
    }
    if let Some(Object::OutputPolygon(polygon)) = pool.object_mut_by_id(object_id(100)) {
        polygon.line_attributes = object_id(105).into();
        polygon.width = 40;
        polygon.height = 30;
        polygon.points = vec![
            Point { x: 0, y: 0 },
            Point { x: 40, y: 0 },
            Point { x: 20, y: 30 },
        ];
        polygon.polygon_type = 0;
    }
    if let Some(Object::OutputPolygon(polygon)) = pool.object_mut_by_id(object_id(101)) {
        polygon.line_attributes = object_id(105).into();
        polygon.width = 40;
        polygon.height = 30;
        polygon.points = vec![
            Point { x: 0, y: 0 },
            Point { x: 40, y: 0 },
            Point { x: 20, y: 30 },
        ];
        polygon.polygon_type = 3;
    }
    if let Some(Object::OutputEllipse(ellipse)) = pool.object_mut_by_id(object_id(102)) {
        ellipse.line_attributes = object_id(105).into();
        ellipse.width = 40;
        ellipse.height = 30;
        ellipse.ellipse_type = 3;
    }
    if let Some(Object::FillAttributes(fill)) = pool.object_mut_by_id(object_id(103)) {
        fill.fill_type = 2;
    }
    if let Some(Object::ScaledGraphic(graphic)) = pool.object_mut_by_id(object_id(104)) {
        graphic.width = 40;
        graphic.height = 30;
        // Scaling 4, horizontal justification 2, vertical justification 1.
        graphic.scale_type = 4 | (2 << 3) | (1 << 5);
    }

    let mut working_set = object_with_id(ObjectType::WorkingSet, 256);
    if let Object::WorkingSet(ws) = &mut working_set {
        ws.active_mask = object_id(2);
        ws.object_refs.push(ObjectRef {
            id: object_id(200),
            offset: Point::default(),
        });
    }
    pool.add(working_set);
    pool.add(object_with_id(ObjectType::DataMask, 2));
    let mut identifier = object_with_id(ObjectType::PictureGraphic, 200);
    if let Object::PictureGraphic(picture) = &mut identifier {
        picture.width = 8;
        picture.actual_width = 8;
        picture.actual_height = 1;
        picture.data = vec![0];
    }
    pool.add(identifier);
    let mut project = EditorProject::from(pool);
    project.set_vt_version(VtVersion::Version6).unwrap();
    // This test compares protocol IDs after rebuilding, so reserve them explicitly.
    for id in 100..=105 {
        project.queue_object_id_assignment(object_id(id), object_id(id), true);
    }
    assert!(project.commit_queued("Reserve fixture IDs"));
    let archive = project_zip::generate(&project).expect("generate ZIP with numeric enum values");
    let root = temporary_directory("aitd-cli-zip-numeric-enums");
    let extracted = root.join("bundle");
    fs::create_dir_all(&extracted).unwrap();
    extract(&archive.to_zip().unwrap(), &extracted);

    let output = run_build(&extracted.join("build.sh"), &root);
    assert!(
        output.status.success(),
        "build.sh failed for numeric enum values\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let rebuilt =
        EditorProject::load_project(fs::read(extracted.join("build/project.aitp")).unwrap())
            .expect("load rebuilt project");
    let pool = rebuilt.get_pool();
    let polygon_type = |id| match pool.object_by_id(object_id(id)).unwrap() {
        Object::OutputPolygon(polygon) => {
            assert_eq!(polygon.points.len(), 3);
            polygon.polygon_type
        }
        object => panic!("expected polygon, got {:?}", object.object_type()),
    };
    assert_eq!(polygon_type(100), 0);
    assert_eq!(polygon_type(101), 3);
    match pool.object_by_id(object_id(102)).unwrap() {
        Object::OutputEllipse(ellipse) => assert_eq!(ellipse.ellipse_type, 3),
        object => panic!("expected ellipse, got {:?}", object.object_type()),
    }
    match pool.object_by_id(object_id(103)).unwrap() {
        Object::FillAttributes(fill) => assert_eq!(fill.fill_type, 2),
        object => panic!("expected fill attributes, got {:?}", object.object_type()),
    }
    match pool.object_by_id(object_id(104)).unwrap() {
        Object::ScaledGraphic(graphic) => {
            assert_eq!(graphic.scale_type, 4 | (2 << 3) | (1 << 5));
        }
        object => panic!("expected scaled graphic, got {:?}", object.object_type()),
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_exported_command_does_not_publish_build_artifacts() {
    let (project, _, _) = source_project();
    let mut archive = project_zip::generate(&project).unwrap();
    let (_, command) = archive
        .files
        .iter_mut()
        .find(|(path, _)| path.ends_with(".aitd"))
        .expect("exported command file");
    *command = b"not-a-valid-aitd-command\n".to_vec();

    let root = temporary_directory("aitd-cli-zip-failure");
    let extracted = root.join("bundle");
    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&extracted).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    extract(&archive.to_zip().unwrap(), &extracted);
    let output = run_build(&extracted.join("build.sh"), &elsewhere);
    assert!(!output.status.success());
    assert!(
        !extracted.join("build").exists(),
        "failed command must not publish build artifacts"
    );
    fs::remove_dir_all(root).unwrap();
}
