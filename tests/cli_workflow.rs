//! Standalone CLI workflows share a temporary project within one calling process.
#![cfg(not(target_arch = "wasm32"))]

use ag_iso_stack::object_pool::{ObjectPool, ObjectType};
use ag_iso_terminal_designer::{default_object, EditorProject};
use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
    sync::Mutex,
};

// CLI sessions are keyed by the calling shell/process, so serialize tests that
// invoke the binary directly from this test harness process.
static CLI_SESSION: Mutex<()> = Mutex::new(());

fn incompatible_project_bytes() -> Vec<u8> {
    let mut pool = ObjectPool::default();
    let mut unsupported = default_object(ObjectType::OutputList);
    unsupported.mut_id().set_value(100).unwrap();
    pool.add(unsupported);
    for (kind, id) in [
        (ObjectType::WorkingSet, 200),
        (ObjectType::DataMask, 201),
        (ObjectType::PictureGraphic, 202),
    ] {
        let mut object = default_object(kind);
        object.mut_id().set_value(id).unwrap();
        match &mut object {
            ag_iso_stack::object_pool::object::Object::WorkingSet(ws) => {
                ws.active_mask = ag_iso_stack::object_pool::ObjectId::new(201).unwrap();
                ws.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
                    id: ag_iso_stack::object_pool::ObjectId::new(202).unwrap(),
                    offset: Default::default(),
                });
            }
            ag_iso_stack::object_pool::object::Object::PictureGraphic(picture) => {
                picture.width = 8;
                picture.actual_width = 8;
                picture.actual_height = 1;
                picture.data = vec![0];
            }
            _ => {}
        }
        pool.add(object);
    }
    let project = EditorProject::from(pool);
    let mut saved: Value = serde_json::from_slice(&project.save_project().unwrap()).unwrap();
    saved["settings"]["vt_version"] = 2.into();
    serde_json::to_vec(&saved).unwrap()
}

fn cli(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aitd"))
        .current_dir(cwd)
        .args(["--json"])
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["success"], true, "{value}");
    assert!(value.get("project_instance_id").is_none(), "{value}");
    assert!(value.get("revision").is_none(), "{value}");
    value
}

fn failure(output: Output) -> Value {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    serde_json::from_slice(&output.stderr).unwrap()
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    path
}

fn new_session(cwd: &Path) {
    success(cli(cwd, &["project", "new", "--discard"]));
}

fn add_identifier(cwd: &Path, working_set: &str) {
    success(cli(
        cwd,
        &["object", "create", "OutputString", "Identifier"],
    ));
    success(cli(
        cwd,
        &["object", "create", "FontAttributes", "IdentifierFont"],
    ));
    success(cli(
        cwd,
        &[
            "reference",
            "set",
            "Identifier",
            "font_attributes",
            "IdentifierFont",
        ],
    ));
    success(cli(
        cwd,
        &["placement", "add", working_set, "Identifier", "0", "0"],
    ));
}

fn get(cwd: &Path, target: &str) -> Value {
    success(cli(cwd, &["object", "get", target]))
}

#[test]
fn graphic_conversion_cli_accepts_configurable_mappings_without_encoding_flags() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-graphic-conversion");
    new_session(&cwd);
    success(cli(
        &cwd,
        &["object", "create", "PictureGraphic", "Picture"],
    ));

    let image = image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255]).unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    std::fs::write(cwd.join("pixel.png"), bytes.into_inner()).unwrap();
    success(cli(
        &cwd,
        &["graphic", "import", "pixel.png", "--name", "Pixel"],
    ));
    success(cli(
        &cwd,
        &[
            "graphic",
            "assign",
            "Pixel",
            "Picture",
            "--format",
            "four_bit",
            "--transparency",
            "flatten",
            "--map",
            "rgba:FF0000FF=1",
        ],
    ));
    let saved_path = cwd.join("converted.aitp");
    success(cli(
        &cwd,
        &[
            "project",
            "save",
            saved_path.to_str().unwrap(),
            "--skip-validation",
        ],
    ));
    let saved = EditorProject::load_project(std::fs::read(saved_path).unwrap()).unwrap();
    let ag_iso_stack::object_pool::object::Object::PictureGraphic(template) =
        default_object(ObjectType::PictureGraphic)
    else {
        unreachable!()
    };
    use ag_iso_terminal_designer::graphics::conversion::{
        self, ColourMapping, ConversionOptions, SourceColour, TransparencyMode,
    };
    let palette = ag_iso_terminal_designer::effective_colour::colours(saved.get_pool())
        .map(|colour| colour.as_rgb());
    let mut options = ConversionOptions::suggested_with_palette(
        ag_iso_terminal_designer::graphics::PictureGraphicFormat::FourBit,
        &palette,
    );
    options.transparency = TransparencyMode::Flatten;
    options.mappings.insert(
        SourceColour::Rgba([255, 0, 0, 255]),
        ColourMapping::Index(1),
    );
    let preview = conversion::preview_image(
        &template,
        &std::fs::read(cwd.join("pixel.png")).unwrap(),
        &options,
        &palette,
    )
    .unwrap();
    let converted = saved.get_pool().objects_by_type(ObjectType::PictureGraphic)[0];
    let ag_iso_stack::object_pool::object::Object::PictureGraphic(converted) = converted else {
        unreachable!()
    };
    assert_eq!(converted.data, preview.picture.data);
    assert_eq!(converted.format, preview.picture.format);
    assert_eq!(converted.options, preview.picture.options);
    success(cli(
        &cwd,
        &["graphic", "import", "pixel.png", "--name", "NewPixel"],
    ));
    success(cli(
        &cwd,
        &[
            "graphic",
            "create-picture",
            "NewPixel",
            "--format",
            "four_bit",
            "--map",
            "rgba:FF0000FF=1",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "graphic",
            "convert",
            "Pixel",
            "--format",
            "monochrome",
            "--mapping",
            "luminance_threshold",
            "--threshold",
            "127",
            "--invert",
        ],
    ));
    let invalid = failure(cli(
        &cwd,
        &[
            "graphic",
            "convert",
            "Pixel",
            "--format",
            "four_bit",
            "--mapping",
            "luminance_threshold",
        ],
    ));
    assert!(invalid["error"]["message"]
        .as_str()
        .unwrap()
        .contains("only available for monochrome output"));
    let help = cli(&cwd, &["graphic", "convert", "--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--format"));
    assert!(help.contains("--map"));
    assert!(!help.contains("--encoding"));
    assert!(!help.contains("--dither"));

    success(cli(&cwd, &["project", "close", "--discard"]));
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn direct_edits_are_typed_and_check_does_not_commit() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-direct-edit");
    new_session(&cwd);

    let status = success(cli(&cwd, &["status"]));
    assert_eq!(status["result"]["loaded"], true);
    let capabilities = success(cli(&cwd, &["capabilities"]));
    assert_eq!(capabilities["result"]["json_input"], false);

    success(cli(
        &cwd,
        &["object", "create", "NumberVariable", "Counter"],
    ));
    success(cli(&cwd, &["object", "set", "Counter", "value=42"]));
    success(cli(
        &cwd,
        &["object", "create", "StringVariable", "Literal"],
    ));
    success(cli(
        &cwd,
        &["object", "set", "Literal", "value={\"enabled\":true}"],
    ));
    assert_eq!(
        get(&cwd, "Literal")["result"]["objects"][0]["properties"]["value"],
        "{\"enabled\":true}"
    );

    success(cli(&cwd, &["object", "create", "OutputPolygon", "Shape"]));
    success(cli(
        &cwd,
        &["object", "field", "Shape", "points.0", "x=12", "y=20"],
    ));
    success(cli(
        &cwd,
        &[
            "object", "array", "append", "Shape", "points", "x=30", "y=40",
        ],
    ));
    let shape = get(&cwd, "Shape");
    assert_eq!(
        shape["result"]["objects"][0]["properties"]["points"][0]["x"],
        12
    );
    assert_eq!(
        shape["result"]["objects"][0]["properties"]["points"][3]["x"],
        30
    );

    success(cli(
        &cwd,
        &[
            "object",
            "create",
            "NumberVariable",
            "Explicit",
            "--override-id",
            "6100",
        ],
    ));
    assert_eq!(get(&cwd, "Explicit")["result"]["objects"][0]["id"], 6100);
    let duplicate_id = failure(cli(
        &cwd,
        &[
            "object",
            "create",
            "NumberVariable",
            "Duplicate",
            "--override-id",
            "6100",
        ],
    ));
    assert!(duplicate_id["error"]["message"].as_str().is_some());

    let checked = success(cli(
        &cwd,
        &["--check", "object", "set", "Counter", "value=77"],
    ));
    assert_eq!(checked["success"], true);
    assert_eq!(
        get(&cwd, "Counter")["result"]["objects"][0]["properties"]["value"],
        42
    );
    let malformed = failure(cli(
        &cwd,
        &["object", "set", "Counter", "value={\"not\":\"a number\"}"],
    ));
    assert!(malformed["error"]["message"].as_str().is_some());
    assert_eq!(
        get(&cwd, "Counter")["result"]["objects"][0]["properties"]["value"],
        42
    );
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn command_files_are_atomic_and_share_state_between_directories() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let root = temp_dir("aitd-command-file");
    let first = root.join("first");
    let second = root.join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    new_session(&first);
    success(cli(
        &first,
        &["object", "create", "NumberVariable", "Counter"],
    ));
    success(cli(&first, &["object", "set", "Counter", "value=1"]));

    std::fs::write(
        first.join("valid.aitcmd"),
        "object set Counter value=2\nproject settings set mask_size=640\n",
    )
    .unwrap();
    success(cli(&first, &["--check", "run", "--file", "valid.aitcmd"]));
    assert_eq!(
        get(&second, "Counter")["result"]["objects"][0]["properties"]["value"],
        1
    );

    success(cli(&first, &["run", "--file", "valid.aitcmd"]));
    let counter = get(&second, "Counter");
    assert_eq!(counter["result"]["objects"][0]["properties"]["value"], 2);
    assert_eq!(
        success(cli(&second, &["project", "settings", "show"]))["result"]["settings"]["mask_size"],
        640
    );

    std::fs::write(
        first.join("invalid.aitcmd"),
        "object set Counter value=3\nproject settings set mask_size=0\n",
    )
    .unwrap();
    let failed = failure(cli(&first, &["run", "--file", "invalid.aitcmd"]));
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("line 2"));
    assert_eq!(
        get(&second, "Counter")["result"]["objects"][0]["properties"]["value"],
        2
    );
    assert_eq!(
        success(cli(&second, &["project", "settings", "show"]))["result"]["settings"]["mask_size"],
        640
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_save_is_explicit_and_validation_can_be_skipped() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-save");
    new_session(&cwd);
    let project_path = cwd.join("machine.aitp");
    let path = project_path.to_str().unwrap();
    let fixture_bytes = incompatible_project_bytes();
    std::fs::write(&project_path, &fixture_bytes).unwrap();
    success(cli(&cwd, &["project", "load", path, "--discard"]));
    success(cli(
        &cwd,
        &["object", "create", "NumberVariable", "Counter"],
    ));
    success(cli(&cwd, &["object", "set", "Counter", "value=42"]));
    let close_dirty = failure(cli(&cwd, &["project", "close"]));
    assert_eq!(close_dirty["error"]["code"], "PROJECT_FAILED");
    assert_eq!(std::fs::read(&project_path).unwrap(), fixture_bytes);
    let validation = failure(cli(&cwd, &["validate"]));
    assert_eq!(validation["result"]["valid"], false);
    let blocked = failure(cli(&cwd, &["project", "save", path]));
    assert_eq!(blocked["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(std::fs::read(&project_path).unwrap(), fixture_bytes);
    let skipped = success(cli(&cwd, &["project", "save", path, "--skip-validation"]));
    let backup = skipped["result"]["backup"].as_str().unwrap();
    assert_eq!(std::fs::read(backup).unwrap(), fixture_bytes);
    let saved_bytes = std::fs::read(&project_path).unwrap();

    success(cli(&cwd, &["project", "close"]));
    let closed = failure(cli(&cwd, &["object", "list"]));
    assert!(closed["error"]["message"].as_str().is_some());
    success(cli(&cwd, &["project", "load", path]));
    assert_eq!(
        get(&cwd, "Counter")["result"]["objects"][0]["properties"]["value"],
        42
    );

    let externally_changed = [saved_bytes.as_slice(), b"external edit"].concat();
    std::fs::write(&project_path, &externally_changed).unwrap();
    success(cli(
        &cwd,
        &["object", "create", "NumberVariable", "Another"],
    ));
    let stale_source = failure(cli(&cwd, &["project", "save", path, "--skip-validation"]));
    assert_eq!(stale_source["error"]["code"], "SOURCE_CHANGED");
    assert_eq!(std::fs::read(&project_path).unwrap(), externally_changed);
    assert_eq!(
        get(&cwd, "Another")["result"]["objects"][0]["name"],
        "Another"
    );

    // Restore the loaded source bytes, then repair the unsupported VT version.
    std::fs::write(&project_path, &saved_bytes).unwrap();
    success(cli(&cwd, &["project", "settings", "set", "vt_version=6"]));
    assert_eq!(success(cli(&cwd, &["validate"]))["result"]["valid"], true);
    let saved = success(cli(&cwd, &["project", "save", path]));
    let backup = saved["result"]["backup"].as_str().unwrap();
    assert_eq!(std::fs::read(backup).unwrap(), saved_bytes);
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn separate_shells_have_independent_sessions() {
    let cwd = temp_dir("aitd-shell-isolation");
    let binary = env!("CARGO_BIN_EXE_aitd");
    let script = cwd.join("create.sh");
    std::fs::write(
        &script,
        "#!/usr/bin/env bash\nset -e\n\"$AITD\" --json project new --discard\n\
         \"$AITD\" --json object create NumberVariable Private\n:\n",
    )
    .unwrap();
    let create = Command::new("bash")
        .current_dir(&cwd)
        .arg(&script)
        .env("AITD", binary)
        .output();
    let create = create.unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let output = Command::new("bash")
        .current_dir(&cwd)
        .args(["-c", "\"$AITD\" --json status; :"])
        .env("AITD", binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["result"]["loaded"], false, "{status}");
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn project_export_publishes_validated_artifacts_and_refuses_existing_outputs() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-project-export");
    new_session(&cwd);
    success(cli(&cwd, &["object", "create", "WorkingSet", "Pool"]));
    success(cli(&cwd, &["object", "create", "DataMask", "Screen"]));
    success(cli(
        &cwd,
        &["reference", "set", "Pool", "active_mask", "Screen"],
    ));
    add_identifier(&cwd, "Pool");
    success(cli(
        &cwd,
        &[
            "object",
            "create",
            "NumberVariable",
            "Counter",
            "--override-id",
            "6100",
        ],
    ));
    success(cli(&cwd, &["object", "set", "Counter", "value=42"]));
    success(cli(&cwd, &["export", "project", "--output-dir", "build"]));
    let saved = std::fs::read(cwd.join("build/project.aitp")).unwrap();
    let restored = EditorProject::load_project(saved.clone()).unwrap();
    assert_eq!(
        std::fs::read(cwd.join("build/project.iop")).unwrap(),
        restored.export_iop().unwrap()
    );
    let rejected = failure(cli(&cwd, &["export", "project", "--output-dir", "build"]));
    assert_eq!(rejected["error"]["code"], "OUTPUT_EXISTS");
    assert_eq!(
        std::fs::read(cwd.join("build/project.aitp")).unwrap(),
        saved
    );
    std::fs::create_dir(cwd.join("empty")).unwrap();
    assert_eq!(
        failure(cli(&cwd, &["export", "project", "--output-dir", "empty"]))["error"]["code"],
        "OUTPUT_EXISTS"
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("missing", cwd.join("link")).unwrap();
        assert_eq!(
            failure(cli(&cwd, &["export", "project", "--output-dir", "link"]))["error"]["code"],
            "OUTPUT_EXISTS"
        );
    }
    std::fs::write(cwd.join("invalid.aitp"), incompatible_project_bytes()).unwrap();
    success(cli(&cwd, &["project", "load", "invalid.aitp", "--discard"]));
    assert_eq!(
        failure(cli(&cwd, &["export", "project", "--output-dir", "failed"]))["error"]["code"],
        "VALIDATION_FAILED"
    );
    assert!(!cwd.join("failed").exists());
    assert!(std::fs::read_dir(&cwd).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".aitd-export-")));
    success(cli(&cwd, &["project", "close", "--discard"]));
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn region_reports_each_placement_of_a_reused_container_at_its_own_position() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-region-reuse");
    new_session(&cwd);
    for command in [
        "object create DataMask Screen",
        "object create Container Box",
        "object set Box width=50 height=50",
        "object create Container Dot",
        "object set Dot width=5 height=5",
        "placement add Box Dot 10 0",
        "placement add Screen Box 0 0",
        "placement add Screen Box 100 0",
    ] {
        success(cli(&cwd, &command.split_whitespace().collect::<Vec<_>>()));
    }
    let region = success(cli(&cwd, &["region", "Screen", "--region", "all"]));
    let mut dots: Vec<_> = region["result"]["region"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|object| object["name"] == "Dot")
        .map(|object| object["absolute"]["x"].as_i64().unwrap())
        .collect();
    dots.sort();
    assert_eq!(dots, [10, 110]);
}

#[test]
fn object_list_finds_unused_objects_and_selects_columns() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-list-unused");
    new_session(&cwd);
    for command in [
        "object create WorkingSet WS",
        "object create DataMask Main",
        "object create DataMask Spare",
        "object create DataMask Leftover",
        "object create Button Go",
        "object create Macro GoSpare",
        "object create Macro Orphan",
        "object create ObjectPointer Ptr",
        "object create Container Pointed",
        "object create Container Swapped",
        "object create FontAttributes UnusedFont",
        "reference set WS active_mask Main",
        "placement add Main Go 0 0",
        "placement add Main Ptr 0 0",
        "reference set Ptr value Pointed",
        "macro append GoSpare change-active-mask WS Spare",
        "macro append GoSpare change-numeric-value Ptr Swapped",
        "event bind Go OnKeyPress=GoSpare",
    ] {
        success(cli(&cwd, &command.split_whitespace().collect::<Vec<_>>()));
    }

    let unused = success(cli(&cwd, &["object", "list", "--unused"]));
    let mut names: Vec<_> = unused["result"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|object| object["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    // The Working Set is top-level; parents, pointer values, event macros and
    // macro command targets all count as references.
    assert_eq!(names, ["Leftover", "Orphan", "UnusedFont"]);
    assert!(unused["result"].get("unresolved_macros").is_none());

    let listed = success(cli(
        &cwd,
        &[
            "object",
            "list",
            "--query",
            "Swapped",
            "--columns",
            "name,id,c_ident",
        ],
    ));
    assert_eq!(
        listed["result"]["columns"],
        serde_json::json!(["name", "id", "c_ident"])
    );
    let row = &listed["result"]["objects"][0];
    assert_eq!(row["name"], "Swapped");
    assert_eq!(row["c_ident"], "SWAPPED");
    assert!(row["id"].is_u64());
    assert!(row.get("type").is_none());

    // Readable output keeps the requested order and the ID column.
    let output = Command::new(env!("CARGO_BIN_EXE_aitd"))
        .current_dir(&cwd)
        .args([
            "object",
            "list",
            "--query",
            "Swapped",
            "--columns",
            "c_ident,id",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    let id = row["id"].as_u64().unwrap();
    assert!(text.contains("  c_ident  id\n"), "{text}");
    assert!(text.contains(&format!("  SWAPPED  {id}\n")), "{text}");

    let error = failure(cli(&cwd, &["object", "list", "--columns", "name,bogus"]));
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENTS");
}

#[test]
fn object_get_shows_allowed_pointer_targets_from_both_sides() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-pointer-targets");
    new_session(&cwd);
    for command in [
        "object create ObjectPointer Selector",
        "object create Container Right",
        "object create Container Left",
        "pointer set Selector Left Right",
    ] {
        success(cli(&cwd, &command.split_whitespace().collect::<Vec<_>>()));
    }
    let names = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let pointer = get(&cwd, "Selector");
    assert_eq!(
        names(&pointer["result"]["objects"][0]["allowed_targets"]),
        ["Left", "Right"]
    );
    let target = get(&cwd, "Left");
    assert_eq!(
        names(&target["result"]["objects"][0]["allowed_by"]),
        ["Selector"]
    );

    let context = success(cli(&cwd, &["context", "--scope", "pool"]));
    let selector = context["result"]["context"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|object| object["name"] == "Selector")
        .unwrap();
    assert_eq!(names(&selector["allowed_targets"]), ["Left", "Right"]);

    // An allowed target may be shown at runtime, so it is not unused.
    let unused = success(cli(&cwd, &["object", "list", "--unused"]));
    let unused = names(&unused["result"]["objects"]);
    assert!(!unused.contains(&"Left".to_owned()), "{unused:?}");
}

#[test]
fn deleting_a_picture_graphic_removes_its_recovered_image() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-delete-picture");
    new_session(&cwd);
    let image = image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255]).unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    std::fs::write(cwd.join("pixel.png"), bytes.into_inner()).unwrap();
    for name in ["Part", "Kept"] {
        success(cli(
            &cwd,
            &["graphic", "import", "pixel.png", "--name", name],
        ));
        success(cli(&cwd, &["graphic", "create-picture", name]));
    }
    let assets = |cwd: &Path| -> Vec<String> {
        let list = success(cli(cwd, &["graphic", "list", "--status", "available"]));
        list["result"]["graphics"]["graphics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(assets(&cwd).is_empty());

    let deleted = success(cli(&cwd, &["object", "delete", "Part"]));
    assert_eq!(
        deleted["result"]["removed_assets"],
        serde_json::json!(["Part"])
    );
    assert!(assets(&cwd).is_empty());

    let kept = success(cli(&cwd, &["object", "delete", "Kept", "--keep-image"]));
    assert_eq!(
        kept["result"]["recovered_assets"],
        serde_json::json!(["Kept"])
    );
    assert_eq!(assets(&cwd), ["Kept"]);
}

#[test]
fn check_reports_the_objects_and_names_a_real_run_would_change() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-check-changes");
    new_session(&cwd);
    success(cli(&cwd, &["object", "create", "DataMask", "Home Screen"]));
    let id = get(&cwd, "Home Screen")["result"]["objects"][0]["id"].clone();

    let checked = success(cli(
        &cwd,
        &["--check", "object", "rename", "Home Screen", "Home screen"],
    ));
    assert_eq!(checked["result"]["changed_ids"], serde_json::json!([id]));
    assert_eq!(
        checked["result"]["names"][id.to_string()],
        "Home screen",
        "{checked}"
    );
    assert!(checked["result"]["transaction_id"].is_null());
    // The check leaves the project unchanged.
    let listed = success(cli(&cwd, &["object", "list"]));
    assert_eq!(listed["result"]["objects"][0]["name"], "Home Screen");

    let created = success(cli(
        &cwd,
        &["--check", "object", "create", "Container", "Box"],
    ));
    assert_eq!(
        created["result"]["allocated_ids"],
        created["result"]["changed_ids"]
    );
    assert_eq!(
        created["result"]["allocated_ids"].as_array().unwrap().len(),
        1
    );

    let applied = success(cli(
        &cwd,
        &["object", "rename", "Home Screen", "Home screen"],
    ));
    assert_eq!(
        applied["result"]["changed_ids"],
        checked["result"]["changed_ids"]
    );
}

#[test]
fn component_objects_get_instance_header_names_and_capture_reports_the_instance() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-component-header");
    new_session(&cwd);
    for command in [
        "object create WorkingSet WS",
        "object create DataMask Main",
        "object create Button Base",
        "object create OutputString Value",
        "object create FontAttributes Font",
        "reference set WS active_mask Main",
        "object set Base width=148 height=60",
        "object set Value width=140 height=20",
        "reference set Value font_attributes Font",
        "placement add Base Value 0 8",
        "placement add Main Base 10 20",
    ] {
        success(cli(&cwd, &command.split_whitespace().collect::<Vec<_>>()));
    }

    let checked = success(cli(
        &cwd,
        &["--check", "component", "capture", "Base", "Toggle button"],
    ));
    assert_eq!(
        checked["result"]["created_instances"],
        serde_json::json!(["Toggle button 1"])
    );
    let captured = success(cli(
        &cwd,
        &["component", "capture", "Base", "Toggle button"],
    ));
    assert_eq!(
        captured["result"]["created_instances"],
        serde_json::json!(["Toggle button 1"])
    );
    success(cli(
        &cwd,
        &[
            "component",
            "instance-rename",
            "Toggle button 1",
            "Panel one",
        ],
    ));
    let created = success(cli(
        &cwd,
        &[
            "component",
            "instance-create",
            "Panel two",
            "Toggle button",
            "Main",
            "200",
            "10",
        ],
    ));
    assert_eq!(
        created["result"]["created_instances"],
        serde_json::json!(["Panel two"])
    );
    let conflict = failure(cli(
        &cwd,
        &["component", "instance-rename", "Panel two", "panel-one"],
    ));
    assert_eq!(conflict["error"]["code"], "VALIDATION_FAILED");

    success(cli(&cwd, &["export", "header", "--output", "pool.h"]));
    let header = std::fs::read_to_string(cwd.join("pool.h")).unwrap();
    for identifier in ["PANEL_ONE_VALUE", "PANEL_TWO_VALUE"] {
        assert!(
            header.contains(&format!("#define {identifier} ")),
            "{header}"
        );
    }
    assert!(!header.contains("TOGGLE_BUTTON"), "{header}");
}

fn run_all(cwd: &Path, commands: &[&str]) {
    for command in commands {
        success(cli(cwd, &command.split_whitespace().collect::<Vec<_>>()));
    }
}

fn child_offsets(cwd: &Path, id: u64) -> Vec<(i64, i64)> {
    let object = success(cli(cwd, &["object", "get", &id.to_string(), "--children"]));
    // Omitted when the object has no children.
    object["result"]["objects"][0]["positioned_children"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|child| (child["x"].as_i64().unwrap(), child["y"].as_i64().unwrap()))
        .collect()
}

#[test]
fn capture_keeps_shared_references_and_names_parse_as_documented() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-capture-shared");
    new_session(&cwd);
    run_all(
        &cwd,
        &[
            "object create WorkingSet WS",
            "object create DataMask Main",
            "object create Container Gauge",
            "object create OutputNumber Speed",
            "object create NumberVariable CountVar",
            "object create NumberVariable OtherVar",
            "object create FontAttributes SharedFont",
            "object create FontAttributes TitleFont",
            "reference set WS active_mask Main",
            "object set Gauge width=100 height=40",
            "object set Speed width=80 height=20",
            "reference set Speed font_attributes SharedFont",
            "reference set Speed variable_reference CountVar",
            "placement add Gauge Speed 0 0",
            "placement add Main Gauge 0 0",
        ],
    );
    let refused = failure(cli(&cwd, &["component", "capture", "Gauge", "Gauge"]));
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(message.contains("--keep-shared-references"), "{message}");
    assert!(message.contains("CountVar"), "{message}");

    let captured = success(cli(
        &cwd,
        &[
            "component",
            "capture",
            "Gauge",
            "Gauge",
            "--keep-shared-references",
        ],
    ));
    assert_eq!(
        captured["result"]["created_instances"],
        serde_json::json!(["Gauge 1"])
    );
    // Shared objects stay single pool objects instead of being copied.
    let fonts = success(cli(&cwd, &["object", "list", "--type", "FontAttributes"]));
    assert_eq!(fonts["result"]["total"], 2);
    let speed_id = captured["result"]["changed_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_u64().unwrap())
        .find(|id| {
            get(&cwd, &id.to_string())["result"]["objects"][0]["object_type"] == "OutputNumber"
        })
        .expect("the captured OutputNumber is reachable by ID");
    let speed = get(&cwd, &speed_id.to_string());
    assert_eq!(
        speed["result"]["objects"][0]["references"]["font_attributes"]["name"],
        "SharedFont"
    );

    // node-reference accepts object:NAME; node:NAME only matches nodes.
    success(cli(
        &cwd,
        &[
            "component",
            "node-reference",
            "Gauge",
            "Speed",
            "font_attributes",
            "object:TitleFont",
        ],
    ));
    let speed = get(&cwd, &speed_id.to_string());
    assert_eq!(
        speed["result"]["objects"][0]["references"]["font_attributes"]["name"],
        "TitleFont"
    );
    failure(cli(
        &cwd,
        &[
            "component",
            "node-reference",
            "Gauge",
            "Speed",
            "font_attributes",
            "node:TitleFont",
        ],
    ));

    // override-set takes a plain object name, and still a JSON-quoted one.
    success(cli(
        &cwd,
        &[
            "component",
            "parameter-add",
            "Gauge",
            "Variable",
            "Speed",
            "variable_reference",
            "ObjectReference",
            "CountVar",
        ],
    ));
    for value in ["OtherVar", "\"CountVar\""] {
        success(cli(
            &cwd,
            &["component", "override-set", "Gauge 1", "Variable", value],
        ));
    }
    let speed = get(&cwd, &speed_id.to_string());
    assert_eq!(
        speed["result"]["objects"][0]["references"]["variable_reference"]["name"],
        "CountVar"
    );
}

#[test]
fn delete_unplaces_and_object_get_accepts_ids() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-delete-unplace");
    new_session(&cwd);
    run_all(
        &cwd,
        &[
            "object create DataMask Home",
            "object create DataMask Other",
            "object create Container Box",
            "object create Container 77",
            "placement add Home Box 0 0",
            "placement add Home Box 50 0",
            "placement add Other Box 0 0",
        ],
    );
    let refused = failure(cli(&cwd, &["object", "delete", "Box"]));
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(message.contains("placed in Home (2×)"), "{message}");
    assert!(message.contains("--unplace"), "{message}");
    success(cli(&cwd, &["object", "delete", "Box", "--unplace"]));
    let listed = success(cli(&cwd, &["object", "list", "--query", "Box"]));
    assert_eq!(listed["result"]["total"], 0);

    let run_id = get(&cwd, "Home")["result"]["objects"][0]["id"].clone();
    let by_id = get(&cwd, &run_id.to_string());
    assert_eq!(by_id["result"]["objects"][0]["name"], "Home");
    // A numeric name still wins over an ID.
    assert_eq!(get(&cwd, "77")["result"]["objects"][0]["name"], "77");
    let missing = failure(cli(&cwd, &["object", "get", "999"]));
    assert_eq!(missing["error"]["code"], "OBJECT_NOT_FOUND");
}

#[test]
fn a_component_parameter_can_position_a_child_per_instance() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-position-parameter");
    new_session(&cwd);
    run_all(
        &cwd,
        &[
            "object create WorkingSet WS",
            "object create DataMask Main",
            "object create Button Frame",
            "object create Container Icon",
            "reference set WS active_mask Main",
            "object set Frame width=148 height=60",
            "object set Icon width=20 height=20",
            "placement add Frame Icon 4 4",
            "placement add Main Frame 0 0",
            "component capture Frame Button",
            "component parameter-add Button ChildPos Icon position Position 10,8",
            "component instance-create Second Button Main 200 0",
            "component override-set Second ChildPos 30,12",
        ],
    );
    let instances = success(cli(&cwd, &["instance", "list"]));
    let mut offsets = Vec::new();
    for name in ["Button 1", "Second"] {
        let instance = success(cli(&cwd, &["instance", "get", name]));
        let root = instance["result"]["instances"][0]["compiled_ids"]
            .as_object()
            .and_then(|ids| {
                ids.values()
                    .map(|id| id.as_u64().unwrap())
                    .find(|id| !child_offsets(&cwd, *id).is_empty())
            })
            .unwrap_or_else(|| panic!("no root for {name}: {instances}"));
        offsets.push(child_offsets(&cwd, root));
    }
    assert_eq!(offsets, [vec![(10, 8)], vec![(30, 12)]]);

    // Moving the child in the master moves instances without an override.
    success(cli(
        &cwd,
        &[
            "component",
            "node-children-set",
            "Button",
            "Frame",
            "Icon:6:6",
        ],
    ));
    let instance = success(cli(&cwd, &["instance", "get", "Button 1"]));
    let root = instance["result"]["instances"][0]["compiled_ids"]
        .as_object()
        .unwrap()
        .values()
        .map(|id| id.as_u64().unwrap())
        .find(|id| !child_offsets(&cwd, *id).is_empty())
        .unwrap();
    assert_eq!(child_offsets(&cwd, root), [(6, 6)]);

    let root = failure(cli(
        &cwd,
        &[
            "component",
            "parameter-add",
            "Button",
            "RootPos",
            "Frame",
            "position",
            "Position",
            "0,0",
        ],
    ));
    assert_eq!(root["error"]["code"], "VALIDATION_FAILED");
}

fn placements_of(cwd: &Path, parent: &str) -> Vec<(u64, i64, i64)> {
    let object = success(cli(cwd, &["object", "get", parent, "--children"]));
    object["result"]["objects"][0]["positioned_children"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|child| {
            (
                child["child"]["id"].as_u64().unwrap(),
                child["x"].as_i64().unwrap(),
                child["y"].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn mask_size_errors_list_every_overflow_and_placement_shift_moves_all() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-mask-shift");
    new_session(&cwd);
    run_all(
        &cwd,
        &[
            "object create WorkingSet WS",
            "object create DataMask Home",
            "object create Container A",
            "object create Container B",
            "object create Button Frame",
            "object create Container Icon",
            "reference set WS active_mask Home",
            "object set A width=100 height=100",
            "object set B width=50 height=50",
            "object set Frame width=148 height=60",
            "object set Icon width=20 height=20",
            "placement add Home A 400 40",
            "placement add Home B 0 420",
            "placement add Home B 60 420",
            "placement add Frame Icon 4 4",
            "placement add Home Frame 300 300",
            "component capture Frame Toggle",
            "component instance-create Second Toggle Home 100 300",
        ],
    );

    // Every overflowing placement is listed with its size, not just the first.
    let refused = failure(cli(&cwd, &["project", "settings", "set", "mask_size=400"]));
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(message.contains("4 objects would be outside"), "{message}");
    assert!(
        message.contains("'A' (3000) 100×100 at (400, 40)"),
        "{message}"
    );
    assert!(
        message.contains("'B' (3001) 50×50 at (60, 420)"),
        "{message}"
    );
    assert!(message.contains("'Toggle 1/Frame'"), "{message}");

    let before = placements_of(&cwd, "Home");
    success(cli(
        &cwd,
        &["placement", "shift", "Home", "0", "-20", "--all"],
    ));
    let after = placements_of(&cwd, "Home");
    assert_eq!(after.len(), before.len());
    for ((id, x, y), (after_id, after_x, after_y)) in before.iter().zip(&after) {
        assert_eq!((id, *x, *y - 20), (after_id, *after_x, *after_y));
    }
    // Named children: an object (all its placements) and an instance.
    success(cli(
        &cwd,
        &["placement", "shift", "Home", "5", "0", "B", "Second"],
    ));
    let named = placements_of(&cwd, "Home");
    let moved: Vec<_> = after
        .iter()
        .zip(&named)
        .filter(|(old, new)| old != new)
        .map(|(old, new)| (new.1 - old.1, new.2 - old.2))
        .collect();
    assert_eq!(moved, [(5, 0), (5, 0), (5, 0)]);

    let usage = cli(&cwd, &["placement", "shift", "Home", "1", "1"]);
    assert!(!usage.status.success());
    let outside = failure(cli(
        &cwd,
        &["placement", "shift", "Home", "0", "-500", "--all"],
    ));
    assert!(
        outside["error"]["message"]
            .as_str()
            .unwrap()
            .contains("objects would be outside"),
        "{outside}"
    );
}

#[test]
fn component_instance_objects_have_readable_names() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-component-names");
    new_session(&cwd);
    run_all(
        &cwd,
        &[
            "object create WorkingSet WS",
            "object create DataMask Home",
            "object create Button Frame",
            "object create OutputString Value",
            "object create FontAttributes Font",
            "reference set WS active_mask Home",
            "object set Frame width=148 height=60",
            "object set Value width=140 height=20",
            "reference set Value font_attributes Font",
            "placement add Frame Value 0 8",
            "placement add Home Frame 10 20",
        ],
    );
    // A real capture reuses the captured IDs; readable output names them by
    // instance and node, as --check does.
    let output = Command::new(env!("CARGO_BIN_EXE_aitd"))
        .current_dir(&cwd)
        .args(["component", "capture", "Frame", "Toggle"])
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Toggle 1/Frame"), "{text}");
    assert!(text.contains("Toggle 1/Value"), "{text}");
    success(cli(
        &cwd,
        &["component", "instance-rename", "Toggle 1", "Panel one"],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "instance-create",
            "Panel two",
            "Toggle",
            "Home",
            "200",
            "20",
        ],
    ));

    // Lists, gets and readable output name them `<instance>/<node>`.
    let listed = success(cli(&cwd, &["object", "list", "--type", "OutputString"]));
    let names: Vec<_> = listed["result"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|object| object["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["Panel one/Value", "Panel two/Value"]);
    let value = get(&cwd, "panel one / value");
    assert_eq!(value["result"]["objects"][0]["name"], "Panel one/Value");
    assert_eq!(
        value["result"]["objects"][0]["references"]["font_attributes"]["name"],
        "Panel one/Font"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_aitd"))
        .current_dir(&cwd)
        .args([
            "component",
            "instance-create",
            "Third",
            "Toggle",
            "Home",
            "200",
            "200",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Third/Value"), "{text}");

    // Edits are refused with a pointer to the component commands.
    let refused = failure(cli(
        &cwd,
        &["object", "set", "Panel one/Value", "width=100"],
    ));
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(message.contains("component override-set"), "{message}");
    // Also when the name is resolved inside an operation.
    let refused = failure(cli(
        &cwd,
        &["placement", "add", "Panel one/Frame", "Font", "0", "0"],
    ));
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(message.contains("component override-set"), "{message}");

    // A component object as tree root shows its own contents.
    let tree = success(cli(&cwd, &["tree", "Panel one/Frame"]));
    let names: Vec<_> = tree["result"]["context"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|object| object["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains(&"Panel one/Value".to_owned()), "{names:?}");

    // Deleting the mask an instance sits on names the instance.
    let refused = failure(cli(&cwd, &["object", "delete", "Home", "--unplace"]));
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("parent of component instance"),
        "{message}"
    );
}

#[test]
fn retargeting_a_position_parameter_moves_the_new_child_in_the_master() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-position-retarget");
    new_session(&cwd);
    run_all(
        &cwd,
        &[
            "object create WorkingSet WS",
            "object create DataMask Home",
            "object create Button Frame",
            "object create Container Icon",
            "object create Container Badge",
            "reference set WS active_mask Home",
            "object set Frame width=148 height=60",
            "object set Icon width=20 height=20",
            "object set Badge width=20 height=20",
            "placement add Frame Icon 4 4",
            "placement add Frame Badge 60 4",
            "placement add Home Frame 0 0",
            "component capture Frame Button",
            "component parameter-add Button ChildPos Icon position Position 10,8",
            "component parameter-set Button ChildPos ChildPos Badge position Position 10,8",
        ],
    );
    // Master child offsets by node name.
    let component = success(cli(&cwd, &["component", "get", "Button", "--nodes"]));
    let nodes = component["result"]["component"]["nodes"]
        .as_array()
        .unwrap();
    let name_of = |id: &Value| {
        nodes
            .iter()
            .find(|node| node["id"] == *id)
            .and_then(|node| node["name"].as_str())
            .unwrap()
            .to_owned()
    };
    let mut offsets: Vec<_> = nodes
        .iter()
        .flat_map(|node| node["children"].as_array().cloned().unwrap_or_default())
        .map(|child| {
            (
                name_of(&child["child"]),
                child["x"].as_i64().unwrap(),
                child["y"].as_i64().unwrap(),
            )
        })
        .collect();
    offsets.sort();
    // The new target follows the parameter; the old one keeps its offset.
    assert_eq!(
        offsets,
        [("Badge".to_owned(), 10, 8), ("Icon".to_owned(), 10, 8)]
    );
}
