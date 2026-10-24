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
