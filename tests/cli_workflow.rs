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

fn get(cwd: &Path, target: &str) -> Value {
    success(cli(cwd, &["object", "get", target]))
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
            "--id",
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
            "--id",
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
fn list_and_event_commands_keep_existing_entries() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-relationships");
    new_session(&cwd);

    for (object_type, name) in [
        ("OutputNumber", "First"),
        ("OutputNumber", "Second"),
        ("InputList", "Choices"),
        ("Macro", "Press"),
        ("Macro", "Release"),
        ("Button", "Trigger"),
    ] {
        success(cli(&cwd, &["object", "create", object_type, name]));
    }
    let page = success(cli(
        &cwd,
        &[
            "object",
            "list",
            "--type",
            "OutputNumber",
            "--offset",
            "1",
            "--limit",
            "1",
        ],
    ));
    assert_eq!(page["result"]["objects"].as_array().unwrap().len(), 1);
    assert_eq!(page["result"]["offset"], 1);
    success(cli(
        &cwd,
        &["list", "set", "Choices", "First", "null", "Second"],
    ));
    success(cli(&cwd, &["event", "bind", "Trigger", "OnKeyPress=Press"]));
    success(cli(
        &cwd,
        &["event", "bind", "Trigger", "OnKeyRelease=Release"],
    ));

    let choices = success(cli(&cwd, &["object", "get", "Choices", "--children"]));
    let list = &choices["result"]["objects"][0]["object_list"];
    assert_eq!(list["kind"], "Nullable");
    assert_eq!(list["objects"].as_array().unwrap().len(), 3);
    assert_eq!(list["objects"][0]["name"], "First");
    assert!(list["objects"][1].is_null());
    assert_eq!(list["objects"][2]["name"], "Second");

    let trigger = success(cli(&cwd, &["object", "get", "Trigger", "--macros"]));
    let bindings = trigger["result"]["objects"][0]["macro_references"]
        .as_array()
        .unwrap();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0]["event"], "OnKeyPress");
    assert_eq!(bindings[1]["event"], "OnKeyRelease");
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
fn resource_imports_and_exports_use_local_files() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-resource-workflow");
    let inputs = cwd.join("inputs");
    std::fs::create_dir(&inputs).unwrap();
    std::fs::write(inputs.join("payload.bin"), [1_u8, 2, 3, 4]).unwrap();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([220, 40, 30, 255]))
        .save(inputs.join("pixel.png"))
        .unwrap();
    new_session(&cwd);
    for (object_type, name) in [
        ("WorkingSet", "Pool"),
        ("DataMask", "Screen"),
        ("Container", "Host"),
        ("StringVariable", "Label"),
        ("GraphicData", "Bytes"),
    ] {
        success(cli(&cwd, &["object", "create", object_type, name]));
    }
    success(cli(
        &cwd,
        &["object", "set", "Host", "width=200", "height=150"],
    ));
    success(cli(
        &cwd,
        &["reference", "set", "Pool", "active_mask", "Screen"],
    ));
    success(cli(&cwd, &["placement", "add", "Screen", "Host", "0", "0"]));
    let screen_id = get(&cwd, "Screen")["result"]["objects"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string();
    std::fs::write(
        inputs.join("changes.aitcmd"),
        "object set Label value='007 {plain text}'\n\
         object bytes-import Bytes data payload.bin\n\
         graphic import pixel.png --name Pixel\n",
    )
    .unwrap();

    success(cli(&cwd, &["run", "--file", "inputs/changes.aitcmd"]));
    assert_eq!(
        get(&cwd, "Label")["result"]["objects"][0]["properties"]["value"],
        "007 {plain text}"
    );
    assert_eq!(
        get(&cwd, "Bytes")["result"]["objects"][0]["properties"]["data"]["byte_length"],
        4
    );
    success(cli(
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
        .unwrap();
    success(cli(
        &cwd,
        &["graphic", "export", asset_id, "--output", "graphic.png"],
    ));
    assert!(std::fs::read(cwd.join("graphic.png"))
        .unwrap()
        .starts_with(b"\x89PNG\r\n\x1a\n"));

    success(cli(
        &cwd,
        &["preview", "--mask", &screen_id, "--output", "preview.png"],
    ));
    assert!(std::fs::read(cwd.join("preview.png"))
        .unwrap()
        .starts_with(b"\x89PNG\r\n\x1a\n"));

    let bundle = [
        "export", "bundle", "--iop", "pool.iop", "--header", "pool.h",
    ];
    success(cli(&cwd, &bundle));
    let original_iop = std::fs::read(cwd.join("pool.iop")).unwrap();
    let original_header = std::fs::read(cwd.join("pool.h")).unwrap();
    assert!(!original_iop.is_empty());
    assert!(!original_header.is_empty());
    let refused = failure(cli(&cwd, &bundle));
    assert_eq!(refused["error"]["code"], "OUTPUT_EXISTS");
    success(cli(
        &cwd,
        &[
            "export", "bundle", "--iop", "pool.iop", "--header", "pool.h", "--force",
        ],
    ));
    assert_eq!(std::fs::read(cwd.join("pool.iop")).unwrap(), original_iop);
    assert_eq!(std::fs::read(cwd.join("pool.h")).unwrap(), original_header);
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
