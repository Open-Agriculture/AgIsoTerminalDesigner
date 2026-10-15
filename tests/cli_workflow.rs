//! Shell-to-editor workflows exercise direct editing and the authenticated bridge.
#![cfg(not(target_arch = "wasm32"))]

use ag_iso_terminal_designer::{ai::automation_bridge::EditorAutomationBridge, EditorProject};
use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

fn cli(
    bridge: &mut EditorAutomationBridge,
    project: &mut EditorProject,
    cwd: &Path,
    args: &[&str],
) -> Output {
    let instance = bridge.editor_instance_id().to_owned();
    let cwd = cwd.to_owned();
    let args: Vec<_> = args.iter().map(|s| s.to_string()).collect();
    let caller = thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_aitd"))
            .current_dir(cwd)
            .args(["--instance", &instance, "--json"])
            .args(args)
            .env_remove("AG_ISO_EDITOR_INSTANCE_ID")
            .output()
            .unwrap()
    });
    let until = Instant::now() + Duration::from_secs(15);
    while !caller.is_finished() && Instant::now() < until {
        bridge.poll(Some(&mut *project));
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        caller.is_finished(),
        "CLI did not finish while the editor was polling"
    );
    caller.join().unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
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

fn get(
    bridge: &mut EditorAutomationBridge,
    project: &mut EditorProject,
    cwd: &Path,
    target: &str,
) -> Value {
    success(cli(bridge, project, cwd, &["object", "get", target]))
}

#[test]
fn direct_edits_are_typed_and_check_does_not_commit() {
    let cwd = temp_dir("aitd-direct-edit");
    let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
    let mut project = EditorProject::new_pool();

    let status = success(cli(&mut bridge, &mut project, &cwd, &["status"]));
    assert!(status["success"].as_bool().unwrap());
    assert!(status["project_instance_id"].as_str().is_some());
    assert!(status["revision"].as_u64().is_some());

    let capabilities = success(cli(&mut bridge, &mut project, &cwd, &["capabilities"]));
    assert_eq!(capabilities["result"]["json_input"], false);

    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "create", "NumberVariable", "Counter"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "set", "Counter", "value=42"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "create", "StringVariable", "Literal"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "set", "Literal", "value={\"enabled\":true}"],
    ));
    let literal = get(&mut bridge, &mut project, &cwd, "Literal");
    assert_eq!(
        literal["result"]["objects"][0]["properties"]["value"],
        "{\"enabled\":true}"
    );

    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "create", "OutputPolygon", "Shape"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "field", "Shape", "points.0", "x=12", "y=20"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "object", "array", "append", "Shape", "points", "x=30", "y=40",
        ],
    ));
    let shape = get(&mut bridge, &mut project, &cwd, "Shape");
    assert_eq!(
        shape["result"]["objects"][0]["properties"]["points"][0]["x"],
        12
    );
    assert_eq!(
        shape["result"]["objects"][0]["properties"]["points"][0]["y"],
        20
    );
    assert_eq!(
        shape["result"]["objects"][0]["properties"]["points"][3]["x"],
        30
    );

    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "create", "ExtendedInputAttributes", "Characters"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "object",
            "array",
            "append",
            "Characters",
            "code_planes",
            "number=0",
        ],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "object",
            "array",
            "append",
            "Characters",
            "code_planes.0.character_ranges",
            "first_character=65",
            "last_character=90",
        ],
    ));
    let characters = get(&mut bridge, &mut project, &cwd, "Characters");
    assert_eq!(
        characters["result"]["objects"][0]["properties"]["code_planes"][0]["character_ranges"][0]
            ["first_character"],
        65
    );
    assert_eq!(
        characters["result"]["objects"][0]["properties"]["code_planes"][0]["character_ranges"][0]
            ["last_character"],
        90
    );

    let revision = project.revision();
    let checked = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["--check", "object", "set", "Counter", "value=77"],
    ));
    assert_eq!(project.revision(), revision);
    assert_eq!(
        get(&mut bridge, &mut project, &cwd, "Counter")["result"]["objects"][0]["properties"]
            ["value"],
        42
    );
    assert!(checked["success"].as_bool().unwrap());

    let malformed = failure(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "set", "Counter", "value={\"not\":\"a number\"}"],
    ));
    assert!(malformed["error"]["message"].as_str().is_some());
    assert_eq!(project.revision(), revision);
    assert_eq!(
        get(&mut bridge, &mut project, &cwd, "Counter")["result"]["objects"][0]["properties"]
            ["value"],
        42
    );

    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn incremental_list_and_event_commands_keep_existing_entries() {
    let cwd = temp_dir("aitd-relationships");
    let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
    let mut project = EditorProject::new_pool();

    for (object_type, name) in [
        ("OutputNumber", "First"),
        ("OutputNumber", "Second"),
        ("InputList", "Choices"),
        ("Macro", "Press"),
        ("Macro", "Release"),
        ("Button", "Trigger"),
    ] {
        success(cli(
            &mut bridge,
            &mut project,
            &cwd,
            &["object", "create", object_type, name],
        ));
    }

    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["list", "set", "Choices", "First", "null", "Second"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["event", "bind", "Trigger", "OnKeyPress=Press"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["event", "bind", "Trigger", "OnKeyRelease=Release"],
    ));

    let choices = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "get", "Choices", "--children"],
    ));
    let list = &choices["result"]["objects"][0]["object_list"];
    assert_eq!(list["kind"], "Nullable");
    assert_eq!(list["objects"].as_array().unwrap().len(), 3);
    assert_eq!(list["objects"][0]["name"], "First");
    assert!(list["objects"][1].is_null());
    assert_eq!(list["objects"][2]["name"], "Second");

    let trigger = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "get", "Trigger", "--macros"],
    ));
    let bindings = trigger["result"]["objects"][0]["macro_references"]
        .as_array()
        .unwrap();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0]["event"], "OnKeyPress");
    assert_eq!(bindings[1]["event"], "OnKeyRelease");
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn command_files_apply_atomically_as_one_undo_step() {
    let cwd = temp_dir("aitd-command-file");
    let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
    let mut project = EditorProject::new_pool();
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "create", "NumberVariable", "Counter"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "set", "Counter", "value=1"],
    ));
    project.mark_saved();

    std::fs::write(
        cwd.join("valid.aitd"),
        "object set Counter value=2\nproject settings set mask_size=640\n",
    )
    .unwrap();
    let before = project.revision();
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["--check", "run", "--file", "valid.aitd"],
    ));
    assert_eq!(project.revision(), before);
    assert_eq!(project.mask_size, 500);
    assert_eq!(
        get(&mut bridge, &mut project, &cwd, "Counter")["result"]["objects"][0]["properties"]
            ["value"],
        1
    );

    let applied = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["run", "--file", "valid.aitd"],
    ));
    assert_eq!(project.revision(), before + 1);
    assert_eq!(project.mask_size, 640);
    assert!(project.is_dirty());
    assert!(applied["success"].as_bool().unwrap());

    std::fs::write(
        cwd.join("invalid.aitd"),
        "object set Counter value=3\nproject settings set mask_size=0\n",
    )
    .unwrap();
    let before_failure = project.revision();
    let failed = failure(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["run", "--file", "invalid.aitd"],
    ));
    assert_eq!(project.revision(), before_failure);
    assert_eq!(project.mask_size, 640);
    assert_eq!(
        get(&mut bridge, &mut project, &cwd, "Counter")["result"]["objects"][0]["properties"]
            ["value"],
        2
    );
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("line 2"));

    assert!(project.undo_operation());
    assert_eq!(project.mask_size, 500);
    assert_eq!(
        get(&mut bridge, &mut project, &cwd, "Counter")["result"]["objects"][0]["properties"]
            ["value"],
        1
    );
    assert!(!project.is_dirty());
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn settings_validation_and_revision_guards_reject_stale_edits() {
    let cwd = temp_dir("aitd-settings-guards");
    let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
    let mut project = EditorProject::new_pool();

    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "create", "GraphicData", "ImageBytes"],
    ));
    let before_invalid = project.revision();
    failure(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["project", "settings", "set", "vt_version=5"],
    ));
    failure(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["project", "settings", "set", "mask_size=0"],
    ));
    assert_eq!(project.revision(), before_invalid);
    assert_eq!(project.mask_size, 500);

    let edited = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "create", "NumberVariable", "Fresh"],
    ));
    let revision = edited["revision"].as_u64().unwrap();
    assert_eq!(revision, project.revision());

    let stale_revision = revision.saturating_sub(1).to_string();
    let stale = failure(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["--revision", &stale_revision, "object", "delete", "Fresh"],
    ));
    assert_eq!(stale["error"]["code"], "PROJECT_REVISION_MISMATCH");

    let wrong_project = failure(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "--project-instance",
            "different-project",
            "object",
            "delete",
            "Fresh",
        ],
    ));
    assert_eq!(wrong_project["error"]["code"], "PROJECT_INSTANCE_MISMATCH");
    assert!(get(&mut bridge, &mut project, &cwd, "Fresh")["success"] == true);
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn relative_inputs_mixed_batch_preview_and_exports_work_end_to_end() {
    let cwd = temp_dir("aitd-io-workflow");
    let inputs = cwd.join("inputs");
    std::fs::create_dir(&inputs).unwrap();
    std::fs::write(inputs.join("label.txt"), "007 {plain text}").unwrap();
    std::fs::write(inputs.join("payload.bin"), [1_u8, 2, 3, 4]).unwrap();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([220, 40, 30, 255]))
        .save(inputs.join("pixel.png"))
        .unwrap();

    let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
    let mut project = EditorProject::new_pool();
    for (object_type, name) in [
        ("WorkingSet", "Pool"),
        ("DataMask", "Screen"),
        ("Container", "CaptureRoot"),
        ("Container", "Host"),
        ("StringVariable", "Label"),
        ("GraphicData", "Bytes"),
    ] {
        success(cli(
            &mut bridge,
            &mut project,
            &cwd,
            &["object", "create", object_type, name],
        ));
    }
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "set", "CaptureRoot", "width=120", "height=100"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["object", "set", "Host", "width=200", "height=150"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["reference", "set", "Pool", "active_mask", "Screen"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["placement", "add", "Screen", "Host", "0", "0"],
    ));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["placement", "add", "Host", "CaptureRoot", "10", "10"],
    ));
    let capture_root_id = get(&mut bridge, &mut project, &cwd, "CaptureRoot")["result"]["objects"]
        [0]["id"]
        .as_u64()
        .unwrap();
    let screen_id = get(&mut bridge, &mut project, &cwd, "Screen")["result"]["objects"][0]["id"]
        .as_u64()
        .unwrap();
    project.mark_saved();

    std::fs::write(
        inputs.join("changes.aitd"),
        format!(
            "object set Label value --value-file label.txt\n\
             object bytes-import Bytes data payload.bin\n\
             object create OutputRectangle Child\n\
             object create LineAttributes ChildLine\n\
             reference set Child line_attributes ChildLine\n\
             object set Child width=30 height=20\n\
             placement add CaptureRoot Child 5 7\n\
             component capture {capture_root_id} Reusable\n\
             graphic import pixel.png --name Pixel\n\
             project settings set mask_size=640\n"
        ),
    )
    .unwrap();
    let before = project.revision();
    let edit = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["run", "--file", "inputs/changes.aitd"],
    ));
    assert!(edit["success"].as_bool().unwrap());
    assert_eq!(project.revision(), before + 1);
    assert_eq!(project.mask_size, 640);
    assert!(project.is_dirty());
    assert_eq!(
        get(&mut bridge, &mut project, &cwd, "Label")["result"]["objects"][0]["properties"]
            ["value"],
        "007 {plain text}"
    );
    assert_eq!(
        get(&mut bridge, &mut project, &cwd, "Bytes")["result"]["objects"][0]["properties"]["data"]
            ["byte_length"],
        4
    );
    assert_eq!(
        success(cli(
            &mut bridge,
            &mut project,
            &cwd,
            &["component", "list", "--query", "Reusable"],
        ))["result"]["total"],
        1
    );

    let schema = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["schema", "OutputRectangle", "--properties", "width"],
    ));
    let properties = schema["result"]["schemas"][0]["properties"]
        .as_array()
        .unwrap();
    assert_eq!(properties.len(), 1);
    assert_eq!(properties[0]["name"], "width");

    let nested_schema = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "schema",
            "ExtendedInputAttributes",
            "--properties",
            "code_planes",
        ],
    ));
    let character_range = &nested_schema["result"]["schemas"][0]["properties"][0]
        ["nested_array_elements"]["*.character_ranges"];
    assert_eq!(character_range["first_character"], 0);
    assert_eq!(character_range["last_character"], 0);

    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "object",
            "bytes-export",
            "Bytes",
            "data",
            "--output",
            "copied.bin",
        ],
    ));
    assert_eq!(std::fs::read(cwd.join("copied.bin")).unwrap(), [1, 2, 3, 4]);

    let graphics = success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "graphic",
            "list",
            "--query",
            "Pixel",
            "--status",
            "available",
        ],
    ));
    let asset_id = graphics["result"]["graphics"]["graphics"][0]["asset_id"]
        .as_str()
        .unwrap_or_else(|| panic!("Expected named available asset in {graphics}"));
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &["graphic", "export", asset_id, "--output", "graphic.png"],
    ));
    assert!(std::fs::read(cwd.join("graphic.png"))
        .unwrap()
        .starts_with(b"\x89PNG\r\n\x1a\n"));

    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "preview",
            "--mask",
            &screen_id.to_string(),
            "--output",
            "preview.png",
        ],
    ));
    assert!(std::fs::read(cwd.join("preview.png"))
        .unwrap()
        .starts_with(b"\x89PNG\r\n\x1a\n"));

    let bundle_args = [
        "export", "bundle", "--iop", "pool.iop", "--header", "pool.h",
    ];
    success(cli(&mut bridge, &mut project, &cwd, &bundle_args));
    let original_iop = std::fs::read(cwd.join("pool.iop")).unwrap();
    let original_header = std::fs::read(cwd.join("pool.h")).unwrap();
    assert!(!original_iop.is_empty());
    assert!(!original_header.is_empty());

    let refused = failure(cli(&mut bridge, &mut project, &cwd, &bundle_args));
    assert_eq!(refused["error"]["code"], "OUTPUT_EXISTS");
    assert_eq!(std::fs::read(cwd.join("pool.iop")).unwrap(), original_iop);
    assert_eq!(std::fs::read(cwd.join("pool.h")).unwrap(), original_header);
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "export", "bundle", "--iop", "pool.iop", "--header", "pool.h", "--force",
        ],
    ));

    std::fs::write(cwd.join("protected.iop"), b"keep this content").unwrap();
    std::fs::create_dir(cwd.join("blocked.h")).unwrap();
    failure(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "export",
            "bundle",
            "--iop",
            "protected.iop",
            "--header",
            "blocked.h",
            "--force",
        ],
    ));
    assert_eq!(
        std::fs::read(cwd.join("protected.iop")).unwrap(),
        b"keep this content"
    );
    assert!(std::fs::read_dir(&cwd).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));

    assert!(project.undo_operation());
    assert_eq!(project.mask_size, 500);
    assert!(!project.is_dirty());
    success(cli(
        &mut bridge,
        &mut project,
        &cwd,
        &[
            "object",
            "bytes-export",
            "Bytes",
            "data",
            "--output",
            "empty-after-undo.bin",
        ],
    ));
    assert!(std::fs::read(cwd.join("empty-after-undo.bin"))
        .unwrap()
        .is_empty());
    assert_eq!(
        success(cli(
            &mut bridge,
            &mut project,
            &cwd,
            &[
                "graphic",
                "list",
                "--query",
                "Pixel",
                "--status",
                "available"
            ],
        ))["result"]["graphics"]["total"],
        0
    );
    assert_eq!(
        success(cli(
            &mut bridge,
            &mut project,
            &cwd,
            &["component", "list", "--query", "Reusable"],
        ))["result"]["total"],
        0
    );
    std::fs::remove_dir_all(cwd).unwrap();
}
