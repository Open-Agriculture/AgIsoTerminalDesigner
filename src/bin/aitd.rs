#[cfg(not(target_arch = "wasm32"))]
mod native {
    use ag_iso_terminal_designer::ai::automation_bridge::{
        invoke_runtime_request, AutomationRequest, AutomationResponse,
    };
    use clap::{CommandFactory, Parser, Subcommand};
    use serde_json::{json, Value};
    use std::{
        collections::hash_map::DefaultHasher,
        fs,
        hash::{Hash, Hasher},
        io::{self, Read},
        path::{Path, PathBuf},
        process::ExitCode,
        thread,
        time::{Duration, Instant},
    };

    #[derive(Debug, Parser)]
    #[command(
        name = "aitd",
        about = "Inspect and edit a running AgIsoTerminalDesigner project"
    )]
    struct Cli {
        /// Select an editor when more than one is running.
        #[arg(long, global = true)]
        instance: Option<String>,
        /// Guard commands against a different project.
        #[arg(long, global = true)]
        project_instance: Option<String>,
        /// Guard commands against a different project revision.
        #[arg(long, global = true)]
        revision: Option<u64>,
        /// Use a specific editor-owned draft (also AITD_DRAFT).
        #[arg(long, global = true)]
        draft: Option<String>,
        /// Select a proposal when --view proposal is used.
        #[arg(long, global = true)]
        proposal_id: Option<String>,
        /// Inspect live, draft, or proposal content.
        #[arg(long, global = true, default_value = "live", value_parser = ["live", "draft", "proposal"])]
        view: String,
        /// Require this draft generation before editing.
        #[arg(long, global = true)]
        expect_generation: Option<u64>,
        /// Print structured JSON instead of readable text.
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, Subcommand)]
    enum Command {
        /// Show editor and project status.
        Status,
        /// Describe commands supported by this CLI.
        Capabilities,
        /// List reachable desktop editors.
        Editors,
        /// Generate a Bash completion script for this command tree.
        Completions {
            #[arg(value_parser = ["bash"])]
            shell: String,
        },
        /// Inspect a mask or container hierarchy.
        Tree {
            root: u16,
            #[arg(long, default_value_t = 6)]
            depth: usize,
        },
        /// Read a design context.
        Context {
            #[arg(long, default_value = "active_mask")]
            scope: String,
            #[arg(long)]
            root: Option<u16>,
            #[arg(long, default_value_t = 3)]
            depth: usize,
            #[arg(long, default_value_t = 50)]
            limit: usize,
            #[arg(long, default_value_t = 0)]
            offset: usize,
            #[arg(long, default_value_t = 0)]
            layout_offset: usize,
            #[arg(long)]
            include_properties: bool,
            #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
            include_references: bool,
            #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
            include_selection: bool,
        },
        /// Search project objects.
        Object {
            #[command(subcommand)]
            command: ObjectCommand,
        },
        /// Assign, remap, or reorder object IDs.
        Id {
            #[arg(trailing_var_arg = true, required = true)]
            tokens: Vec<String>,
        },
        /// Read writable properties and relationship rules.
        Schema { object_types: Vec<String> },
        /// Begin, inspect, or submit an editor-owned draft.
        Draft {
            #[command(subcommand)]
            command: DraftCommand,
        },
        /// Inspect and manage submitted reviews.
        Proposal {
            #[command(subcommand)]
            command: ProposalCommand,
        },
        /// Validate the live project or selected candidate.
        Validate,
        /// Render a preview to a PNG file.
        Preview {
            #[arg(long)]
            mask: Option<u16>,
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
        /// Save generated IOP, C header, or both.
        Export {
            #[command(subcommand)]
            command: ExportCommand,
        },
        /// Run plain command lines atomically against a draft.
        Run {
            #[arg(long)]
            file: String,
        },
        /// List reusable components.
        Component {
            #[command(subcommand)]
            command: ComponentCommand,
        },
        /// Inspect or edit graphics.
        Graphic {
            #[command(subcommand)]
            command: GraphicsCommand,
        },
        /// Run design diagnostics.
        Diagnostics {
            #[arg(long, default_value = "active_mask")]
            scope: String,
        },
        /// Inspect a specific design region.
        Region {
            root: u16,
            #[arg(long,default_value="all",value_parser=["top","bottom","left","right","center","top_left","top_right","bottom_left","bottom_right","all"])]
            region: String,
            #[arg(long)]
            types: Vec<String>,
            #[arg(long, default_value = "style_match")]
            purpose: String,
            #[arg(long, default_value_t = 12)]
            limit: usize,
        },
        /// Resolve colour indexes in the selected view.
        Colours { indexes: Vec<u8> },
        /// Show project settings or edit object ID ranges.
        Project {
            #[command(subcommand)]
            command: ProjectCommand,
        },
        /// Inspect component instances.
        Instance {
            #[command(subcommand)]
            command: InstanceCommand,
        },
        /// Edit a specific parent-child placement.
        Placement {
            #[command(subcommand)]
            command: PlacementCommand,
        },
        /// Set an object's reference property.
        Reference {
            #[arg(trailing_var_arg = true, required = true)]
            tokens: Vec<String>,
        },
        /// Edit allowed pointer targets.
        Pointer {
            #[arg(trailing_var_arg = true, required = true)]
            tokens: Vec<String>,
        },
        /// Edit an object's ordered reference list.
        List {
            #[arg(trailing_var_arg = true, required = true)]
            tokens: Vec<String>,
        },
        /// Bind or reorder event-to-macro actions.
        Event {
            #[arg(trailing_var_arg = true, required = true)]
            tokens: Vec<String>,
        },
        /// Add or edit object labels.
        Label {
            #[arg(trailing_var_arg = true, required = true)]
            tokens: Vec<String>,
        },
    }
    #[derive(Debug, Subcommand)]
    enum ObjectCommand {
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long = "type")]
            object_type: Option<String>,
            #[arg(long)]
            parent: Option<u16>,
            #[arg(long, default_value_t = 0)]
            offset: usize,
            #[arg(long, default_value_t = 100)]
            limit: usize,
            /// Read all matching pages from one revision.
            #[arg(long)]
            all: bool,
        },
        Get {
            id: String,
            #[arg(long)]
            parents: bool,
            #[arg(long)]
            children: bool,
            #[arg(long)]
            macros: bool,
            #[arg(long)]
            style: bool,
            #[arg(long, default_value_t = 0)]
            offset: usize,
            #[arg(long, default_value_t = 0)]
            parent_offset: usize,
            #[arg(long, default_value_t = 0)]
            property_offset: usize,
        },
        Create {
            object_type: String,
            #[arg(required_unless_present = "named", conflicts_with = "named")]
            name: Option<String>,
            #[arg(long = "name")]
            named: Option<String>,
        },
        Set {
            id: String,
            assignments: Vec<String>,
            /// Read the value for a single named property from a text file.
            #[arg(long)]
            value_file: Option<PathBuf>,
        },
        Rename {
            id: String,
            name: String,
        },
        Delete {
            id: String,
        },
        Order {
            ids: Vec<String>,
        },
        Duplicate {
            id: u16,
            #[arg(long)]
            name: Option<String>,
        },
        Field {
            id: String,
            property: String,
            assignment: String,
        },
        Array {
            #[command(subcommand)]
            command: ObjectArrayCommand,
        },
        BytesImport {
            id: String,
            property: String,
            file: PathBuf,
        },
        /// Save a byte-valued property without JSON encoding.
        BytesExport {
            id: String,
            property: String,
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
    }
    #[derive(Debug, Subcommand)]
    enum ObjectArrayCommand {
        Set {
            object: String,
            property: String,
            index: u16,
            value: String,
        },
        Append {
            object: String,
            property: String,
            value: String,
        },
        Remove {
            object: String,
            property: String,
            index: u16,
        },
        Clear {
            object: String,
            property: String,
        },
    }
    #[derive(Debug, Subcommand)]
    enum ComponentCommand {
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long)]
            category: Option<String>,
            #[arg(long, default_value_t = 50)]
            limit: u8,
        },
        Get {
            id: String,
            #[arg(long)]
            nodes: bool,
        },
        Instantiate {
            component_id: String,
            parent: u16,
            x: i32,
            y: i32,
        },
        Move {
            instance_id: String,
            x: i32,
            y: i32,
        },
        Duplicate {
            instance_id: String,
            x: i32,
            y: i32,
        },
        Delete {
            instance_id: String,
        },
        Detach {
            instance_id: String,
        },
        Override {
            instance_id: String,
            parameter: String,
            value: String,
        },
        Reset {
            instance_id: String,
            parameter: String,
        },
        DefinitionRename {
            component_id: String,
            name: String,
        },
        DefinitionDuplicate {
            component_id: String,
        },
        DefinitionDelete {
            component_id: String,
        },
        NodeSet {
            component_id: String,
            node_id: String,
            property: String,
            value: String,
        },
        /// Assign a node reference to an object, node:UUID, or null.
        NodeReference {
            component_id: String,
            node_id: String,
            property: String,
            target: String,
        },
        ParameterRemove {
            component_id: String,
            parameter_id: String,
        },
        Capture {
            root: u16,
            name: String,
        },
        ParameterAdd {
            component_id: String,
            name: String,
            target_node: String,
            property: String,
            kind: String,
            default_value: String,
        },
        ParameterSet {
            component_id: String,
            parameter_id: String,
            name: String,
            target_node: String,
            property: String,
            kind: String,
            default_value: String,
        },
    }
    #[derive(Debug, Subcommand)]
    enum ProjectCommand {
        /// Identify the active project, original file path, and current save path.
        Current,
        Settings {
            #[command(subcommand)]
            command: Option<ProjectSettingsCommand>,
        },
        Set {
            #[arg(long)]
            mask_size: Option<u16>,
            #[arg(long)]
            vt_version: Option<u8>,
            #[arg(long)]
            soft_key_width: Option<u16>,
            #[arg(long)]
            soft_key_height: Option<u16>,
        },
        IdRangeSet {
            object_type: String,
            start: u16,
            end: u16,
        },
        IdRangeReset,
    }
    #[derive(Debug, Subcommand)]
    enum ProjectSettingsCommand {
        Show,
        /// Change settings with typed key=value assignments.
        Set {
            #[arg(required = true)]
            assignments: Vec<String>,
        },
    }
    #[derive(Debug, Subcommand)]
    enum InstanceCommand {
        List,
        Get { id: String },
    }
    #[derive(Debug, Subcommand)]
    enum PlacementCommand {
        Add {
            parent: String,
            child: String,
            x: i16,
            y: i16,
            #[arg(long)]
            occurrence: Option<usize>,
        },
        Move {
            parent: String,
            child: String,
            x: i16,
            y: i16,
            #[arg(long)]
            occurrence: Option<usize>,
        },
        Remove {
            parent: String,
            child: String,
            #[arg(long)]
            occurrence: Option<usize>,
        },
        Order {
            parent: String,
            children: Vec<String>,
        },
    }
    #[derive(Debug, Subcommand)]
    enum DraftCommand {
        Begin {
            #[arg(long)]
            name: Option<String>,
        },
        Status,
        Diff,
        Submit,
        Reopen,
        Discard,
    }
    #[derive(Debug, Subcommand)]
    enum ProposalCommand {
        Status {
            id: Option<String>,
        },
        /// Show the pending candidate's before/after changes.
        Diff {
            id: Option<String>,
        },
        Wait {
            id: Option<String>,
            #[arg(long, default_value_t = 120)]
            timeout: u64,
        },
        Reject {
            id: String,
        },
    }
    #[derive(Debug, Subcommand)]
    enum GraphicsCommand {
        Get {
            id: String,
        },
        /// Save an available asset or PictureGraphic as a PNG.
        Export {
            id: String,
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long, default_value = "all")]
            status: String,
            #[arg(long, default_value_t = 50)]
            limit: u8,
        },
        Import {
            file: PathBuf,
            #[arg(long)]
            name: Option<String>,
        },
        Assign {
            asset_id: String,
            object: String,
        },
        CreatePicture {
            asset_id: String,
        },
        Rename {
            asset_id: String,
            name: String,
        },
        Delete {
            asset_id: String,
        },
    }
    #[derive(Debug, Subcommand)]
    enum ExportCommand {
        Iop {
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            proposal: Option<String>,
            #[arg(long)]
            force: bool,
        },
        Header {
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            proposal: Option<String>,
            #[arg(long)]
            force: bool,
        },
        Bundle {
            #[arg(long)]
            iop: PathBuf,
            #[arg(long)]
            header: PathBuf,
            #[arg(long)]
            proposal: Option<String>,
            #[arg(long)]
            force: bool,
        },
    }

    fn error(code: &str, message: impl Into<String>) -> Value {
        json!({"error":{"code":code,"message":message.into()}})
    }
    fn response_value(response: AutomationResponse) -> Result<Value, Value> {
        if !response.success {
            let e = response.error.map(|e| json!({"code":e.code,"message":e.message,"expected_revision":e.expected_revision,"current_revision":e.current_revision})).unwrap_or_else(|| json!({"code":"AUTOMATION_FAILED","message":"Editor automation request failed"}));
            return Err(json!({"error":e}));
        }
        Ok(
            json!({"editor_instance":response.editor_instance_id,"project_instance":response.project_instance_id,"project_revision":response.revision,"result":response.result}),
        )
    }
    fn call(
        instance: Option<&str>,
        action: &str,
        args: Value,
        identity: Option<(&str, u64)>,
    ) -> Result<Value, Value> {
        let request = AutomationRequest {
            project_instance_id: identity.map(|x| x.0.to_owned()),
            revision: identity.map(|x| x.1),
            action: action.to_owned(),
            args,
        };
        invoke_runtime_request(instance, request)
            .map_err(|e| json!({"error":{"code":e.code,"message":e.message,"expected_revision":e.expected_revision,"current_revision":e.current_revision}}))
            .and_then(response_value)
    }
    fn current(
        instance: Option<&str>,
        action: &str,
        mut args: Value,
        project: Option<&str>,
        revision: Option<u64>,
    ) -> Result<Value, Value> {
        let status = call(instance, "get_project_status", Value::Null, None)?;
        if !status["result"]["project_open"].as_bool().unwrap_or(false) {
            return Err(error(
                "PROJECT_NOT_OPEN",
                "Open a project in the editor first.",
            ));
        }
        let binding = load_binding();
        let bound_draft = matches!(
            action,
            "edit_draft" | "submit_draft" | "reopen_draft" | "discard_draft" | "draft_diff"
        ) && args["draft_id"].as_str() == binding["draft_id"].as_str();
        let id = project
            .or_else(|| {
                bound_draft
                    .then(|| binding["project_instance"].as_str())
                    .flatten()
            })
            .or_else(|| status["project_instance"].as_str())
            .unwrap_or_default();
        let rev = revision
            .or_else(|| {
                bound_draft
                    .then(|| binding["base_revision"].as_u64())
                    .flatten()
            })
            .or_else(|| status["project_revision"].as_u64())
            .unwrap_or_default();
        if let Some(map) = args.as_object_mut() {
            map.entry("view").or_insert(json!("live"));
        }
        let pinned_instance = instance.or_else(|| status["editor_instance"].as_str());
        call(pinned_instance, action, args, Some((id, rev)))
    }
    fn runtime_selection(cli: &Cli) -> (Option<String>, Option<String>) {
        let bound = load_binding();
        (
            cli.instance
                .clone()
                .or_else(|| std::env::var("AG_ISO_EDITOR_INSTANCE_ID").ok())
                .or_else(|| bound["editor_instance"].as_str().map(str::to_owned)),
            cli.draft
                .clone()
                .or_else(|| std::env::var("AITD_DRAFT").ok()),
        )
    }
    fn draft_binding_path() -> Result<PathBuf, String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let canonical = fs::canonicalize(&cwd).unwrap_or(cwd);
        let mut h = DefaultHasher::new();
        canonical.hash(&mut h);
        let username = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let dir = std::env::temp_dir().join(format!(
            "aitd-{}",
            username
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
        ));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let metadata = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Draft binding directory is not a real directory: {}",
                dir.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(dir.join(format!("cwd-{:016x}.draft", h.finish())))
    }
    fn store_binding(id: &str, generation: u64, envelope: Option<&Value>) -> Result<(), String> {
        let path = draft_binding_path()?;
        let previous = read_binding(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .unwrap_or_else(|| json!({}));
        let mut binding = json!({"draft_id":id,"generation":generation});
        if previous["draft_id"].as_str() == Some(id) {
            for key in [
                "latest_proposal",
                "editor_instance",
                "project_instance",
                "base_revision",
            ] {
                if let Some(p) = previous.get(key) {
                    binding[key] = p.clone();
                }
            }
        }
        if let Some(response) = envelope {
            binding["editor_instance"] = response["editor_instance"].clone();
            binding["project_instance"] = response["result"]["draft"]
                .get("project_instance")
                .unwrap_or(&response["project_instance"])
                .clone();
            binding["base_revision"] = response["result"]["draft"]["base_revision"].clone();
        }
        let bytes = serde_json::to_vec(&binding).map_err(|e| e.to_string())?;
        let temporary = path.with_extension(format!(
            "{}.{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn load_binding() -> Value {
        draft_binding_path()
            .ok()
            .and_then(|p| read_binding(&p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null)
    }
    fn read_binding(path: &Path) -> Result<Vec<u8>, String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "Draft binding is not a regular file: {}",
                path.display()
            ));
        }
        fs::read(path).map_err(|e| e.to_string())
    }
    fn selected_draft(explicit: Option<String>) -> Result<String, Value> {
        if let Some(id) = explicit {
            return Ok(id);
        }
        let path = draft_binding_path().map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
        read_binding(&path)
            .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
            .map(|s| {
                serde_json::from_str::<Value>(&s)
                    .ok()
                    .and_then(|v| v["draft_id"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| s.trim().to_owned())
            })
            .map_err(|_| {
                error(
                    "NO_ACTIVE_DRAFT",
                    "Run `aitd draft begin` or pass --draft ID.",
                )
            })
    }
    fn cached_generation_for(id: &str) -> Option<u64> {
        let path = draft_binding_path().ok()?;
        let value = serde_json::from_slice::<Value>(&read_binding(&path).ok()?).ok()?;
        (value["draft_id"].as_str() == Some(id))
            .then(|| value["generation"].as_u64())
            .flatten()
    }
    fn draft_generation(value: &Value) -> Option<u64> {
        value["result"]["draft"]["generation"]
            .as_u64()
            .or_else(|| value["result"]["generation"].as_u64())
    }
    fn selected_generation(instance: Option<&str>, id: &str) -> Result<u64, Value> {
        if let Some(generation) = cached_generation_for(id) {
            return Ok(generation);
        }
        let status = call(instance, "draft_status", json!({"draft_id":id}), None)?;
        let generation = draft_generation(&status)
            .ok_or_else(|| error("DRAFT_RESPONSE_INVALID", "Draft generation is missing"))?;
        store_binding(id, generation, Some(&status))
            .map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
        Ok(generation)
    }
    fn remember_proposal(id: &str) -> Result<(), String> {
        let path = draft_binding_path()?;
        let mut value =
            serde_json::from_slice::<Value>(&read_binding(&path)?).map_err(|e| e.to_string())?;
        value["latest_proposal"] = json!(id);
        let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        let temporary = path.with_extension(format!(
            "{}.{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(temporary, path).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn selected_proposal(explicit: Option<String>) -> Result<String, Value> {
        if let Some(id) = explicit {
            return Ok(id);
        }
        let p = draft_binding_path().map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
        let v = serde_json::from_slice::<Value>(
            &read_binding(&p)
                .map_err(|_| error("NO_PROPOSAL", "Submit a draft or pass a proposal ID."))?,
        )
        .map_err(|e| error("DRAFT_BINDING_INVALID", e.to_string()))?;
        v["latest_proposal"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| error("NO_PROPOSAL", "Submit a draft or pass a proposal ID."))
    }
    fn scoped(mut args: Value, view: &str, draft: Option<&str>, proposal: Option<&str>) -> Value {
        args["view"] = json!(view);
        if let Some(id) = draft {
            args["draft_id"] = json!(id);
        }
        if let Some(id) = proposal {
            args["proposal_id"] = json!(id);
        }
        args
    }
    fn perform_edit(
        instance: Option<&str>,
        project: Option<&str>,
        revision: Option<u64>,
        draft: &str,
        tokens: &[String],
    ) -> Result<Value, Value> {
        let actions = ag_iso_terminal_designer::ai::commands::parse_edit(tokens)
            .map_err(|e| error("INVALID_USAGE", e))?;
        edit_actions(instance, project, revision, draft, actions)
    }
    fn perform_group_edit(
        instance: Option<&str>,
        project: Option<&str>,
        revision: Option<u64>,
        draft_override: Option<String>,
        group: &str,
        tokens: Vec<String>,
    ) -> Result<Value, Value> {
        let draft = selected_draft(draft_override)?;
        let all = std::iter::once(group.to_owned())
            .chain(tokens)
            .collect::<Vec<_>>();
        perform_edit(instance, project, revision, &draft, &all)
    }
    fn edit_actions(
        instance: Option<&str>,
        project: Option<&str>,
        revision: Option<u64>,
        draft: &str,
        actions: Vec<Value>,
    ) -> Result<Value, Value> {
        let generation = selected_generation(instance, draft)?;
        let response = current(
            instance,
            "edit_draft",
            json!({"draft_id":draft,"generation":generation,"actions":actions}),
            project,
            revision,
        )?;
        if let Some(next) = response["result"]["draft"]["generation"]
            .as_u64()
            .or_else(|| response["result"]["generation"].as_u64())
        {
            store_binding(draft, next, Some(&response))
                .map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
        }
        Ok(response)
    }

    fn parse_target_token(value: &str) -> Value {
        if let Some(name) = value.strip_prefix("name:") {
            json!({"name":name})
        } else if let Some(hex) = value.strip_prefix("0x") {
            u16::from_str_radix(hex, 16)
                .map(|id| json!({"id":id}))
                .unwrap_or_else(|_| json!({"name":value}))
        } else {
            value
                .parse::<u16>()
                .map(|id| json!({"id":id}))
                .unwrap_or_else(|_| json!({"name":value}))
        }
    }
    fn execute(cli: Cli) -> Result<Value, Value> {
        let selected_view = cli.view.clone();
        let expect_generation = cli.expect_generation;
        let proposal_id = cli.proposal_id.clone();
        let draft_for_view = if selected_view == "draft" {
            Some(selected_draft(
                cli.draft
                    .clone()
                    .or_else(|| std::env::var("AITD_DRAFT").ok()),
            )?)
        } else {
            None
        };
        let export_revision = cli.revision;
        let (instance, draft_override) = runtime_selection(&cli);
        let instance = instance.as_deref();
        if let Some(expected) = expect_generation {
            let id = selected_draft(draft_override.clone())?;
            let status = call(instance, "draft_status", json!({"draft_id":id}), None)?;
            if draft_generation(&status) != Some(expected) {
                return Err(error(
                    "DRAFT_CONFLICT",
                    format!("Expected generation {expected}; inspect the draft before retrying."),
                ));
            }
            store_binding(&id, expected, Some(&status))
                .map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
        }
        let project = cli.project_instance.as_deref();
        let revision = cli.revision;
        match cli.command {
            Command::Capabilities => Ok(
                json!({"automation_api_version":2,"commands":Cli::command().get_subcommands().map(|command|json!({"name":command.get_name(),"description":command.get_about().map(ToString::to_string),"subcommands":command.get_subcommands().map(|sub|sub.get_name()).collect::<Vec<_>>()})).collect::<Vec<_>>(),"json_input":false,"max_draft_actions":256,"apply_and_project_save":"editor_only","exports":["iop","header","bundle"]}),
            ),
            Command::Status => call(instance, "get_project_status", Value::Null, None),
            Command::Editors => {
                ag_iso_terminal_designer::ai::automation_bridge::list_runtime_instances()
                    .map(|ids| json!({"result":{"editors":ids}}))
                    .map_err(|e| error("EDITOR_CONNECTION_FAILED", e))
            }
            Command::Completions { shell } => {
                Ok(json!({"result":{"shell":shell,"script":bash_completions()}}))
            }
            Command::Tree { root, depth } => current(
                instance,
                "get_design_context",
                scoped(
                    json!({"root_id":root,"depth":depth,"max_objects":200}),
                    &selected_view,
                    draft_for_view.as_deref(),
                    proposal_id.as_deref(),
                ),
                project,
                revision,
            ),
            Command::Context {
                scope,
                root,
                depth,
                limit,
                offset,
                layout_offset,
                include_properties,
                include_references,
                include_selection,
            } => current(
                instance,
                "get_design_context",
                scoped(
                    json!({"scope":scope,"root_id":root,"depth":depth,"max_objects":limit,"offset":offset,"layout_offset":layout_offset,"include_properties":include_properties,"include_references":include_references,"include_selection":include_selection}),
                    &selected_view,
                    draft_for_view.as_deref(),
                    proposal_id.as_deref(),
                ),
                project,
                revision,
            ),
            Command::Schema { object_types } => {
                let action = if !object_types.is_empty() && object_types != ["types"] {
                    "get_object_schema"
                } else {
                    "get_object_types"
                };
                current(
                    instance,
                    action,
                    scoped(
                        json!({"object_types":object_types}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                )
            }
            Command::Object { command } => match command {
                ObjectCommand::List {
                    query,
                    object_type,
                    parent,
                    offset,
                    limit,
                    all,
                } => {
                    let mut args = scoped(
                        json!({"query":query,"type":object_type,"parent_id":parent,"offset":offset,"limit":limit}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    );
                    let mut result =
                        current(instance, "list_objects", args.clone(), project, revision)?;
                    let identity = result["project_instance"].as_str().unwrap().to_owned();
                    let inspected_revision = result["project_revision"].as_u64().unwrap();
                    let editor = result["editor_instance"].as_str().unwrap().to_owned();
                    let generation = result["result"]["generation"].clone();
                    let mut objects = result["result"]["objects"].as_array().unwrap().clone();
                    while all && result["result"]["has_more"].as_bool() == Some(true) {
                        args["offset"] = json!(offset + objects.len());
                        let next = current(
                            Some(&editor),
                            "list_objects",
                            args.clone(),
                            Some(&identity),
                            Some(inspected_revision),
                        )?;
                        if next["result"]["generation"] != generation {
                            return Err(error(
                                "DRAFT_CONFLICT",
                                "Draft changed while paging objects; inspect it again",
                            ));
                        }
                        let page = next["result"]["objects"].as_array().unwrap();
                        if page.is_empty() {
                            return Err(error(
                                "AUTOMATION_FAILED",
                                "Object pagination did not advance",
                            ));
                        }
                        objects.extend(page.iter().cloned());
                        result = next;
                    }
                    result["result"]["objects"] = json!(objects);
                    result["result"]["offset"] = json!(offset);
                    Ok(result)
                }
                ObjectCommand::Get {
                    id,
                    parents,
                    children,
                    macros,
                    style,
                    offset,
                    parent_offset,
                    property_offset,
                } => current(
                    instance,
                    "get_objects",
                    scoped(
                        json!({"targets":[parse_target_token(&id)],"include_parents":parents,"include_children":children,"include_macros":macros,"include_nearby_style":style,"detail_offset":offset,"parent_offset":parent_offset,"property_offset":property_offset}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
                ObjectCommand::Create {
                    object_type,
                    name,
                    named,
                } => {
                    let draft = selected_draft(draft_override)?;
                    perform_edit(
                        instance,
                        project,
                        revision,
                        &draft,
                        &[
                            "object".into(),
                            "create".into(),
                            object_type,
                            name.or(named).unwrap(),
                        ],
                    )
                }
                ObjectCommand::Set {
                    id,
                    mut assignments,
                    value_file,
                } => {
                    if let Some(path) = value_file {
                        if assignments.len() != 1 || assignments[0].contains('=') {
                            return Err(error("INVALID_USAGE","Use object set TARGET PROPERTY --value-file PATH for one text value."));
                        }
                        let text = fs::read_to_string(path)
                            .map_err(|e| error("INPUT_FAILED", e.to_string()))?;
                        assignments[0] = format!("{}={text}", assignments[0]);
                    }
                    let draft = selected_draft(draft_override)?;
                    let mut tokens = vec!["object".into(), "set".into(), id];
                    tokens.extend(assignments);
                    perform_edit(instance, project, revision, &draft, &tokens)
                }
                ObjectCommand::Rename { id, name } => {
                    let draft = selected_draft(draft_override)?;
                    perform_edit(
                        instance,
                        project,
                        revision,
                        &draft,
                        &["object".into(), "rename".into(), id, name],
                    )
                }
                ObjectCommand::Delete { id } => {
                    let draft = selected_draft(draft_override)?;
                    perform_edit(
                        instance,
                        project,
                        revision,
                        &draft,
                        &["object".into(), "delete".into(), id],
                    )
                }
                ObjectCommand::Order { ids } => {
                    let mut tokens = vec!["object".into(), "order".into(), "set".into()];
                    tokens.extend(ids);
                    let draft = selected_draft(draft_override)?;
                    perform_edit(instance, project, revision, &draft, &tokens)
                }
                ObjectCommand::Duplicate { id, name } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"object_duplicate","object_id":id,"name":name})],
                ),
                ObjectCommand::Field {
                    id,
                    property,
                    assignment,
                } => {
                    let draft = selected_draft(draft_override)?;
                    perform_edit(
                        instance,
                        project,
                        revision,
                        &draft,
                        &[
                            "object".into(),
                            "field".into(),
                            "set".into(),
                            id,
                            property,
                            assignment,
                        ],
                    )
                }
                ObjectCommand::Array { command } => {
                    let tokens = match command {
                        ObjectArrayCommand::Set {
                            object,
                            property,
                            index,
                            value,
                        } => vec![
                            "object".into(),
                            "array".into(),
                            "set".into(),
                            object,
                            property,
                            index.to_string(),
                            value,
                        ],
                        ObjectArrayCommand::Append {
                            object,
                            property,
                            value,
                        } => vec![
                            "object".into(),
                            "array".into(),
                            "append".into(),
                            object,
                            property,
                            value,
                        ],
                        ObjectArrayCommand::Remove {
                            object,
                            property,
                            index,
                        } => vec![
                            "object".into(),
                            "array".into(),
                            "remove".into(),
                            object,
                            property,
                            index.to_string(),
                        ],
                        ObjectArrayCommand::Clear { object, property } => vec![
                            "object".into(),
                            "array".into(),
                            "clear".into(),
                            object,
                            property,
                        ],
                    };
                    let draft = selected_draft(draft_override)?;
                    perform_edit(instance, project, revision, &draft, &tokens)
                }
                ObjectCommand::BytesImport { id, property, file } => {
                    let bytes = fs::read(file).map_err(|e| error("INPUT_FAILED", e.to_string()))?;
                    edit_actions(
                        instance,
                        project,
                        revision,
                        &selected_draft(draft_override)?,
                        vec![
                            json!({"kind":"object_bytes_set","object":parse_target_token(&id),"property":property,"bytes_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes)}),
                        ],
                    )
                }
                ObjectCommand::BytesExport {
                    id,
                    property,
                    output,
                    force,
                } => {
                    let response = current(
                        instance,
                        "export_object_bytes",
                        scoped(
                            json!({"object":parse_target_token(&id),"property":property}),
                            &selected_view,
                            draft_for_view.as_deref(),
                            proposal_id.as_deref(),
                        ),
                        project,
                        revision,
                    )?;
                    let bytes = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        response["result"]["bytes_base64"].as_str().unwrap(),
                    )
                    .map_err(|e| error("EXPORT_RESPONSE_INVALID", e.to_string()))?;
                    write_output(&output, &bytes, force)?;
                    Ok(
                        json!({"editor_instance":response["editor_instance"],"project_instance":response["project_instance"],"project_revision":response["project_revision"],"result":{"output":output,"bytes":bytes.len(),"view":selected_view}}),
                    )
                }
            },
            Command::Id { tokens } => {
                let draft = selected_draft(draft_override)?;
                perform_edit(
                    instance,
                    project,
                    revision,
                    &draft,
                    &["id".into(), tokens[0].clone()]
                        .into_iter()
                        .chain(tokens.into_iter().skip(1))
                        .collect::<Vec<_>>(),
                )
            }
            Command::Draft { command } => match command {
                DraftCommand::Begin { name } => {
                    let result = current(
                        instance,
                        "begin_draft",
                        json!({"name":name}),
                        project,
                        revision,
                    )?;
                    if let Some(id) = result["result"]["draft"]["draft_id"].as_str() {
                        let generation = result["result"]["draft"]["generation"]
                            .as_u64()
                            .unwrap_or(0);
                        store_binding(id, generation, Some(&result))
                            .map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
                    }
                    Ok(result)
                }
                DraftCommand::Status => {
                    let id = selected_draft(draft_override)?;
                    let result = call(instance, "draft_status", json!({"draft_id":id}), None)?;
                    if let Some(generation) = draft_generation(&result) {
                        store_binding(&id, generation, Some(&result))
                            .map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
                    }
                    Ok(result)
                }
                DraftCommand::Diff => {
                    let id = selected_draft(draft_override)?;
                    current(
                        instance,
                        "draft_diff",
                        json!({"draft_id":id}),
                        project,
                        revision,
                    )
                }
                DraftCommand::Submit => {
                    let id = selected_draft(draft_override)?;
                    let g = selected_generation(instance, &id)?;
                    let submitted = current(
                        instance,
                        "submit_draft",
                        json!({"draft_id":id,"generation":g}),
                        project,
                        revision,
                    )?;
                    if let Some(pid) = submitted["result"]["proposal_id"].as_str() {
                        remember_proposal(pid).map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
                    }
                    Ok(submitted)
                }
                DraftCommand::Reopen => {
                    let id = selected_draft(draft_override)?;
                    let g = selected_generation(instance, &id)?;
                    let reopened = current(
                        instance,
                        "reopen_draft",
                        json!({"draft_id":id,"generation":g}),
                        project,
                        revision,
                    )?;
                    if let Some(g) = draft_generation(&reopened) {
                        store_binding(&id, g, Some(&reopened))
                            .map_err(|e| error("DRAFT_BINDING_FAILED", e))?;
                    }
                    Ok(reopened)
                }
                DraftCommand::Discard => {
                    let id = selected_draft(draft_override)?;
                    let g = selected_generation(instance, &id)?;
                    let result = current(
                        instance,
                        "discard_draft",
                        json!({"draft_id":id,"generation":g}),
                        project,
                        revision,
                    )?;
                    if let Ok(path) = draft_binding_path() {
                        let _ = fs::remove_file(path);
                    }
                    Ok(result)
                }
            },
            Command::Proposal { command } => match command {
                ProposalCommand::Status { id } => {
                    let id = if id.is_some() {
                        id
                    } else {
                        Some(selected_proposal(None)?)
                    };
                    call(
                        instance,
                        "proposal_outcome",
                        json!({"proposal_id":id}),
                        None,
                    )
                }
                ProposalCommand::Reject { id } => current(
                    instance,
                    "reject_proposal",
                    json!({"proposal_id":id}),
                    project,
                    revision,
                ),
                ProposalCommand::Diff { id } => current(
                    instance,
                    "proposal_diff",
                    json!({"proposal_id":selected_proposal(id)?}),
                    project,
                    revision,
                ),
                ProposalCommand::Wait { id, timeout } => {
                    let id = selected_proposal(id)?;
                    let until = Instant::now() + Duration::from_secs(timeout);
                    loop {
                        let v = call(
                            instance,
                            "proposal_outcome",
                            json!({"proposal_id":id}),
                            None,
                        )?;
                        let state = v["result"]["outcome"]["status"]
                            .as_str()
                            .or_else(|| v["result"]["status"].as_str())
                            .unwrap_or("unknown");
                        if state != "pending" {
                            if state == "applied" {
                                return Ok(v);
                            }
                            return Err(
                                json!({"error":{"code":state.to_ascii_uppercase(),"message":format!("Proposal finished with outcome '{state}'."),"details":v["result"]}}),
                            );
                        }
                        if Instant::now() >= until {
                            return Err(error(
                                "WAIT_TIMEOUT",
                                format!("Proposal {id} is still pending."),
                            ));
                        }
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            },
            Command::Component { command } => match command {
                ComponentCommand::List {
                    query,
                    category,
                    limit,
                } => current(
                    instance,
                    "list_components",
                    scoped(
                        json!({"query":query,"category":category,"limit":limit}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
                ComponentCommand::Get { id, nodes } => current(
                    instance,
                    "get_component",
                    scoped(
                        json!({"component_id":id,"include_nodes":nodes}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
                ComponentCommand::Instantiate {
                    component_id,
                    parent,
                    x,
                    y,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component","action":{"type":"InstantiateComponent","component_id":component_id,"parent":parent,"x":x,"y":y}}),
                    ],
                ),
                ComponentCommand::Move { instance_id, x, y } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"component_move","instance_id":instance_id,"x":x,"y":y})],
                ),
                ComponentCommand::Duplicate { instance_id, x, y } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_duplicate","instance_id":instance_id,"x":x,"y":y}),
                    ],
                ),
                ComponentCommand::Delete { instance_id } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"component_delete","instance_id":instance_id})],
                ),
                ComponentCommand::Detach { instance_id } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"component_detach","instance_id":instance_id})],
                ),
                ComponentCommand::Override {
                    instance_id,
                    parameter,
                    value,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_override","instance_id":instance_id,"parameter":parameter,"value":value}),
                    ],
                ),
                ComponentCommand::Reset {
                    instance_id,
                    parameter,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_reset","instance_id":instance_id,"parameter":parameter}),
                    ],
                ),
                ComponentCommand::DefinitionRename { component_id, name } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_definition_rename","component_id":component_id,"name":name}),
                    ],
                ),
                ComponentCommand::DefinitionDuplicate { component_id } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_definition_duplicate","component_id":component_id}),
                    ],
                ),
                ComponentCommand::DefinitionDelete { component_id } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"component_definition_delete","component_id":component_id})],
                ),
                ComponentCommand::NodeSet {
                    component_id,
                    node_id,
                    property,
                    value,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_node_set","component_id":component_id,"node_id":node_id,"property":property,"value_lexical":value}),
                    ],
                ),
                ComponentCommand::NodeReference {
                    component_id,
                    node_id,
                    property,
                    target,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_node_reference","component_id":component_id,"node_id":node_id,"property":property,"target":target}),
                    ],
                ),
                ComponentCommand::ParameterRemove {
                    component_id,
                    parameter_id,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_parameter_remove","component_id":component_id,"parameter_id":parameter_id}),
                    ],
                ),
                ComponentCommand::Capture { root, name } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"component_capture","root":root,"name":name})],
                ),
                ComponentCommand::ParameterAdd {
                    component_id,
                    name,
                    target_node,
                    property,
                    kind,
                    default_value,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_parameter_add","component_id":component_id,"parameter":{"name":name,"description":null,"target_node":target_node,"property":property,"kind":kind,"default_value_lexical":default_value}}),
                    ],
                ),
                ComponentCommand::ParameterSet {
                    component_id,
                    parameter_id,
                    name,
                    target_node,
                    property,
                    kind,
                    default_value,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"component_parameter_set","component_id":component_id,"parameter":{"id":parameter_id,"name":name,"description":null,"target_node":target_node,"property":property,"kind":kind,"default_value_lexical":default_value}}),
                    ],
                ),
            },
            Command::Graphic { command } => match command {
                GraphicsCommand::Get { id } => current(
                    instance,
                    "get_graphic",
                    scoped(
                        json!({"id":id}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
                GraphicsCommand::Export { id, output, force } => {
                    let response = current(
                        instance,
                        "get_graphic",
                        scoped(
                            json!({"id":id,"export":true}),
                            &selected_view,
                            draft_for_view.as_deref(),
                            proposal_id.as_deref(),
                        ),
                        project,
                        revision,
                    )?;
                    let encoded = response["result"]["graphic"]["png_base64"]
                        .as_str()
                        .ok_or_else(|| {
                            error(
                                "GRAPHIC_EXPORT_FAILED",
                                "Graphic response omitted PNG bytes",
                            )
                        })?;
                    let bytes =
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                            .map_err(|e| error("GRAPHIC_EXPORT_FAILED", e.to_string()))?;
                    write_output(&output, &bytes, force)?;
                    Ok(
                        json!({"editor_instance":response["editor_instance"],"project_instance":response["project_instance"],"project_revision":response["project_revision"],"result":{"output":output,"bytes":bytes.len(),"view":selected_view}}),
                    )
                }
                GraphicsCommand::List {
                    query,
                    status,
                    limit,
                } => current(
                    instance,
                    "list_graphics",
                    scoped(
                        json!({"query":query,"status":status,"limit":limit}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
                GraphicsCommand::Import { file, name } => {
                    let bytes =
                        fs::read(&file).map_err(|e| error("INPUT_FAILED", e.to_string()))?;
                    let action = json!({"kind":"graphic_import","filename":file.file_name().and_then(|x|x.to_str()).unwrap_or("graphic"),"bytes_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes),"name":name});
                    edit_actions(
                        instance,
                        project,
                        revision,
                        &selected_draft(draft_override)?,
                        vec![action],
                    )
                }
                GraphicsCommand::Assign { asset_id, object } => {
                    let id = parse_target_token(&object)["id"].as_u64().ok_or_else(|| {
                        error(
                            "INVALID_USAGE",
                            "graphic assign requires a numeric object ID.",
                        )
                    })?;
                    edit_actions(
                        instance,
                        project,
                        revision,
                        &selected_draft(draft_override)?,
                        vec![json!({"kind":"graphic_assign","asset_id":asset_id,"object":id})],
                    )
                }
                GraphicsCommand::CreatePicture { asset_id } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"graphic_create_picture","asset_id":asset_id})],
                ),
                GraphicsCommand::Rename { asset_id, name } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"graphic_rename","asset_id":asset_id,"name":name})],
                ),
                GraphicsCommand::Delete { asset_id } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"graphic_delete","asset_id":asset_id})],
                ),
            },
            Command::Diagnostics { scope } => current(
                instance,
                "analyze_design",
                scoped(
                    json!({"scope":scope}),
                    &selected_view,
                    draft_for_view.as_deref(),
                    proposal_id.as_deref(),
                ),
                project,
                revision,
            ),
            Command::Validate => {
                let args = if selected_view == "live" {
                    json!({"view":selected_view})
                } else {
                    let id = draft_for_view.clone();
                    scoped(
                        json!({}),
                        &selected_view,
                        id.as_deref(),
                        proposal_id.as_deref(),
                    )
                };
                current(instance, "validate_design", args, project, revision)
            }
            Command::Preview {
                mask,
                output,
                force,
            } => {
                let draft = if selected_view == "live" {
                    None
                } else if selected_view == "draft" {
                    draft_for_view.clone()
                } else {
                    None
                };
                let result = current(
                    instance,
                    "preview_design",
                    scoped(
                        json!({"mask_id":mask}),
                        &selected_view,
                        draft.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                )?;
                let encoded = result["result"]["png_base64"].as_str().ok_or_else(|| {
                    error(
                        "PREVIEW_RESPONSE_INVALID",
                        "Preview response omitted png_base64.",
                    )
                })?;
                let bytes =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                        .map_err(|e| error("PREVIEW_RESPONSE_INVALID", e.to_string()))?;
                write_output(&output, &bytes, force)?;
                Ok(
                    json!({"output":output,"bytes":bytes.len(),"project_revision":result["project_revision"]}),
                )
            }
            Command::Export { command } => export(instance, project, export_revision, command),
            Command::Run { file } => run_file(instance, project, revision, draft_override, &file),
            Command::Region {
                root,
                region,
                types,
                purpose,
                limit,
            } => current(
                instance,
                "inspect_design_region",
                scoped(
                    json!({"root_id":root,"region":region,"types":types,"purpose":purpose,"max_results":limit}),
                    &selected_view,
                    draft_for_view.as_deref(),
                    proposal_id.as_deref(),
                ),
                project,
                revision,
            ),
            Command::Colours { indexes } => current(
                instance,
                "resolve_colours",
                scoped(
                    json!({"indexes":indexes}),
                    &selected_view,
                    draft_for_view.as_deref(),
                    proposal_id.as_deref(),
                ),
                project,
                revision,
            ),
            Command::Project { command } => match command {
                ProjectCommand::Current => call(instance, "get_project_status", Value::Null, None),
                ProjectCommand::Settings {
                    command: None | Some(ProjectSettingsCommand::Show),
                } => current(
                    instance,
                    "project_settings",
                    scoped(
                        json!({}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
                ProjectCommand::Settings {
                    command: Some(ProjectSettingsCommand::Set { assignments }),
                } => {
                    let mut tokens = vec!["project".into(), "settings".into(), "set".into()];
                    tokens.extend(assignments);
                    perform_edit(
                        instance,
                        project,
                        revision,
                        &selected_draft(draft_override)?,
                        &tokens,
                    )
                }
                ProjectCommand::IdRangeSet {
                    object_type,
                    start,
                    end,
                } => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![
                        json!({"kind":"id_range_set","object_type":object_type,"start":start,"end":end}),
                    ],
                ),
                ProjectCommand::IdRangeReset => edit_actions(
                    instance,
                    project,
                    revision,
                    &selected_draft(draft_override)?,
                    vec![json!({"kind":"id_range_reset"})],
                ),
                ProjectCommand::Set {
                    mask_size,
                    vt_version,
                    soft_key_width,
                    soft_key_height,
                } => {
                    let mut values = serde_json::Map::new();
                    for (key, value) in [
                        ("mask_size", mask_size.map(|v| json!(v))),
                        ("vt_version", vt_version.map(|v| json!(v))),
                        ("soft_key_width", soft_key_width.map(|v| json!(v))),
                        ("soft_key_height", soft_key_height.map(|v| json!(v))),
                    ] {
                        if let Some(v) = value {
                            values.insert(key.into(), v);
                        }
                    }
                    edit_actions(
                        instance,
                        project,
                        revision,
                        &selected_draft(draft_override)?,
                        vec![json!({"kind":"settings","values":values})],
                    )
                }
            },
            Command::Instance { command } => match command {
                InstanceCommand::List => current(
                    instance,
                    "get_component_instances",
                    scoped(
                        json!({}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
                InstanceCommand::Get { id } => current(
                    instance,
                    "get_component_instances",
                    scoped(
                        json!({"instance_id":id}),
                        &selected_view,
                        draft_for_view.as_deref(),
                        proposal_id.as_deref(),
                    ),
                    project,
                    revision,
                ),
            },
            Command::Placement { command } => {
                let tokens: Vec<String> = match command {
                    PlacementCommand::Add {
                        parent,
                        child,
                        x,
                        y,
                        occurrence,
                    } => vec![
                        "placement".into(),
                        "add".into(),
                        parent,
                        child,
                        x.to_string(),
                        y.to_string(),
                    ]
                    .into_iter()
                    .chain(
                        occurrence
                            .into_iter()
                            .flat_map(|n| vec!["--occurrence".into(), n.to_string()]),
                    )
                    .collect(),
                    PlacementCommand::Move {
                        parent,
                        child,
                        x,
                        y,
                        occurrence,
                    } => vec![
                        "placement".into(),
                        "move".into(),
                        parent,
                        child,
                        x.to_string(),
                        y.to_string(),
                    ]
                    .into_iter()
                    .chain(
                        occurrence
                            .into_iter()
                            .flat_map(|n| vec!["--occurrence".into(), n.to_string()]),
                    )
                    .collect(),
                    PlacementCommand::Remove {
                        parent,
                        child,
                        occurrence,
                    } => vec!["placement".into(), "remove".into(), parent, child]
                        .into_iter()
                        .chain(
                            occurrence
                                .into_iter()
                                .flat_map(|n| vec!["--occurrence".into(), n.to_string()]),
                        )
                        .collect(),
                    PlacementCommand::Order { parent, children } => {
                        vec!["placement".into(), "order".into(), parent]
                            .into_iter()
                            .chain(children.into_iter())
                            .collect()
                    }
                };
                let id = selected_draft(draft_override)?;
                let _ = expect_generation;
                perform_edit(instance, project, revision, &id, &tokens)
            }
            Command::Reference { tokens } => perform_group_edit(
                instance,
                project,
                revision,
                draft_override,
                "reference",
                tokens,
            ),
            Command::Pointer { tokens } => perform_group_edit(
                instance,
                project,
                revision,
                draft_override,
                "pointer",
                tokens,
            ),
            Command::List { tokens } => {
                perform_group_edit(instance, project, revision, draft_override, "list", tokens)
            }
            Command::Event { tokens } => {
                perform_group_edit(instance, project, revision, draft_override, "event", tokens)
            }
            Command::Label { tokens } => {
                perform_group_edit(instance, project, revision, draft_override, "label", tokens)
            }
        }
    }

    fn write_output(path: &Path, bytes: &[u8], force: bool) -> Result<(), Value> {
        if path.exists() && !force {
            return Err(error(
                "OUTPUT_EXISTS",
                format!("{} exists; pass --force to replace it.", path.display()),
            ));
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        }
        let tmp = temporary_output_path(path);
        fs::write(&tmp, bytes).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        fs::rename(&tmp, path).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        Ok(())
    }
    fn temporary_output_path(path: &Path) -> PathBuf {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        path.with_file_name(format!(
            ".{name}.aitd-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }
    fn export(
        instance: Option<&str>,
        project: Option<&str>,
        revision: Option<u64>,
        cmd: ExportCommand,
    ) -> Result<Value, Value> {
        let (format, outputs, proposal, force) = match cmd {
            ExportCommand::Iop {
                output,
                proposal,
                force,
            } => ("iop", vec![("iop", output)], proposal, force),
            ExportCommand::Header {
                output,
                proposal,
                force,
            } => ("header", vec![("header", output)], proposal, force),
            ExportCommand::Bundle {
                iop,
                header,
                proposal,
                force,
            } => (
                "bundle",
                vec![("iop", iop), ("header", header)],
                proposal,
                force,
            ),
        };
        if outputs.len() == 2 && outputs[0].1 == outputs[1].1 {
            return Err(error(
                "INVALID_USAGE",
                "IOP and header outputs must use different paths.",
            ));
        }
        for (_, p) in &outputs {
            if p.exists() && !force {
                return Err(error(
                    "OUTPUT_EXISTS",
                    format!("{} exists; pass --force to replace it.", p.display()),
                ));
            }
        }
        let v = current(
            instance,
            "export_artifacts",
            json!({"format":format,"proposal_id":proposal,"expected_revision":revision}),
            project,
            revision,
        )?;
        let artifacts = &v["result"]["artifacts"];
        let mut staged = Vec::new();
        for (key, path) in outputs {
            let text = artifacts[key].as_str().ok_or_else(|| {
                error(
                    "EXPORT_RESPONSE_INVALID",
                    format!("export omitted {key} artifact"),
                )
            })?;
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text)
                .map_err(|e| error("EXPORT_RESPONSE_INVALID", e.to_string()))?;
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
            }
            let tmp = temporary_output_path(&path);
            fs::write(&tmp, &bytes).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
            staged.push((tmp, path, bytes.len()));
        }
        for (tmp, path, _) in &staged {
            fs::rename(tmp, path)
                .map_err(|e| error("OUTPUT_FAILED", format!("{}: {e}", path.display())))?;
        }
        Ok(
            json!({"editor_instance":v["editor_instance"],"project_instance":v["project_instance"],"project_revision":v["project_revision"],"result":{"outputs":staged.iter().map(|(_,p,n)|json!({"path":p,"bytes":n})).collect::<Vec<_>>(),"vt_version":v["result"]["vt_version"],"proposal_id":v["result"]["proposal_id"]}}),
        )
    }

    fn split_line(line: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut quote = None;
        let mut escape = false;
        let mut started = false;
        for c in line.chars() {
            if escape {
                cur.push(c);
                escape = false;
                started = true;
                continue;
            }
            if c == '\\' {
                escape = true;
                started = true;
                continue;
            }
            if let Some(q) = quote {
                if c == q {
                    quote = None
                } else {
                    cur.push(c)
                }
                started = true;
                continue;
            }
            if c == '\'' || c == '"' {
                quote = Some(c);
                started = true;
                continue;
            }
            if c.is_whitespace() {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false
                }
            } else {
                cur.push(c);
                started = true
            }
        }
        if escape {
            return Err("trailing escape".into());
        }
        if quote.is_some() {
            return Err("unclosed quote".into());
        }
        if started {
            out.push(cur)
        }
        Ok(out)
    }
    fn run_file(
        instance: Option<&str>,
        project: Option<&str>,
        revision: Option<u64>,
        draft_override: Option<String>,
        file: &str,
    ) -> Result<Value, Value> {
        let (contents, base) = if file == "-" {
            let mut s = String::new();
            io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| error("INPUT_FAILED", e.to_string()))?;
            (s, std::env::current_dir().unwrap_or_default())
        } else {
            let p = PathBuf::from(file);
            (
                fs::read_to_string(&p).map_err(|e| error("INPUT_FAILED", e.to_string()))?,
                p.parent().unwrap_or(Path::new(".")).to_path_buf(),
            )
        };
        let draft = selected_draft(draft_override)?;
        let mut actions = Vec::new();
        let mut logical_lines = Vec::new();
        let mut pending = String::new();
        let mut start_line = 1;
        for (index, line) in contents.lines().enumerate() {
            if pending.is_empty() {
                start_line = index + 1;
            }
            let continuation = line.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1;
            if continuation {
                pending.push_str(&line[..line.len() - 1]);
                pending.push(' ');
            } else {
                pending.push_str(line);
                logical_lines.push((start_line, std::mem::take(&mut pending)));
            }
        }
        if !pending.is_empty() {
            return Err(error(
                "SCRIPT_PARSE_ERROR",
                format!("line {start_line}: trailing continuation"),
            ));
        }
        for (line_no, line) in logical_lines {
            let line_no = line_no - 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut tokens = split_line(line)
                .map_err(|e| error("SCRIPT_PARSE_ERROR", format!("line {}: {e}", line_no + 1)))?;
            // Validate command-file arguments with the same Clap tree as shell commands.
            let command = Cli::try_parse_from(
                std::iter::once("aitd").chain(tokens.iter().map(String::as_str)),
            )
            .map_err(|e| error("SCRIPT_PARSE_ERROR", format!("line {}: {e}", line_no + 1)))?;
            if command.instance.is_some()
                || command.project_instance.is_some()
                || command.revision.is_some()
                || command.draft.is_some()
                || command.proposal_id.is_some()
                || command.expect_generation.is_some()
                || command.json
                || command.view != "live"
            {
                return Err(error(
                    "SCRIPT_COMMAND_UNSUPPORTED",
                    format!(
                        "line {}: select the editor and draft on run, not inside the batch",
                        line_no + 1
                    ),
                ));
            }
            match command.command {
                Command::Object {
                    command:
                        ObjectCommand::Create {
                            object_type,
                            name,
                            named,
                        },
                } => {
                    tokens = vec![
                        "object".into(),
                        "create".into(),
                        object_type,
                        name.or(named).unwrap(),
                    ];
                }
                Command::Object {
                    command:
                        ObjectCommand::Field {
                            id,
                            property,
                            assignment,
                        },
                } => {
                    tokens = vec![
                        "object".into(),
                        "field".into(),
                        "set".into(),
                        id,
                        property,
                        assignment,
                    ];
                }
                Command::Object {
                    command: ObjectCommand::Duplicate { id, name },
                } => {
                    tokens = vec!["object".into(), "duplicate".into(), id.to_string()];
                    if let Some(name) = name {
                        tokens.push(name);
                    }
                }
                Command::Object {
                    command: ObjectCommand::Order { ids },
                } => {
                    tokens = vec!["object".into(), "order".into(), "set".into()];
                    tokens.extend(ids);
                }
                Command::Object {
                    command: ObjectCommand::BytesImport { id, property, file },
                } => {
                    tokens = vec![
                        "object".into(),
                        "bytes".into(),
                        "import".into(),
                        id,
                        property,
                        file.to_string_lossy().into(),
                    ];
                }
                Command::Object {
                    command:
                        ObjectCommand::Set {
                            id,
                            mut assignments,
                            value_file: Some(file),
                        },
                } => {
                    if assignments.len() != 1 || assignments[0].contains('=') {
                        return Err(error(
                            "SCRIPT_PARSE_ERROR",
                            format!(
                                "line {}: specify one property with --value-file",
                                line_no + 1
                            ),
                        ));
                    }
                    let text = fs::read_to_string(base.join(file))
                        .map_err(|e| error("INPUT_FAILED", format!("line {}: {e}", line_no + 1)))?;
                    assignments[0] = format!("{}={text}", assignments[0]);
                    tokens = vec!["object".into(), "set".into(), id];
                    tokens.extend(assignments);
                }
                Command::Project {
                    command:
                        ProjectCommand::Set {
                            mask_size,
                            vt_version,
                            soft_key_width,
                            soft_key_height,
                        },
                } => {
                    tokens = vec!["project".into(), "settings".into(), "set".into()];
                    for (key, value) in [
                        ("mask_size", mask_size.map(u64::from)),
                        ("vt_version", vt_version.map(u64::from)),
                        ("soft_key_width", soft_key_width.map(u64::from)),
                        ("soft_key_height", soft_key_height.map(u64::from)),
                    ] {
                        if let Some(value) = value {
                            tokens.push(format!("{key}={value}"));
                        }
                    }
                }
                Command::Project {
                    command:
                        ProjectCommand::IdRangeSet {
                            object_type,
                            start,
                            end,
                        },
                } => {
                    tokens = vec![
                        "project".into(),
                        "id-range".into(),
                        "set".into(),
                        object_type,
                        start.to_string(),
                        end.to_string(),
                    ];
                }
                Command::Project {
                    command: ProjectCommand::IdRangeReset,
                } => tokens = vec!["project".into(), "id-range".into(), "reset".into()],
                _ => {}
            }
            if tokens
                .first()
                .is_some_and(|x| ["draft", "proposal", "export", "run"].contains(&x.as_str()))
            {
                return Err(error(
                    "SCRIPT_COMMAND_UNSUPPORTED",
                    format!(
                        "line {}: lifecycle and output commands cannot run in a batch",
                        line_no + 1
                    ),
                ));
            }
            let parsed = if tokens.first().is_some_and(|x| x == "graphic")
                && tokens.get(1).is_some_and(|x| x == "import")
            {
                let path = base.join(tokens.get(2).ok_or_else(|| {
                    error(
                        "SCRIPT_COMMAND_ERROR",
                        format!("line {}: graphic import requires a file", line_no + 1),
                    )
                })?);
                let bytes = fs::read(&path).map_err(|e| {
                    error("SCRIPT_COMMAND_ERROR", format!("line {}: {e}", line_no + 1))
                })?;
                let name = tokens
                    .windows(2)
                    .find(|pair| pair[0] == "--name")
                    .map(|pair| pair[1].clone());
                vec![
                    json!({"kind":"graphic_import","filename":path.file_name().and_then(|s|s.to_str()).unwrap_or("graphic"),"bytes_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes),"name":name}),
                ]
            } else if tokens.first().is_some_and(|x| x == "object")
                && tokens.get(1).is_some_and(|x| x == "bytes")
                && tokens.get(2).is_some_and(|x| x == "import")
            {
                let object = tokens.get(3).ok_or_else(|| {
                    error(
                        "SCRIPT_COMMAND_ERROR",
                        format!(
                            "line {}: object bytes import requires object ID",
                            line_no + 1
                        ),
                    )
                })?;
                let property = tokens.get(4).ok_or_else(|| {
                    error(
                        "SCRIPT_COMMAND_ERROR",
                        format!(
                            "line {}: object bytes import requires property",
                            line_no + 1
                        ),
                    )
                })?;
                let file = tokens.get(5).ok_or_else(|| {
                    error(
                        "SCRIPT_COMMAND_ERROR",
                        format!("line {}: object bytes import requires file", line_no + 1),
                    )
                })?;
                let bytes = fs::read(base.join(file)).map_err(|e| {
                    error("SCRIPT_COMMAND_ERROR", format!("line {}: {e}", line_no + 1))
                })?;
                vec![
                    json!({"kind":"object_bytes_set","object":parse_target_token(object),"property":property,"bytes_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes)}),
                ]
            } else {
                ag_iso_terminal_designer::ai::commands::parse_edit(&tokens).map_err(|e| {
                    error("SCRIPT_COMMAND_ERROR", format!("line {}: {e}", line_no + 1))
                })?
            };
            actions.extend(parsed.into_iter().map(|mut action| {
                action["_line"] = json!(line_no + 1);
                action
            }));
            if actions.len() > 256 {
                return Err(error(
                    "SCRIPT_LIMIT",
                    "A command file may contain at most 256 edit actions.",
                ));
            }
        }
        edit_actions(instance, project, revision, &draft, actions)
    }

    fn render(value: &Value) -> String {
        if let Some(script) = value["result"]["script"].as_str() {
            return script.to_owned();
        }
        if let Some(e) = value.get("error") {
            format!(
                "{}: {}",
                e["code"].as_str().unwrap_or("ERROR"),
                e["message"].as_str().unwrap_or("request failed")
            )
        } else if let Some(result) = value.get("result") {
            let mut lines = Vec::new();
            if let Some(fields) = result.as_object() {
                for (key, value) in fields {
                    if let Some(rows) = value
                        .as_array()
                        .filter(|rows| !rows.is_empty() && rows[0].is_object())
                    {
                        lines.push(format!("{key}:"));
                        let columns: Vec<_> = rows
                            .iter()
                            .filter_map(Value::as_object)
                            .flat_map(|row| row.keys())
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect();
                        lines.push(format!(
                            "  {}",
                            columns
                                .iter()
                                .map(|c| c.as_str())
                                .collect::<Vec<_>>()
                                .join("  ")
                        ));
                        for row in rows {
                            lines.push(format!(
                                "  {}",
                                columns
                                    .iter()
                                    .map(|c| display_cell(&row[*c]))
                                    .collect::<Vec<_>>()
                                    .join("  ")
                            ));
                        }
                    } else if value.is_object() {
                        lines.push(format!("{key}:"));
                        lines.extend(
                            render(&json!({"result":value}))
                                .lines()
                                .map(|line| format!("  {line}")),
                        );
                    } else {
                        lines.push(format!("{key}: {}", display_cell(value)));
                    }
                }
                lines.join("\n")
            } else {
                display_cell(result)
            }
        } else {
            display_cell(value)
        }
    }
    fn display_cell(value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Null => "-".into(),
            Value::Array(items) => items
                .iter()
                .map(display_cell)
                .collect::<Vec<_>>()
                .join(", "),
            Value::Object(fields) => fields
                .iter()
                .map(|(key, value)| format!("{key}={}", display_cell(value)))
                .collect::<Vec<_>>()
                .join(", "),
            _ => value.to_string(),
        }
    }
    fn bash_completions() -> String {
        fn commands(command: &clap::Command, path: String, items: &mut Vec<(String, String)>) {
            let choices = command
                .get_subcommands()
                .map(|c| c.get_name().to_owned())
                .chain(
                    command
                        .get_arguments()
                        .filter_map(|arg| arg.get_long().map(|long| format!("--{long}"))),
                )
                .collect::<Vec<_>>()
                .join(" ");
            items.push((path.clone(), choices));
            for child in command.get_subcommands() {
                commands(child, format!("{path} {}", child.get_name()), items);
            }
        }
        let mut items = Vec::new();
        commands(&Cli::command(), "aitd".into(), &mut items);
        let paths = items
            .iter()
            .map(|(path, _)| format!("'{path}'"))
            .collect::<Vec<_>>()
            .join("|");
        let mut script=format!("# Generated by aitd completions bash\n_aitd_complete() {{\n  local path=aitd word words cur=${{COMP_WORDS[COMP_CWORD]}}\n  for word in \"${{COMP_WORDS[@]:1:COMP_CWORD-1}}\"; do\n    case \"$path $word\" in {paths}) path=\"$path $word\" ;; esac\n  done\n  case \"$path\" in\n");
        for (path, choices) in items {
            script.push_str(&format!("    '{path}') words='{choices}' ;;\n"));
        }
        script.push_str("  esac\n  COMPREPLY=( $(compgen -W \"$words\" -- \"$cur\") )\n}\ncomplete -F _aitd_complete aitd\n");
        script
    }
    pub fn run() -> ExitCode {
        let args: Vec<_> = std::env::args_os().collect();
        let machine = args.iter().any(|arg| arg == "--json");
        let cli = match Cli::try_parse_from(&args) {
            Ok(cli) => cli,
            Err(err)
                if matches!(
                    err.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) =>
            {
                let _ = err.print();
                return ExitCode::SUCCESS;
            }
            Err(err) => {
                if machine {
                    eprintln!("{}", error("INVALID_USAGE", err.to_string()));
                } else {
                    let _ = err.print();
                }
                return ExitCode::from(2);
            }
        };
        let machine = cli.json;
        match execute(cli) {
            Ok(value) => {
                if machine {
                    println!("{}", value)
                } else {
                    println!("{}", render(&value))
                }
                ExitCode::SUCCESS
            }
            Err(value) => {
                if machine {
                    eprintln!("{}", value)
                } else {
                    eprintln!("{}", render(&value))
                }
                let code = value["error"]["code"].as_str().unwrap_or("");
                match code {
                    "INVALID_USAGE" => ExitCode::from(2),
                    "PROJECT_NOT_OPEN" | "EDITOR_NOT_FOUND" | "EDITOR_CONNECTION_FAILED" => {
                        ExitCode::from(3)
                    }
                    "STALE_PROJECT"
                    | "DRAFT_CONFLICT"
                    | "PROPOSAL_STALE"
                    | "PROJECT_INSTANCE_MISMATCH"
                    | "PROJECT_REVISION_MISMATCH"
                    | "EXPORT_REVISION_CONFLICT" => ExitCode::from(4),
                    "VALIDATION_FAILED" => ExitCode::from(5),
                    "REJECTED" => ExitCode::from(6),
                    "INVALIDATED" => ExitCode::from(7),
                    "SUPERSEDED" => ExitCode::from(8),
                    "APPLY_FAILED" => ExitCode::from(9),
                    "WAIT_TIMEOUT" => ExitCode::from(10),
                    _ => ExitCode::FAILURE,
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    native::run()
}
#[cfg(target_arch = "wasm32")]
fn main() {}
