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

fn cli_readable(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aitd"))
        .current_dir(cwd)
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
fn entity_targets_normalize_names_and_numeric_looking_names_stay_names() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-name-targets");
    new_session(&cwd);

    success(cli(
        &cwd,
        &["object", "create", "NumberVariable", "Start Button"],
    ));
    success(cli(&cwd, &["object", "set", "START_BUTTON", "value=17"]));
    assert_eq!(
        get(&cwd, "start-button")["result"]["objects"][0]["properties"]["value"],
        17
    );
    let normalized_duplicate = failure(cli(
        &cwd,
        &["object", "create", "NumberVariable", "start_button"],
    ));
    assert!(normalized_duplicate["error"]["message"].as_str().is_some());

    success(cli(&cwd, &["object", "create", "NumberVariable", "123"]));
    success(cli(&cwd, &["object", "set", "123", "value=23"]));
    assert_eq!(
        get(&cwd, "123")["result"]["objects"][0]["properties"]["value"],
        23
    );

    success(cli(&cwd, &["object", "create", "Container", "name:Root"]));
    assert_eq!(
        get(&cwd, "name:name:root")["result"]["objects"][0]["name"],
        "name:Root"
    );
    success(cli(&cwd, &["tree", "name:name:root"]));

    success(cli(
        &cwd,
        &[
            "object",
            "create",
            "NumberVariable",
            "Fixed Counter",
            "--override-id",
            "6100",
        ],
    ));
    assert_eq!(
        get(&cwd, "fixed-counter")["result"]["objects"][0]["id"],
        6100
    );
    success(cli(
        &cwd,
        &["object", "override-id", "FIXED_COUNTER", "automatic"],
    ));
    assert_ne!(
        get(&cwd, "Fixed Counter")["result"]["objects"][0]["id"],
        6100
    );

    std::fs::write(
        cwd.join("sequential.aitcmd"),
        "object create NumberVariable 'Later Name'\nobject set LATER_NAME value=31\n",
    )
    .unwrap();
    success(cli(&cwd, &["run", "--file", "sequential.aitcmd"]));
    assert_eq!(
        get(&cwd, "later-name")["result"]["objects"][0]["properties"]["value"],
        31
    );
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn readable_cli_reports_names_when_creating_listing_and_deleting() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-readable-names");
    new_session(&cwd);

    let created = cli_readable(
        &cwd,
        &["object", "create", "NumberVariable", "Friendly Counter"],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created = String::from_utf8(created.stdout).unwrap();
    assert!(created.contains("Friendly Counter"), "{created}");
    assert!(!created.contains("allocated_ids"), "{created}");
    assert!(!created.contains("changed_ids"), "{created}");

    let listed = cli_readable(&cwd, &["object", "list"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(listed.contains("Friendly Counter"), "{listed}");
    assert!(!listed.contains("allocated_ids"), "{listed}");
    assert!(!listed.contains("changed_ids"), "{listed}");

    let deleted = cli_readable(&cwd, &["object", "delete", "friendly-counter"]);
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    let deleted = String::from_utf8(deleted.stdout).unwrap();
    assert!(deleted.contains("Friendly Counter"), "{deleted}");
    assert!(!deleted.contains("allocated_ids"), "{deleted}");
    assert!(!deleted.contains("changed_ids"), "{deleted}");

    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn component_node_and_parameter_names_are_scoped_to_their_definition() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-component-name-scopes");
    new_session(&cwd);

    for (component, root) in [
        ("First Component", "First Root"),
        ("Second Component", "Second Root"),
    ] {
        success(cli(
            &cwd,
            &[
                "component",
                "definition-create",
                component,
                root,
                "Container",
            ],
        ));
        success(cli(
            &cwd,
            &[
                "component",
                "node-create",
                component,
                "Shared Node",
                "OutputNumber",
            ],
        ));
        success(cli(
            &cwd,
            &[
                "component",
                "parameter-add-typed",
                component,
                "Shared Parameter",
                "shared_node",
                "value",
                "Number",
                "0",
            ],
        ));
    }

    let duplicate_node = failure(cli(
        &cwd,
        &[
            "component",
            "node-create",
            "first_component",
            "shared_node",
            "OutputNumber",
        ],
    ));
    assert!(duplicate_node["error"]["message"].as_str().is_some());
    let duplicate_parameter = failure(cli(
        &cwd,
        &[
            "component",
            "parameter-add-typed",
            "FIRST COMPONENT",
            "shared_parameter",
            "Shared Node",
            "value",
            "Number",
            "0",
        ],
    ));
    assert!(duplicate_parameter["error"]["message"].as_str().is_some());

    success(cli(
        &cwd,
        &[
            "component",
            "parameter-remove",
            "first_component",
            "SHARED_PARAMETER",
        ],
    ));
    let first = success(cli(
        &cwd,
        &["component", "get", "First Component", "--nodes"],
    ));
    let second = success(cli(
        &cwd,
        &["component", "get", "second_component", "--nodes"],
    ));
    assert_eq!(
        first["result"]["component"]["parameters"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        second["result"]["component"]["parameters"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn component_reference_and_graphic_parameters_roundtrip_names() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-component-typed-names");
    let image_path = cwd.join("pixel.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 180, 70, 255]))
        .save(&image_path)
        .unwrap();
    new_session(&cwd);

    for (object_type, name) in [
        ("DataMask", "Screen"),
        ("NumberVariable", "Default Counter"),
        ("NumberVariable", "Override Counter"),
    ] {
        success(cli(&cwd, &["object", "create", object_type, name]));
    }
    for name in ["Default Artwork", "Override Artwork"] {
        success(cli(
            &cwd,
            &[
                "graphic",
                "import",
                image_path.to_str().unwrap(),
                "--name",
                name,
            ],
        ));
    }
    success(cli(
        &cwd,
        &[
            "component",
            "definition-create",
            "Name Card",
            "Root",
            "Container",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "node-create",
            "Name Card",
            "Input Node",
            "InputNumber",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "node-create",
            "Name Card",
            "Image Node",
            "PictureGraphic",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "node-children-set",
            "Name Card",
            "Root",
            "Input_Node:0:0",
            "Image_Node:0:0",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "parameter-add-typed",
            "Name Card",
            "Counter Reference",
            "Input Node",
            "variable_reference",
            "ObjectReference",
            "Default Counter",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "parameter-add-typed",
            "Name Card",
            "Artwork",
            "Image Node",
            "graphic",
            "Graphic",
            "Default Artwork",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "instance-create",
            "Current Card",
            "Name Card",
            "Screen",
            "10",
            "10",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "override-set",
            "current_card",
            "counter_reference",
            r#"{"name":"Override Counter"}"#,
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "override-set",
            "Current Card",
            "ARTWORK",
            r#""Override Artwork""#,
        ],
    ));

    let definition = success(cli(&cwd, &["component", "get", "NAME_CARD", "--nodes"]));
    let parameters = definition["result"]["component"]["parameters"]
        .as_array()
        .unwrap();
    assert!(parameters.iter().any(|parameter| {
        parameter["name"] == "Counter Reference"
            && parameter["default_value"]["name"] == "Default Counter"
    }));
    let graphic_id = success(cli(&cwd, &["graphic", "get", "override_artwork"]))["result"]
        ["graphic"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let instance = success(cli(&cwd, &["instance", "get", "CURRENT CARD"]));
    let instances = instance["result"]["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 1);
    let overrides = instances[0]["overrides"].as_object().unwrap();
    let reference_parameter = parameters
        .iter()
        .find(|parameter| parameter["name"] == "Counter Reference")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let artwork_parameter = parameters
        .iter()
        .find(|parameter| parameter["name"] == "Artwork")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(overrides[reference_parameter]["name"], "Override Counter");
    assert_eq!(overrides[artwork_parameter], graphic_id);
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn component_nested_object_targets_reject_numeric_ids() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-component-nested-targets");
    new_session(&cwd);
    success(cli(&cwd, &["object", "create", "DataMask", "Screen"]));
    success(cli(&cwd, &["object", "create", "Macro", "Press"]));
    success(cli(
        &cwd,
        &[
            "component",
            "definition-create",
            "References",
            "Root",
            "WorkingSet",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "node-create",
            "References",
            "Macro Node",
            "Macro",
        ],
    ));
    success(cli(
        &cwd,
        &[
            "component",
            "node-create",
            "References",
            "List Node",
            "InputList",
        ],
    ));

    for (args, detail) in [
        (
            vec![
                "component",
                "node-reference-set",
                "References",
                "Root",
                "active_mask",
                r#"{"ExternalObject":{"id":42}}"#,
            ],
            "ExternalObject",
        ),
        (
            vec![
                "component",
                "node-macro-references-set",
                "References",
                "Macro Node",
                r#"[{"event":"OnKeyPress","target":{"id":42}}]"#,
            ],
            "component macro references",
        ),
        (
            vec![
                "component",
                "node-object-list-set",
                "References",
                "List Node",
                r#"{"Required":[{"id":42}]}"#,
            ],
            "name",
        ),
    ] {
        let failed = failure(cli(&cwd, &args));
        let message = failed["error"]["message"].as_str().unwrap();
        assert!(
            message.contains(detail) || message.contains("name"),
            "unexpected rejection: {message}"
        );
    }
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
    std::fs::write(
        inputs.join("changes.aitcmd"),
        "object set Label value='007 {plain text}'\n\
         object bytes-import Bytes data payload.bin\n\
         graphic import pixel.png --name Pixel\n",
    )
    .unwrap();
    let absolute_pixel = inputs.join("pixel.png").canonicalize().unwrap();
    std::fs::write(
        inputs.join("absolute-import.aitcmd"),
        format!(
            "graphic import {} --name 'Absolute Pixel'\n",
            absolute_pixel.display()
        ),
    )
    .unwrap();

    success(cli(&cwd, &["run", "--file", "inputs/changes.aitcmd"]));
    success(cli(
        &cwd,
        &["run", "--file", "inputs/absolute-import.aitcmd"],
    ));
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
    assert!(graphics["result"]["graphics"]["graphics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|graphic| graphic["name"] == "Pixel"));
    let absolute_graphic = success(cli(&cwd, &["graphic", "get", "absolute-pixel"]));
    assert_eq!(
        absolute_graphic["result"]["graphic"]["name"],
        "Absolute Pixel"
    );
    success(cli(
        &cwd,
        &["graphic", "export", "pixel", "--output", "graphic.png"],
    ));
    assert!(std::fs::read(cwd.join("graphic.png"))
        .unwrap()
        .starts_with(b"\x89PNG\r\n\x1a\n"));

    success(cli(
        &cwd,
        &["preview", "--mask", "SCREEN", "--output", "preview.png"],
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

#[test]
fn project_export_publishes_validated_artifacts_and_refuses_existing_outputs() {
    let _session = CLI_SESSION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cwd = temp_dir("aitd-project-export");
    new_session(&cwd);
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
