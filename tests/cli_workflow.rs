//! A single shell-to-editor workflow exercises the command and bridge contracts.
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
    project: &EditorProject,
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
            .env_remove("AITD_DRAFT")
            .env_remove("AG_ISO_EDITOR_INSTANCE_ID")
            .output()
            .unwrap()
    });
    let until = Instant::now() + Duration::from_secs(15);
    while !caller.is_finished() && Instant::now() < until {
        bridge.poll(Some(project));
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

#[test]
fn commands_draft_script_review_and_export() {
    let cwd = std::env::temp_dir().join(format!("aitd-workflow-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&cwd).unwrap();
    let mut bridge = EditorAutomationBridge::start(eframe::egui::Context::default()).unwrap();
    let mut project = EditorProject::new_pool();
    let original = cwd.join("original.iop");
    let saved = cwd.join("saved.aitp");
    bridge.set_project_location(None, Some(&original), "Original pool");
    let identity = success(cli(&mut bridge, &project, &cwd, &["project", "current"]));
    assert_eq!(
        identity["result"]["original_source_path"],
        original.to_str().unwrap()
    );
    assert!(identity["result"]["current_project_path"].is_null());
    bridge.set_project_location(Some(&saved), Some(&original), "Saved pool");
    let saved_identity = success(cli(&mut bridge, &project, &cwd, &["project", "current"]));
    assert_eq!(
        saved_identity["result"]["current_project_path"],
        saved.to_str().unwrap()
    );
    assert_eq!(
        saved_identity["result"]["original_source_path"],
        original.to_str().unwrap()
    );
    assert_eq!(saved_identity["revision"], identity["revision"]);
    success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["draft", "begin", "--name", "CLI workflow"],
    ));
    success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["object", "create", "NumberVariable", "--name", "Counter"],
    ));
    success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["object", "set", "Counter", "value=42"],
    ));
    let objects = success(cli(
        &mut bridge,
        &project,
        &cwd,
        &[
            "--view",
            "draft",
            "object",
            "list",
            "--type",
            "NumberVariable",
        ],
    ));
    assert_eq!(objects["result"]["objects"].as_array().unwrap().len(), 1);
    success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["--view", "draft", "context", "--scope", "pool"],
    ));
    success(cli(&mut bridge, &project, &cwd, &["draft", "diff"]));
    let colours = success(cli(&mut bridge, &project, &cwd, &["colours", "6"]));
    assert_eq!(colours["result"]["results"][0]["resolved"]["index"], 6);
    let counter_id = objects["result"]["objects"][0]["id"]
        .as_u64()
        .unwrap()
        .to_string();
    let read = success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["--view", "draft", "object", "get", &counter_id],
    ));
    assert_eq!(read["result"]["objects"][0]["properties"]["value"], 42);
    std::fs::write(
        cwd.join("changes.aitd"),
        "object set Counter value=77\nproject settings set mask_size=640\n",
    )
    .unwrap();
    success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["run", "--file", "changes.aitd"],
    ));
    std::fs::write(
        cwd.join("bad.aitd"),
        "object set Counter value=99\nobject set Counter value=bad\n",
    )
    .unwrap();
    let failed = cli(&mut bridge, &project, &cwd, &["run", "--file", "bad.aitd"]);
    assert!(!failed.status.success());
    let read = success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["--view", "draft", "object", "get", &counter_id],
    ));
    assert_eq!(read["result"]["objects"][0]["properties"]["value"], 77);
    let submitted = success(cli(&mut bridge, &project, &cwd, &["draft", "submit"]));
    let proposal_id = submitted["result"]["proposal_id"].as_str().unwrap();
    let pending = success(cli(&mut bridge, &project, &cwd, &["proposal", "status"]));
    assert_eq!(pending["result"]["outcome"]["status"], "pending");
    let applied = project
        .apply_ai_proposal(bridge.take_proposal().unwrap())
        .unwrap();
    bridge.record_applied(proposal_id, project.revision(), &applied.transaction_id);
    let receipt = success(cli(
        &mut bridge,
        &project,
        &cwd,
        &["proposal", "wait", "--timeout", "1"],
    ));
    assert_eq!(receipt["result"]["outcome"]["status"], "applied");
    success(cli(
        &mut bridge,
        &project,
        &cwd,
        &[
            "export",
            "bundle",
            "--proposal",
            proposal_id,
            "--iop",
            "pool.iop",
            "--header",
            "pool.h",
        ],
    ));
    assert_eq!(
        std::fs::read(cwd.join("pool.iop")).unwrap(),
        project.export_iop().unwrap()
    );
    assert!(std::fs::read_to_string(cwd.join("pool.h"))
        .unwrap()
        .contains(&format!("#define COUNTER {counter_id}")));
    assert!(project.undo_operation());
    assert_eq!(project.mask_size, 500);
    let stale = cli(
        &mut bridge,
        &project,
        &cwd,
        &[
            "export",
            "iop",
            "--proposal",
            proposal_id,
            "--output",
            "stale.iop",
        ],
    );
    assert!(!stale.status.success());
    assert!(!cwd.join("stale.iop").exists());
    std::fs::remove_dir_all(cwd).unwrap();
}
