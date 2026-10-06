#[cfg(not(target_arch = "wasm32"))]
mod native {
    use ag_iso_terminal_designer::ai::automation_bridge::{
        invoke_runtime_request, AutomationRequest, AutomationResponse,
    };
    use clap::{Parser, Subcommand};
    use serde_json::{json, Value};
    use std::{
        fs,
        io::{self, Read},
        process::ExitCode,
    };

    #[derive(Debug, Parser)]
    #[command(
        name = "aitd",
        about = "Automate a running AgIsoTerminalDesigner editor"
    )]
    struct Cli {
        /// Select an editor when more than one is running.
        #[arg(long, global = true)]
        instance: Option<String>,
        /// Require the open project to have this identity.
        #[arg(long, global = true)]
        project_instance: Option<String>,
        /// Require the open project to have this revision.
        #[arg(long, global = true)]
        revision: Option<u64>,
        /// Emit compact machine-readable JSON (JSON is the default output).
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, Subcommand)]
    enum Command {
        /// Show editor and open project identity, revision, and pending proposal.
        Status,
        /// Read the current design context from the live editor.
        Context,
        /// Inspect objects by their numeric IDs.
        Objects { ids: Vec<u16> },
        /// Read writable properties and relationship rules for an object type.
        ObjectSchema { object_type: String },
        /// Inspect reusable components.
        Components {
            #[command(subcommand)]
            command: ComponentCommand,
        },
        /// Inspect graphics in the current project or proposal.
        Graphics {
            #[command(subcommand)]
            command: GraphicsCommand,
        },
        /// Analyze the active design or pending proposal.
        Diagnostics {
            #[arg(long, value_parser = ["active_mask", "proposal"])]
            scope: Option<String>,
        },
        /// Stage, inspect, or reject the pending proposal.
        Proposal {
            #[command(subcommand)]
            command: ProposalCommand,
        },
        /// Describe the compact automation command surface.
        Capabilities,
    }

    #[derive(Debug, Subcommand)]
    enum ComponentCommand {
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long)]
            category: Option<String>,
            #[arg(long, default_value_t = 20)]
            limit: u8,
        },
        Get {
            component_id: String,
            #[arg(long)]
            include_nodes: bool,
        },
    }

    #[derive(Debug, Subcommand)]
    enum GraphicsCommand {
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long, default_value = "all")]
            status: String,
            #[arg(long, default_value_t = 20)]
            limit: u8,
        },
    }

    #[derive(Debug, Subcommand)]
    enum ProposalCommand {
        /// Stage a transaction or component actions from a JSON file (use - for stdin).
        Stage {
            #[arg(long)]
            input: String,
        },
        /// Show the current proposal summary.
        Show,
        /// Render the pending proposal preview as JSON including its PNG payload.
        Preview { proposal_id: String },
        /// Reject the current proposal in the editor.
        Reject { proposal_id: String },
    }

    fn error_value(
        code: &str,
        message: impl Into<String>,
        expected: Option<u64>,
        current: Option<u64>,
    ) -> Value {
        let mut error = json!({"code":code,"message":message.into()});
        if let Some(revision) = expected {
            error["expected_revision"] = json!(revision);
        }
        if let Some(revision) = current {
            error["current_revision"] = json!(revision);
        }
        json!({"error":error})
    }

    fn response_value(response: AutomationResponse) -> Result<Value, Value> {
        if !response.success {
            let error = response
            .error
            .map(|e| {
                json!({
                    "code":e.code, "message":e.message,
                    "expected_revision":e.expected_revision, "current_revision":e.current_revision
                })
            })
            .unwrap_or_else(
                || json!({"code":"AUTOMATION_FAILED","message":"Editor automation request failed"}),
            );
            return Err(json!({"error":error}));
        }
        Ok(json!({
            "editor_instance":response.editor_instance_id,
            "project_instance":response.project_instance_id,
            "project_revision":response.revision,
            "result":response.result
        }))
    }

    fn call(
        instance: Option<&str>,
        action: &str,
        args: Value,
        identity: Option<(&str, u64)>,
    ) -> Result<Value, Value> {
        let request = AutomationRequest {
            project_instance_id: identity.map(|(id, _)| id.to_owned()),
            revision: identity.map(|(_, revision)| revision),
            action: action.to_owned(),
            args,
        };
        match invoke_runtime_request(instance, request) {
            Ok(response) => response_value(response),
            Err(error) => Err(error_value(
                &error.code,
                error.message,
                error.expected_revision,
                error.current_revision,
            )),
        }
    }

    fn call_current(
        instance: Option<&str>,
        action: &str,
        args: Value,
        expected_project: Option<&str>,
        expected_revision: Option<u64>,
    ) -> Result<Value, Value> {
        let status = call(instance, "get_project_status", Value::Null, None)?;
        let project_open = status["result"]["project_open"].as_bool().unwrap_or(false);
        if !project_open {
            return Err(error_value(
                "PROJECT_NOT_OPEN",
                "Open a project in the editor before using this command.",
                None,
                None,
            ));
        }
        let project_instance = expected_project
            .or_else(|| status["project_instance"].as_str())
            .unwrap_or_default();
        let revision = expected_revision
            .or_else(|| status["project_revision"].as_u64())
            .unwrap_or_default();
        call(instance, action, args, Some((project_instance, revision)))
    }

    fn read_input(path: &str) -> Result<Value, Value> {
        let mut source = String::new();
        if path == "-" {
            io::stdin()
                .read_to_string(&mut source)
                .map_err(|e| error_value("INVALID_INPUT", e.to_string(), None, None))?;
        } else {
            source = fs::read_to_string(path)
                .map_err(|e| error_value("INVALID_INPUT", e.to_string(), None, None))?;
        }
        serde_json::from_str(&source).map_err(|e| {
            error_value(
                "INVALID_INPUT",
                format!("Invalid JSON input: {e}"),
                None,
                None,
            )
        })
    }

    fn capabilities() -> Value {
        json!({
            "automation_api_version": 1,
            "capabilities": ["live_project_inspection", "components", "graphics", "diagnostics", "staged_proposals", "proposal_preview"],
            "staged_action_categories": ["operation_transactions", "component_actions"],
            "writes": "proposal_staging_only",
            "apply_and_save": "editor_only"
        })
    }

    fn execute(cli: Cli) -> Result<Value, Value> {
        let instance_selection = cli
            .instance
            .or_else(|| std::env::var("AG_ISO_EDITOR_INSTANCE_ID").ok());
        let instance = instance_selection.as_deref();
        let expected_project = cli.project_instance.as_deref();
        let expected_revision = cli.revision;
        let _json = cli.json;
        match cli.command {
            Command::Capabilities => Ok(capabilities()),
            Command::Status => call(instance, "get_project_status", Value::Null, None),
            Command::Context => call_current(
                instance,
                "get_design_context",
                json!({}),
                expected_project,
                expected_revision,
            ),
            Command::Objects { ids } => call_current(
                instance,
                "get_objects",
                json!({"ids":ids}),
                expected_project,
                expected_revision,
            ),
            Command::ObjectSchema { object_type } => call_current(
                instance,
                "get_object_schema",
                json!({"object_type":object_type}),
                expected_project,
                expected_revision,
            ),
            Command::Components { command } => match command {
                ComponentCommand::List {
                    query,
                    category,
                    limit,
                } => call_current(
                    instance,
                    "list_components",
                    json!({"query":query,"category":category,"limit":limit}),
                    expected_project,
                    expected_revision,
                ),
                ComponentCommand::Get {
                    component_id,
                    include_nodes,
                } => call_current(
                    instance,
                    "get_component",
                    json!({"component_id":component_id,"include_nodes":include_nodes}),
                    expected_project,
                    expected_revision,
                ),
            },
            Command::Graphics {
                command:
                    GraphicsCommand::List {
                        query,
                        status,
                        limit,
                    },
            } => call_current(
                instance,
                "list_graphics",
                json!({"query":query,"status":status,"limit":limit}),
                expected_project,
                expected_revision,
            ),
            Command::Diagnostics { scope } => call_current(
                instance,
                "analyze_design",
                json!({"scope":scope.unwrap_or_else(|| "active_mask".into())}),
                expected_project,
                expected_revision,
            ),
            Command::Proposal { command } => match command {
                ProposalCommand::Stage { input } => {
                    let mut actions = read_input(&input)?;
                    let input_project = actions
                        .as_object_mut()
                        .and_then(|map| map.remove("project_instance"))
                        .and_then(|v| v.as_str().map(str::to_owned));
                    let input_revision = actions
                        .as_object_mut()
                        .and_then(|map| map.remove("project_revision"))
                        .and_then(|v| v.as_u64());
                    let expected_project = expected_project.or(input_project.as_deref());
                    let expected_revision = expected_revision.or(input_revision);
                    if expected_project.is_none() || expected_revision.is_none() {
                        return Err(error_value(
                            "PROJECT_GUARD_REQUIRED",
                            "Staging requires project_instance and project_revision from an inspected editor response.",
                            None,
                            None,
                        ));
                    }
                    let action = if actions.get("actions").is_some() {
                        "propose_component_actions"
                    } else {
                        "propose_transaction"
                    };
                    let args = if action == "propose_transaction"
                        && actions.get("transaction").is_none()
                    {
                        let proposal_id = actions
                            .as_object_mut()
                            .and_then(|map| map.remove("proposal_id"));
                        let mut wrapped = json!({"transaction":actions});
                        if let Some(proposal_id) = proposal_id {
                            wrapped["proposal_id"] = proposal_id;
                        }
                        wrapped
                    } else {
                        actions
                    };
                    call_current(instance, action, args, expected_project, expected_revision)
                }
                ProposalCommand::Show => call_current(
                    instance,
                    "proposal_status",
                    json!({}),
                    expected_project,
                    expected_revision,
                ),
                ProposalCommand::Preview { proposal_id } => call_current(
                    instance,
                    "preview_proposal",
                    json!({"proposal_id":proposal_id}),
                    expected_project,
                    expected_revision,
                ),
                ProposalCommand::Reject { proposal_id } => call_current(
                    instance,
                    "reject_proposal",
                    json!({"proposal_id":proposal_id}),
                    expected_project,
                    expected_revision,
                ),
            },
        }
    }

    pub fn run() -> ExitCode {
        let cli = Cli::parse();
        match execute(cli) {
            Ok(value) => {
                println!("{}", value);
                ExitCode::SUCCESS
            }
            Err(error) => {
                println!("{}", error);
                ExitCode::FAILURE
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_machine_readable_inspection_commands() {
            let cli = Cli::try_parse_from(["aitd", "components", "list", "--json"]).unwrap();
            assert!(cli.json);
            assert!(matches!(
                cli.command,
                Command::Components {
                    command: ComponentCommand::List { .. }
                }
            ));
        }

        #[test]
        fn parses_stdin_proposal_staging() {
            let cli = Cli::try_parse_from([
                "aitd",
                "proposal",
                "stage",
                "--input",
                "-",
                "--revision",
                "42",
            ])
            .unwrap();
            assert_eq!(cli.revision, Some(42));
            match cli.command {
                Command::Proposal {
                    command: ProposalCommand::Stage { input },
                } => assert_eq!(input, "-"),
                _ => panic!("expected proposal stage"),
            }
        }

        #[test]
        fn staging_requires_inspected_project_guard() {
            let path =
                std::env::temp_dir().join(format!("aitd-stage-{}.json", uuid::Uuid::new_v4()));
            std::fs::write(&path, r#"{"transaction":{"operations":[]}}"#).unwrap();
            let cli = Cli::try_parse_from([
                "aitd",
                "proposal",
                "stage",
                "--input",
                path.to_str().unwrap(),
            ])
            .unwrap();
            let error = execute(cli).unwrap_err();
            std::fs::remove_file(path).unwrap();
            assert_eq!(error["error"]["code"], "PROJECT_GUARD_REQUIRED");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    native::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
