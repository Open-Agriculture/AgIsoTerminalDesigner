#[cfg(not(target_arch = "wasm32"))]
mod native {
    use ag_iso_terminal_designer::cli::{
        commands::*,
        editing::{EditAction, MAX_EDIT_ACTIONS},
        protocol::AutomationAction,
        session::Session,
    };
    use clap::{CommandFactory, Parser};
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::{
        fs,
        io::{self, Read},
        path::{Path, PathBuf},
        process::ExitCode,
    };

    fn error(code: &str, message: impl Into<String>) -> Value {
        json!({"error":{"code":code,"message":message.into()}})
    }

    /// Holds the session lock for the entire invocation, including outputs.
    struct Client {
        session: RefCell<Session>,
        check: bool,
        machine: bool,
    }
    impl Client {
        fn request(&self, action: AutomationAction, mut args: Value) -> Result<Value, Value> {
            let session = self.session.borrow();
            let project = session
                .project()
                .map_err(|e| error("PROJECT_NOT_OPEN", e))?;
            use ag_iso_terminal_designer::cli::names;
            use AutomationAction as A;
            let object_field = match action {
                A::GetDesignContext | A::InspectDesignRegion => Some("root_id"),
                A::PreviewDesign => Some("mask_id"),
                A::ListObjects => Some("parent_id"),
                _ => None,
            };
            if let Some(field) = object_field {
                if let Some(name) = args[field].as_str() {
                    args[field] = json!(names::object_id(
                        project,
                        name.strip_prefix("name:").unwrap_or(name)
                    )
                    .map_err(|e| error("OBJECT_NOT_FOUND", e))?);
                }
            }
            match action {
                A::GetComponent => {
                    if let Some(name) = args["component_id"].as_str() {
                        args["component_id"] = json!(names::component_id(project, name)
                            .map_err(|e| error("COMPONENT_NOT_FOUND", e))?);
                    }
                }
                A::GetComponentInstances => {
                    if let Some(name) = args["instance_id"].as_str() {
                        args["instance_id"] = json!(names::instance_id(project, name)
                            .map_err(|e| error("INSTANCE_NOT_FOUND", e))?);
                    }
                }
                A::GetGraphic => {
                    if let Some(name) = args["id"].as_str() {
                        let asset = names::graphic_asset_id(project, name);
                        let picture = names::picture_graphic_id(project, name);
                        args["id"] = match (asset, picture) {
                            (Ok(_), Ok(_)) => {
                                return Err(error(
                                    "AMBIGUOUS_GRAPHIC_NAME",
                                    format!(
                                    "Both a graphic asset and PictureGraphic are named '{name}'"
                                ),
                                ))
                            }
                            (Ok(id), Err(e)) if !e.starts_with("Ambiguous") => {
                                json!(id.to_string())
                            }
                            (Err(e), Ok(id)) if !e.starts_with("Ambiguous") => {
                                json!(id.to_string())
                            }
                            (Err(e), _) if e.starts_with("Ambiguous") => {
                                return Err(error("AMBIGUOUS_GRAPHIC_NAME", e))
                            }
                            (_, Err(e)) if e.starts_with("Ambiguous") => {
                                return Err(error("AMBIGUOUS_GRAPHIC_NAME", e))
                            }
                            _ => {
                                return Err(error(
                                    "GRAPHIC_NOT_FOUND",
                                    format!("No graphic named '{name}'"),
                                ))
                            }
                        };
                    }
                }
                _ => (),
            }
            let result =
                ag_iso_terminal_designer::cli::automation_service::execute(project, action, args)
                    .map_err(|e| error(e.code, e.message))?;
            let result = if self.machine {
                result
            } else {
                readable_result(project, result, Default::default())
            };
            Ok(json!({"success":true,"result":result}))
        }
        fn edit(&self, actions: Vec<EditAction>) -> Result<Value, Value> {
            let mut session = self.session.borrow_mut();
            let previous_names = if self.machine {
                Default::default()
            } else {
                let project = session
                    .project()
                    .map_err(|e| error("PROJECT_NOT_OPEN", e))?;
                project
                    .get_pool()
                    .objects()
                    .iter()
                    .map(|object| {
                        (
                            object.id().value().to_string(),
                            project.get_object_info(object).get_name(object),
                        )
                    })
                    .collect()
            };
            let result = ag_iso_terminal_designer::cli::editing::execute_batch(
                session
                    .project_mut()
                    .map_err(|e| error("PROJECT_NOT_OPEN", e))?,
                json!({"actions":actions,"check":self.check}),
            )
            .map_err(|e| error(e.code, e.message))?;
            if !self.check {
                session.persist().map_err(|e| error("SESSION_FAILED", e))?;
            }
            let result = if self.machine {
                result
            } else {
                readable_result(
                    session
                        .project()
                        .map_err(|e| error("PROJECT_NOT_OPEN", e))?,
                    result,
                    previous_names,
                )
            };
            Ok(json!({"success":true,"result":result}))
        }
        fn output(
            &self,
            action: AutomationAction,
            args: Value,
            field: &str,
            output: &Path,
            force: bool,
        ) -> Result<Value, Value> {
            let mut response = self.request(action, args)?;
            let bytes = decode_output(&response["result"], field)?;
            write_output(output, &bytes, force)?;
            response["result"] = json!({"output":output,"bytes":bytes.len()});
            Ok(response)
        }
    }

    /// Present authoring identities as names while retaining numeric protocol
    /// details in JSON output and explicit ID configuration commands.
    fn readable_result(
        project: &ag_iso_terminal_designer::EditorProject,
        mut value: Value,
        mut identities: std::collections::HashMap<String, String>,
    ) -> Value {
        for object in project.get_pool().objects() {
            identities.insert(
                object.id().value().to_string(),
                project.get_object_info(object).get_name(object),
            );
        }
        for definition in project.component_source().definitions.values() {
            identities.insert(definition.id.to_string(), definition.name.clone());
            for node in &definition.nodes {
                identities.insert(node.id.to_string(), node.name.clone());
            }
            for parameter in &definition.parameters {
                identities.insert(parameter.id.to_string(), parameter.name.clone());
            }
        }
        for instance in &project.component_source().instances {
            identities.insert(instance.id.to_string(), instance.name.clone());
        }
        for asset in project.graphic_library().assets.values() {
            identities.insert(asset.id.to_string(), asset.name.clone());
        }
        fn identity(value: &mut Value, names: &std::collections::HashMap<String, String>) {
            let key = match value {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                _ => return,
            };
            if let Some(name) = names.get(&key) {
                *value = json!(name);
            }
        }
        fn visit(value: &mut Value, names: &std::collections::HashMap<String, String>) {
            match value {
                Value::Array(items) => items.iter_mut().for_each(|item| visit(item, names)),
                Value::Object(fields) => {
                    fields.remove("transaction_id");
                    fields.remove("compiled_ids");
                    if fields.get("kind").and_then(Value::as_str) == Some("Graphic") {
                        if let Some(default) = fields.get_mut("default_value") {
                            identity(default, names);
                        }
                    }
                    if let Some(Value::Object(overrides)) = fields.get_mut("overrides") {
                        let old = std::mem::take(overrides);
                        for (key, value) in old {
                            overrides.insert(names.get(&key).cloned().unwrap_or(key), value);
                        }
                    }
                    if fields.get("name").is_some_and(|name| !name.is_null()) {
                        fields.remove("id");
                    }
                    if fields.get("macro_name").is_some_and(|name| !name.is_null()) {
                        fields.remove("macro_id");
                    }
                    for (key, item) in fields.iter_mut() {
                        // Property values and schema defaults are literal data.
                        if matches!(key.as_str(), "properties" | "schema" | "object_id_ranges") {
                            continue;
                        }
                        if matches!(
                            key.as_str(),
                            "id" | "object_id"
                                | "component_id"
                                | "instance_id"
                                | "node_id"
                                | "root_node"
                                | "target_node"
                                | "parameter_id"
                                | "asset_id"
                                | "macro_id"
                                | "root_id"
                                | "materialized_object_id"
                                | "Primitive"
                                | "Internal"
                                | "Parameter"
                                | "Id"
                        ) {
                            identity(item, names);
                        } else if matches!(
                            key.as_str(),
                            "changed_ids" | "allocated_ids" | "affected_objects" | "path"
                        ) {
                            if let Some(items) = item.as_array_mut() {
                                items.iter_mut().for_each(|item| identity(item, names));
                            }
                        } else {
                            visit(item, names);
                        }
                    }
                    if let Some(allocated) = fields.remove("allocated_ids") {
                        fields.insert("allocated_objects".into(), allocated);
                    }
                    if let Some(changed) = fields.remove("changed_ids") {
                        fields.insert("changed_objects".into(), changed);
                    }
                }
                _ => (),
            }
        }
        visit(&mut value, &identities);
        value
    }

    fn execute(cli: Cli) -> Result<Value, Value> {
        use AutomationAction as A;
        if cli.check
            && matches!(
                cli.command,
                Command::Capabilities | Command::Completions { .. } | Command::Backups { .. }
            )
        {
            return Err(error(
                "INVALID_USAGE",
                "--check is only supported by edit commands and run",
            ));
        }
        match &cli.command {
            Command::Capabilities => {
                return Ok(json!({"success":true,"result":{
                    "cli_api_version":5,"json_input":false,"atomic_command_files":true,"max_edit_actions":MAX_EDIT_ACTIONS,
                    "commands":Cli::command().get_subcommands().map(|c|json!({"name":c.get_name(),"description":c.get_about().map(ToString::to_string),"subcommands":c.get_subcommands().map(|s|s.get_name()).collect::<Vec<_>>()})).collect::<Vec<_>>()
                }}))
            }
            Command::Completions { shell } => {
                return Ok(
                    json!({"success":true,"result":{"shell":shell,"script":bash_completions()}}),
                )
            }
            Command::Backups {
                command: BackupCommand::List { project },
            } => {
                return Session::backups_list(project.as_deref())
                    .map(|backups| json!({"success":true,"result":{"backups":backups}}))
                    .map_err(|e| error("BACKUP_FAILED", e));
            }
            _ => {}
        }
        let mut session = Session::open().map_err(|e| error("SESSION_FAILED", e))?;
        if let Command::Project { command } = &cli.command {
            let lifecycle = matches!(
                command,
                ProjectCommand::Load { .. }
                    | ProjectCommand::New { .. }
                    | ProjectCommand::Close { .. }
                    | ProjectCommand::Save { .. }
            );
            if lifecycle {
                if cli.check {
                    return Err(error(
                        "INVALID_USAGE",
                        "--check is only supported by edit commands and run",
                    ));
                }
                let result = match command {
                    ProjectCommand::Load { path, discard } => session
                        .load(path, *discard)
                        .map(|_| json!(session.status()))
                        .map_err(|e| error("PROJECT_FAILED", e)),
                    ProjectCommand::New { discard } => session
                        .new_project(*discard)
                        .map(|_| json!(session.status()))
                        .map_err(|e| error("PROJECT_FAILED", e)),
                    ProjectCommand::Close { discard } => session
                        .close(*discard)
                        .map(|_| json!(session.status()))
                        .map_err(|e| error("PROJECT_FAILED", e)),
                    ProjectCommand::Save {
                        path,
                        force,
                        skip_validation,
                    } => session
                        .save(path, *force, *skip_validation, |project| {
                            let validation =
                                ag_iso_terminal_designer::cli::automation_service::execute(
                                    project,
                                    A::ValidateDesign,
                                    json!({}),
                                )
                                .map_err(|e| e.message)?;
                            if validation["valid"] == true {
                                Ok(())
                            } else {
                                Err(format!(
                                    "Project validation failed: {}",
                                    validation["diagnostics"]
                                ))
                            }
                        })
                        .map(|saved| json!(saved))
                        .map_err(|e| error(e.code, e.message)),
                    _ => unreachable!(),
                }?;
                return Ok(json!({"success":true,"result":result}));
            }
        }
        let status = json!({"success":true,"result":session.status()});
        let client = Client {
            session: RefCell::new(session),
            check: cli.check,
            machine: cli.json,
        };
        let base = std::env::current_dir().map_err(|e| error("INPUT_FAILED", e.to_string()))?;
        if let Some(actions) = command_edit_actions(&cli.command, &base, &|path| {
            fs::read(path).map_err(|e| e.to_string())
        })? {
            return client.edit(actions);
        }
        if cli.check && !matches!(cli.command, Command::Run { .. }) {
            return Err(error(
                "INVALID_USAGE",
                "--check is only supported by edit commands and run",
            ));
        }
        match cli.command {
            Command::Status | Command::Project {command:ProjectCommand::Current} => Ok(status),
            Command::Tree {root,depth} => client.request(A::GetDesignContext,json!({"root_id":root,"depth":depth,"max_objects":200})),
            Command::Context {scope,root,depth,limit,offset,layout_offset,include_properties,include_references,include_selection} => client.request(A::GetDesignContext,json!({"scope":scope,"root_id":root,"depth":depth,"max_objects":limit,"offset":offset,"layout_offset":layout_offset,"include_properties":include_properties,"include_references":include_references,"include_selection":include_selection})),
            Command::Schema {object_types,properties} => {
                if object_types.is_empty() || object_types == ["types"] {client.request(A::GetObjectTypes,json!({}))}
                else {client.request(A::GetObjectSchema,json!({"object_types":object_types,"properties":properties}))}
            }
            Command::Object {command} => match command {
                ObjectCommand::List {query,object_type,parent,offset,limit,all} => {
                    let mut args=json!({"query":query,"type":object_type,"parent_id":parent,"offset":offset,"limit":limit});
                    let mut response=client.request(A::ListObjects,args.clone())?;
                    let mut objects=response["result"]["objects"].as_array().ok_or_else(||error("AUTOMATION_FAILED","Object list missing"))?.clone();
                    while all && response["result"]["has_more"].as_bool()==Some(true) {
                        args["offset"]=json!(offset+objects.len());
                        response=client.request(A::ListObjects,args.clone())?;
                        let page=response["result"]["objects"].as_array().ok_or_else(||error("AUTOMATION_FAILED","Object list missing"))?;
                        if page.is_empty() { return Err(error("AUTOMATION_FAILED","Object pagination did not advance")); }
                        objects.extend(page.iter().cloned());
                    }
                    response["result"]["objects"]=json!(objects); response["result"]["offset"]=json!(offset);
                    Ok(response)
                }
                ObjectCommand::Get {id,parents,children,macros,style,offset,parent_offset,property_offset} => client.request(A::GetObjects,json!({"targets":[typed_target(&id)],"include_parents":parents,"include_children":children,"include_macros":macros,"include_nearby_style":style,"detail_offset":offset,"parent_offset":parent_offset,"property_offset":property_offset})),
                ObjectCommand::BytesExport {id,property,output,force} => client.output(A::ExportObjectBytes,json!({"object":typed_target(&id),"property":property}),"bytes_base64",&output,force),
                _=>unreachable!("edit command converted above"),
            },
            Command::Component {command} => match command {
                ComponentCommand::List {query,category,limit}=>client.request(A::ListComponents,json!({"query":query,"category":category,"limit":limit})),
                ComponentCommand::Get {id,nodes}=>client.request(A::GetComponent,json!({"component_id":id,"include_nodes":nodes})),
                _=>unreachable!("edit command converted above"),
            },
            Command::Graphic {command}=>match command {
                GraphicsCommand::Get {id}=>client.request(A::GetGraphic,json!({"id":id})),
                GraphicsCommand::List {query,status,limit}=>client.request(A::ListGraphics,json!({"query":query,"status":status,"limit":limit})),
                GraphicsCommand::Export {id,output,force}=>client.output(A::GetGraphic,json!({"id":id,"export":true}),"/graphic/png_base64",&output,force),
                _=>unreachable!("edit command converted above"),
            },
            Command::Validate=> {
                let mut response = client.request(A::ValidateDesign,json!({}))?;
                if response["result"]["valid"] == false {
                    response["success"] = json!(false);
                    response["error"] = json!({"code":"VALIDATION_FAILED","message":"Project validation failed"});
                    Err(response)
                } else { Ok(response) }
            },
            Command::Preview {mask,output,force}=>client.output(A::PreviewDesign,json!({"mask_id":mask}),"png_base64",&output,force),
            Command::Export {command}=>export(&client,command),
            Command::Run {file}=>run_file(&client,&file),
            Command::Region {root,region,types,limit}=>client.request(A::InspectDesignRegion,json!({"root_id":root,"region":region,"types":types,"max_results":limit})),
            Command::Colours {colours}=>client.request(A::ResolveColours,json!({"colours":colours})),
            Command::Project {command:ProjectCommand::Settings {command:None|Some(ProjectSettingsCommand::Show)}}=>client.request(A::ProjectSettings,json!({})),
            Command::Instance {command}=>client.request(A::GetComponentInstances,match command {InstanceCommand::List=>json!({}),InstanceCommand::Get {id}=>json!({"instance_id":id})}),
            _=>unreachable!("local and edit commands handled above"),
        }
    }

    fn decode_output(result: &Value, field: &str) -> Result<Vec<u8>, Value> {
        let encoded = (if field.starts_with('/') {
            result.pointer(field)
        } else {
            result.get(field)
        })
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                "EXPORT_RESPONSE_INVALID",
                format!("Output response omitted {field}"),
            )
        })?;
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .map_err(|e| error("EXPORT_RESPONSE_INVALID", e.to_string()))
    }
    fn write_output(path: &Path, bytes: &[u8], force: bool) -> Result<(), Value> {
        if fs::symlink_metadata(path).is_ok() && !force {
            return Err(error(
                "OUTPUT_EXISTS",
                format!("{} exists; pass --force to replace it.", path.display()),
            ));
        }
        let staged = stage_output(path, bytes)?;
        publish_outputs(std::slice::from_ref(&staged), force)
    }
    struct StagedOutput {
        temporary: PathBuf,
        path: PathBuf,
        bytes: usize,
    }
    impl Drop for StagedOutput {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.temporary);
        }
    }
    fn stage_output(path: &Path, bytes: &[u8]) -> Result<StagedOutput, Value> {
        if path.is_dir() {
            return Err(error(
                "OUTPUT_FAILED",
                format!("{} is a directory.", path.display()),
            ));
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        }
        let tmp = temporary_output_path(path);
        let staged = StagedOutput {
            temporary: tmp,
            path: path.to_owned(),
            bytes: bytes.len(),
        };
        fs::write(&staged.temporary, bytes).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        Ok(staged)
    }
    fn publish_outputs(staged: &[StagedOutput], force: bool) -> Result<(), Value> {
        let mut destinations = std::collections::HashSet::new();
        for output in staged {
            let parent = output
                .path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let destination = fs::canonicalize(parent)
                .map_err(|e| error("OUTPUT_FAILED", e.to_string()))?
                .join(
                    output
                        .path
                        .file_name()
                        .ok_or_else(|| error("OUTPUT_FAILED", "Output must name a file"))?,
                );
            if !destinations.insert(destination) {
                return Err(error("INVALID_USAGE", "Outputs must use different paths"));
            }
        }
        for output in staged {
            let result = if force {
                fs::rename(&output.temporary, &output.path)
            } else {
                fs::hard_link(&output.temporary, &output.path)
            };
            result.map_err(|e| {
                let code = if e.kind() == io::ErrorKind::AlreadyExists {
                    "OUTPUT_EXISTS"
                } else {
                    "OUTPUT_FAILED"
                };
                error(code, format!("{}: {e}", output.path.display()))
            })?;
        }
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
    fn export(client: &Client, command: ExportCommand) -> Result<Value, Value> {
        let (format, outputs, force) = match command {
            ExportCommand::Project { output_dir } => return export_project(client, &output_dir),
            ExportCommand::Iop { output, force } => ("iop", vec![("iop", output)], force),
            ExportCommand::Header { output, force } => ("header", vec![("header", output)], force),
            ExportCommand::Bundle { iop, header, force } => {
                ("bundle", vec![("iop", iop), ("header", header)], force)
            }
        };
        if outputs.len() == 2 && outputs[0].1 == outputs[1].1 {
            return Err(error(
                "INVALID_USAGE",
                "IOP and header outputs must use different paths",
            ));
        }
        for (_, path) in &outputs {
            if fs::symlink_metadata(path).is_ok() && !force {
                return Err(error(
                    "OUTPUT_EXISTS",
                    format!("{} exists; pass --force to replace it", path.display()),
                ));
            }
        }
        let session = client.session.borrow();
        let project = session
            .project()
            .map_err(|e| error("PROJECT_NOT_OPEN", e))?;
        let artifacts = ag_iso_terminal_designer::cli::export::artifacts(project, format)
            .map_err(|e| error("VALIDATION_FAILED", e.to_string()))?;
        let staged = outputs
            .into_iter()
            .map(|(key, path)| {
                let bytes = match key {
                    "iop" => artifacts.iop.as_deref(),
                    "header" => artifacts.header.as_deref(),
                    _ => unreachable!(),
                }
                .ok_or_else(|| error("EXPORT_FAILED", "Requested artifact is missing"))?;
                stage_output(&path, bytes)
            })
            .collect::<Result<Vec<_>, Value>>()?;
        publish_outputs(&staged, force)?;
        Ok(json!({"success":true,"result":{
            "outputs":staged.iter().map(|o|json!({"path":o.path,"bytes":o.bytes})).collect::<Vec<_>>(),
            "vt_version":artifacts.vt_version
        }}))
    }

    fn export_project(client: &Client, output: &Path) -> Result<Value, Value> {
        if fs::symlink_metadata(output).is_ok() {
            return Err(error(
                "OUTPUT_EXISTS",
                format!("{} already exists", output.display()),
            ));
        }
        let validation = client.request(AutomationAction::ValidateDesign, json!({}))?;
        if validation["result"]["valid"] != true {
            return Err(error(
                "VALIDATION_FAILED",
                format!(
                    "Project validation failed: {}",
                    validation["result"]["diagnostics"]
                ),
            ));
        }
        let session = client.session.borrow();
        let project = session
            .project()
            .map_err(|e| error("PROJECT_NOT_OPEN", e))?;
        let project_bytes = project
            .save_project()
            .map_err(|e| error("EXPORT_FAILED", e.to_string()))?;
        let iop = project
            .export_iop()
            .map_err(|e| error("EXPORT_FAILED", e.to_string()))?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let staged = parent.join(format!(".aitd-export-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staged).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(staged.clone());
        fs::write(staged.join("project.aitp"), &project_bytes)
            .and_then(|_| fs::write(staged.join("project.iop"), &iop))
            .map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        publish_directory(&staged, output).map_err(|e| {
            error(
                if e.kind() == io::ErrorKind::AlreadyExists {
                    "OUTPUT_EXISTS"
                } else {
                    "OUTPUT_FAILED"
                },
                e.to_string(),
            )
        })?;
        Ok(
            json!({"success":true,"result":{"output_dir":output,"project_bytes":project_bytes.len(),"iop_bytes":iop.len()}}),
        )
    }

    #[cfg(target_os = "linux")]
    fn publish_directory(staged: &Path, output: &Path) -> io::Result<()> {
        use std::os::unix::ffi::OsStrExt;
        let staged = std::ffi::CString::new(staged.as_os_str().as_bytes())?;
        let output = std::ffi::CString::new(output.as_os_str().as_bytes())?;
        // RENAME_NOREPLACE prevents replacing even an empty directory created
        // by another process between the initial check and publication.
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                staged.as_ptr(),
                libc::AT_FDCWD,
                output.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(target_os = "macos")]
    fn publish_directory(staged: &Path, output: &Path) -> io::Result<()> {
        use std::os::unix::ffi::OsStrExt;
        let staged = std::ffi::CString::new(staged.as_os_str().as_bytes())?;
        let output = std::ffi::CString::new(output.as_os_str().as_bytes())?;
        let result =
            unsafe { libc::renamex_np(staged.as_ptr(), output.as_ptr(), libc::RENAME_EXCL) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(windows)]
    fn publish_directory(staged: &Path, output: &Path) -> io::Result<()> {
        // Windows refuses to replace an existing destination directory.
        fs::rename(staged, output)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    fn publish_directory(_staged: &Path, _output: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Atomic directory export without replacement is unavailable on this platform",
        ))
    }

    fn run_file(client: &Client, file: &str) -> Result<Value, Value> {
        let (contents, base) = if file == "-" {
            let mut contents = String::new();
            io::stdin()
                .read_to_string(&mut contents)
                .map_err(|e| error("INPUT_FAILED", e.to_string()))?;
            (contents, std::env::current_dir().unwrap_or_default())
        } else {
            let path = PathBuf::from(file);
            (
                fs::read_to_string(&path).map_err(|e| error("INPUT_FAILED", e.to_string()))?,
                path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            )
        };
        let actions = parse_commands(&contents, &base, &|path| {
            fs::read(path).map_err(|e| e.to_string())
        })?;
        client.edit(actions)
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
        let mut script = format!(
            "# Generated by aitd completions bash\n_aitd_complete() {{\n  local path=aitd word words cur=${{COMP_WORDS[COMP_CWORD]}}\n  for word in \"${{COMP_WORDS[@]:1:COMP_CWORD-1}}\"; do\n    case \"$path $word\" in {paths}) path=\"$path $word\" ;; esac\n  done\n  case \"$path\" in\n"
        );
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
                    "INVALID_USAGE" | "INVALID_ARGUMENTS" => ExitCode::from(2),
                    "PROJECT_NOT_OPEN" => ExitCode::from(3),
                    "SOURCE_CHANGED" => ExitCode::from(4),
                    "VALIDATION_FAILED" => ExitCode::from(5),
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
