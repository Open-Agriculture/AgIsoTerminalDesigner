//! Readable, preflighted CLI project archives. No project snapshot is embedded.
use crate::{
    object_properties::{
        get_object_property, get_property_descriptors, PropertyKey, PropertySemantic,
    },
    operations::{macro_refs, object_list, object_refs, ObjectReferenceList},
    EditorProject,
};
use ag_iso_stack::object_pool::{object::Object, ObjectType};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

mod comparison;
mod job;
use comparison::Comparison;
pub use job::{ExportProgress, ZipExportJob};

type Tokens = Vec<String>;

/// A deterministic archive's uncompressed entries, also useful for inspecting
/// or testing the exact workflow before ZIP serialization.
pub struct CliProjectArchive {
    /// Relative, slash-separated archive paths and their uncompressed contents.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl CliProjectArchive {
    /// Serialize entries with fixed timestamps, stable order and no compression.
    pub fn to_zip(&self) -> Result<Vec<u8>, String> {
        let mut writer = job::ArchiveWriter::new(self);
        loop {
            if let Some(bytes) = writer.advance(self)? {
                return Ok(bytes);
            }
        }
    }
}

#[derive(Default)]
struct Workflow {
    files: BTreeMap<String, Vec<u8>>,
    phases: BTreeMap<(u8, usize, String), Vec<Tokens>>,
    asset_names: Names,
    command_targets: BTreeMap<Tokens, BTreeSet<u16>>,
    named_targets: BTreeMap<String, u16>,
    fixed_ids: BTreeSet<u16>,
    current_object: Option<u16>,
    objects: BTreeMap<u16, ObjectCommands>,
    object_batches: Vec<(u16, String, String, Vec<Tokens>)>,
}

struct ObjectCommands {
    directory: String,
    stem: String,
    dependencies: BTreeSet<u16>,
    commands: Vec<(u8, Tokens)>,
}

impl Workflow {
    fn command(
        &mut self,
        phase: u8,
        directory: &str,
        tokens: impl IntoIterator<Item = impl ToString>,
    ) {
        let tokens = tokens.into_iter().map(|s| s.to_string()).collect();
        if let Some(id) = self.current_object {
            self.objects
                .get_mut(&id)
                .expect("registered object")
                .commands
                .push((phase, tokens));
        } else {
            self.phases
                .entry((phase, 0, directory.to_owned()))
                .or_default()
                .push(tokens);
        }
    }

    fn finish(self, source: &EditorProject) -> Result<CliProjectArchive, String> {
        let mut replay = ReplayState::new(self);
        while !replay.step()? {}
        let mut comparison = Comparison::default();
        while !comparison.advance(source, &replay.candidate)? {}
        Ok(replay.finish_archive())
    }
}

/// A portable, deterministic name within an archive directory. Compare without
/// case so extraction also works on case-insensitive filesystems.
#[derive(Default)]
struct Names(BTreeSet<String>);

impl Names {
    fn allocate(&mut self, directory: &str, name: Option<&str>, id: &str) -> String {
        let slug: String = name
            .unwrap_or_default()
            .trim()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(64)
            .collect();
        let slug = slug.trim_matches('_');
        let mut stem = if slug.is_empty() {
            id.to_owned()
        } else {
            slug.to_owned()
        };
        // Prefixing avoids Windows device names without altering authoring names.
        if matches!(
            stem.to_ascii_uppercase().as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) {
            stem.insert(0, '_');
        }
        self.reserve(directory, stem, id)
    }

    fn reserve(&mut self, directory: &str, mut stem: String, id: &str) -> String {
        if !self
            .0
            .insert(format!("{directory}/{stem}").to_ascii_lowercase())
        {
            stem = format!("{stem}_{id}");
            while !self
                .0
                .insert(format!("{directory}/{stem}").to_ascii_lowercase())
            {
                stem.push('_');
            }
        }
        stem
    }
}

fn append_batches(
    batches: &mut Vec<(String, String, Vec<Tokens>)>,
    directory: &str,
    stem: &str,
    commands: Vec<Tokens>,
) {
    let count = commands
        .len()
        .div_ceil(crate::cli::editing::MAX_EDIT_ACTIONS);
    for (part, chunk) in commands
        .chunks(crate::cli::editing::MAX_EDIT_ACTIONS)
        .enumerate()
    {
        let suffix = if count > 1 {
            format!("_part_{}", part + 1)
        } else {
            String::new()
        };
        batches.push((
            format!("{directory}/{stem}{suffix}.aitd"),
            directory.to_owned(),
            chunk.to_vec(),
        ));
    }
}

/// Targets needed by a generated object command. Compound lists are kept
/// atomic; macro instructions retain order while waiting for their own targets.
fn command_dependencies(
    phase: u8,
    tokens: &[String],
    object: &ObjectCommands,
    named_targets: &BTreeMap<String, u16>,
) -> BTreeSet<u16> {
    let mut ids = BTreeSet::new();
    let mut add = |value: &str| {
        let value = value.strip_prefix("name:").unwrap_or(value);
        if let Some(id) = named_targets.get(&crate::smart_naming::normalized_name(value)) {
            ids.insert(*id);
        }
    };
    match tokens.first().map(String::as_str) {
        Some("reference") => {
            if let Some(value) = tokens.get(4) {
                add(value);
            }
        }
        Some("placement") => {
            if let Some(value) = tokens.get(3) {
                add(value);
            }
        }
        Some("list" | "pointer") => {
            for value in &tokens[3..] {
                add(value);
            }
        }
        Some("event" | "label") => {
            for value in &tokens[3..] {
                if let Some((key, target)) = value.split_once('=') {
                    if tokens[0] != "label" || matches!(key, "id" | "string" | "graphic") {
                        add(target);
                    }
                }
            }
        }
        Some("graphic") if phase == 50 => {
            ids.extend(object.dependencies.iter().copied());
        }
        Some("macro") if phase == 50 => {
            ids.extend(object.dependencies.iter().copied());
        }
        _ => (),
    }
    ids
}

// Independent fields may resolve independently. Ordered append/placement
// commands retain their order within a source object's command stream.
fn command_stream(tokens: &[String]) -> String {
    let mut key = tokens[..3.min(tokens.len())].join(" ");
    if tokens
        .first()
        .is_some_and(|token| token == "reference" || token == "object")
    {
        if let Some(property) = tokens.get(3) {
            key.push_str(property.split('.').next().unwrap_or(property));
        }
    }
    key
}

impl Workflow {
    fn name_for_id(&self, id: u16) -> String {
        self.named_targets
            .iter()
            .find_map(|(name, candidate)| (*candidate == id).then_some(name.clone()))
            .unwrap_or_else(|| id.to_string())
    }

    fn order_objects(&mut self) -> Result<(), String> {
        let directories: BTreeMap<_, _> = self
            .objects
            .iter()
            .map(|(id, object)| (*id, object.directory.clone()))
            .collect();
        let mut remaining = std::mem::take(&mut self.objects);
        let mut created = BTreeSet::new();
        // Source, command, prerequisites. Insertion order preserves commands
        // such as macro appends that must not overtake earlier instructions.
        let mut pending: Vec<(u16, Tokens, BTreeSet<u16>)> = Vec::new();
        while !remaining.is_empty() {
            let id = *remaining
                .iter()
                .min_by_key(|(id, object)| {
                    (
                        !self.fixed_ids.contains(id),
                        object.dependencies.difference(&created).count(),
                        **id,
                    )
                })
                .unwrap()
                .0;
            let mut object = remaining.remove(&id).unwrap();
            object.commands.sort_by_key(|(phase, _)| *phase);
            created.insert(id);
            let mut commands = Vec::new();
            for (phase, tokens) in std::mem::take(&mut object.commands) {
                let dependencies =
                    self.command_targets
                        .get(&tokens)
                        .cloned()
                        .unwrap_or_else(|| {
                            command_dependencies(phase, &tokens, &object, &self.named_targets)
                        });
                if dependencies.is_subset(&created)
                    && !pending.iter().any(|(source, earlier, _)| {
                        *source == id && command_stream(earlier) == command_stream(&tokens)
                    })
                {
                    commands.push(tokens);
                } else {
                    pending.push((id, tokens, dependencies));
                }
            }
            let mut blocked = BTreeSet::new();
            let mut unresolved = Vec::new();
            for (source, tokens, dependencies) in pending.drain(..) {
                let stream = (source, command_stream(&tokens));
                if !blocked.contains(&stream) && dependencies.is_subset(&created) {
                    commands.push(vec![
                        "#".into(),
                        format!(
                            "Resolve pending command from object {}",
                            self.name_for_id(source)
                        ),
                    ]);
                    let mut tokens = tokens;
                    if tokens.first().is_some_and(|token| token == "graphic") {
                        // File inputs were authored relative to their source's
                        // directory, which can differ from this target's folder.
                        let source_dir = &directories[&source];
                        let asset = normalize(&Path::new(source_dir).join(&tokens[3]))?;
                        tokens[3] = format!(
                            "{}{}",
                            "../".repeat(object.directory.split('/').count()),
                            asset
                        );
                    }
                    commands.push(tokens);
                } else {
                    blocked.insert(stream);
                    unresolved.push((source, tokens, dependencies));
                }
            }
            pending = unresolved;
            let mut batches = Vec::new();
            append_batches(&mut batches, &object.directory, &object.stem, commands);
            self.object_batches.extend(
                batches
                    .into_iter()
                    .map(|(path, directory, commands)| (id, path, directory, commands)),
            );
        }
        if let Some((source, _, dependencies)) = pending.first() {
            return Err(format!(
                "Object {source}: unresolved reference targets {:?}",
                dependencies.difference(&created).collect::<Vec<_>>()
            ));
        }
        Ok(())
    }
}

struct ReplayState {
    workflow: Workflow,
    candidate: EditorProject,
    batches: Vec<(String, String, Vec<Tokens>)>,
    index: usize,
    script: String,
}

impl ReplayState {
    fn new(mut workflow: Workflow) -> Self {
        let mut before = Vec::new();
        let mut after = Vec::new();
        for ((phase, sequence, directory), commands) in std::mem::take(&mut workflow.phases) {
            let label = match phase {
                0 => "project_settings",
                5 => "import_graphics",
                60 => "component_definition",
                70 => "component_instance",
                90 => "object_order",
                _ => unreachable!("object commands are grouped separately"),
            };
            let stem = if sequence == 0 {
                label.to_owned()
            } else {
                format!("{label}_{sequence}")
            };
            append_batches(
                if phase <= 5 { &mut before } else { &mut after },
                &directory,
                &stem,
                commands,
            );
        }
        // Reserve complete phase filenames, including split suffixes, before
        // allocating object filenames in the same directories.
        let mut names = Names::default();
        for (path, _, _) in before.iter().chain(&after) {
            names
                .0
                .insert(path.trim_end_matches(".aitd").to_ascii_lowercase());
        }
        let mut batches = before;
        for (id, path, directory, commands) in std::mem::take(&mut workflow.object_batches) {
            let stem = Path::new(&path).file_stem().unwrap().to_str().unwrap();
            let stem = names.reserve(&directory, stem.to_owned(), &id.to_string());
            batches.push((format!("{directory}/{stem}.aitd"), directory, commands));
        }
        batches.extend(after);
        Self {
            workflow,
            candidate: EditorProject::new_pool(),
            batches,
            index: 0,
            script: String::from(BUILD_START),
        }
    }

    fn fraction(&self) -> f32 {
        self.index as f32 / self.batches.len().max(1) as f32
    }

    /// Replay one file, including the CLI's save/reopen boundary.
    fn step(&mut self) -> Result<bool, String> {
        if self.index >= self.batches.len() {
            return Ok(true);
        }
        let (path, directory, chunk) = &self.batches[self.index];
        let text = chunk
            .iter()
            .map(|tokens| {
                if tokens.first().is_some_and(|token| token == "#") {
                    format!("# {}", tokens[1..].join(" ").replace(['\n', '\r'], " "))
                } else {
                    crate::cli::commands::serialize_command(tokens)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let actions = crate::cli::commands::parse_commands(&text, Path::new(directory), &|path| {
            self.workflow
                .files
                .get(&normalize(path)?)
                .cloned()
                .ok_or_else(|| format!("Missing asset {}", path.display()))
        })
        .map_err(|error| format!("{path}: {error}"))?;
        crate::cli::editing::execute_batch(
            &mut self.candidate,
            serde_json::json!({"actions": actions}),
        )
        .map_err(|error| format!("{path}: {error}"))?;
        self.candidate =
            EditorProject::load_project(self.candidate.save_project().map_err(|e| e.to_string())?)
                .map_err(|error| format!("{path}: cannot reopen rebuilt project: {error}"))?;
        self.script.push_str(&format!(
            "\"$AITD\" run --file {}\n",
            crate::cli::commands::quote_token(path)
        ));
        if self
            .workflow
            .files
            .insert(path.clone(), text.into_bytes())
            .is_some()
        {
            return Err(format!("Duplicate archive path: {path}"));
        }
        self.index += 1;
        Ok(self.index >= self.batches.len())
    }

    fn finish_archive(mut self) -> CliProjectArchive {
        self.script.push_str(BUILD_END);
        self.workflow
            .files
            .insert("build.sh".into(), self.script.into_bytes());
        self.workflow
            .files
            .insert("README.txt".into(), README.as_bytes().to_vec());
        CliProjectArchive {
            files: self.workflow.files,
        }
    }
}

fn normalize(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => (),
            Component::ParentDir => {
                parts.pop().ok_or("Asset path escapes archive")?;
            }
            _ => return Err("Absolute asset path in archive".into()),
        }
    }
    Ok(parts.join("/"))
}

fn scalar(value: &Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.to_string()),
        _ => Err("Expected a readable scalar".into()),
    }
}

fn leaves(value: &Value, prefix: &str, result: &mut Vec<String>) -> Result<(), String> {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                leaves(
                    value,
                    &if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{prefix}.{name}")
                    },
                    result,
                )?;
            }
        }
        // Array lengths are authored with array commands after scalar fields.
        Value::Array(_) => (),
        _ => result.push(format!("{prefix}={}", scalar(value)?)),
    }
    Ok(())
}

fn property_commands(id: &str, key: PropertyKey, value: &Value) -> Result<Vec<Tokens>, String> {
    let mut commands = Vec::new();
    emit_property_value(id, &key.to_string(), value, &mut commands)?;
    Ok(commands)
}

pub(crate) fn object_name(project: &EditorProject, id: u16) -> String {
    let object_id = ag_iso_stack::object_pool::ObjectId::new(id).expect("valid pool object ID");
    let object = project
        .get_pool()
        .object_by_id(object_id)
        .expect("referenced pool object exists");
    project.get_object_info(object).get_name(object)
}

/// A shell-friendly object target that preserves reserved names literally.
pub(crate) fn object_token(project: &EditorProject, id: u16) -> String {
    let name = crate::smart_naming::normalized_name(&object_name(project, id));
    if name == "null" || name.starts_with("name:") {
        format!("name:{name}")
    } else {
        name
    }
}

fn object_reference_name(project: &EditorProject, value: &Value) -> Result<String, String> {
    if value.is_null() {
        return Ok("null".into());
    }
    let id = serde_json::from_value::<ag_iso_stack::object_pool::ObjectId>(value.clone())
        .map_err(|error| format!("Invalid object reference {value}: {error}"))?;
    Ok(object_token(project, id.value()))
}

fn macro_name_arguments(
    project: &EditorProject,
    instruction: &crate::macro_authoring::MacroInstruction,
    references: &BTreeSet<u16>,
) -> Result<crate::macro_authoring::MacroInstruction, String> {
    let original = instruction;
    let mut instruction = instruction.clone();
    let positions: &[usize] = match instruction.name.as_str() {
        "hide-show-object"
        | "enable-disable-object"
        | "select-input-object"
        | "change-size"
        | "change-background-colour"
        | "change-numeric-value"
        | "change-end-point"
        | "change-font-attributes"
        | "change-attribute"
        | "change-priority"
        | "change-polygon-point"
        | "change-polygon-scale"
        | "change-string-value"
        | "change-string-value-wide"
        | "execute-extended-macro" => &[0],
        "change-child-location" | "change-child-position" => &[0, 1],
        "change-line-attributes" => &[0],
        "change-fill-attributes" => &[0, 3],
        "change-active-mask" => &[0, 1],
        "change-soft-key-mask" => &[1, 2],
        "change-list-item" => &[0, 2],
        "change-object-label" => &[0, 1, 3],
        "select-colour-map-or-palette" => &[0],
        "lock-unlock-mask" => &[1],
        // Execute Macro stores an 8-bit ID; the CLI still resolves it by name.
        "execute-macro" => &[0],
        _ => &[],
    };
    let mut positions = positions.to_vec();
    if instruction.name == "graphics-context" {
        positions.push(0);
        match instruction.arguments.get(1).map(String::as_str) {
            Some("set-line-attributes" | "set-fill-attributes" | "set-font-attributes")
            | Some(
                "draw-vt-object"
                | "copy-canvas-to-picture-graphic"
                | "copy-viewport-to-picture-graphic",
            ) => positions.push(2),
            _ => (),
        }
    }
    for position in &positions {
        if let Some(argument) = instruction.arguments.get_mut(*position) {
            if argument != "null" {
                if let Ok(id) = argument.parse::<u16>() {
                    if references.contains(&id) {
                        *argument = object_token(project, id);
                    }
                }
            }
        }
    }
    let value_index = match instruction.name.as_str() {
        "change-attribute" => Some((2, Some(1))),
        "change-numeric-value" => Some((1, None)),
        _ => None,
    };
    if let Some((value_index, aid_index)) = value_index {
        let args = &original.arguments;
        let target_id = args
            .get(0)
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| format!("{} target ID is invalid", instruction.name))?;
        let target = project
            .get_pool()
            .object_by_id(
                ag_iso_stack::object_pool::ObjectId::new(target_id)
                    .map_err(|e| format!("Invalid macro target ID: {e:?}"))?,
            )
            .ok_or_else(|| format!("Macro target object {target_id} not found"))?;
        let aid = aid_index
            .map(|index| {
                args.get(index)
                    .and_then(|value| value.parse::<u8>().ok())
                    .ok_or_else(|| "Change Attribute AID is invalid".to_owned())
            })
            .transpose()?;
        if let crate::macro_id_remap::MacroAttributeValueKind::LocalObjectId16 { value_offset } =
            crate::macro_id_remap::macro_attribute_value_kind(
                target.object_type(),
                aid,
                project.vt_version(),
            )
        {
            let argument = args
                .get(value_index)
                .ok_or_else(|| format!("{} value is missing", instruction.name))?;
            let packed = argument
                .parse::<u32>()
                .map_err(|_| format!("{} object-valued field is not numeric", instruction.name))?;
            let shift = value_offset * 8;
            let id = ((packed >> shift) & 0xffff) as u16;
            let reference_only = u32::from(id) << shift;
            if packed != reference_only {
                return Err(format!(
                    "{} object-valued field for object {target_id} AID {:?} has non-reference padding; cannot export it as a name without changing its value",
                    instruction.name, aid,
                ));
            }
            if id == u16::MAX {
                instruction.arguments[value_index] = "null".into();
            } else if references.contains(&id) {
                instruction.arguments[value_index] = object_token(project, id);
            }
        }
    }
    Ok(instruction)
}

fn emit_property_value(
    id: &str,
    path: &str,
    value: &Value,
    commands: &mut Vec<Tokens>,
) -> Result<(), String> {
    match value {
        Value::Object(_) => {
            let mut tokens = vec!["object".into(), "field".into(), id.into(), path.into()];
            leaves(value, "", &mut tokens)?;
            if tokens.len() > 4 {
                commands.push(tokens);
            }
            emit_nested_arrays(id, path, value, commands)?;
        }
        Value::Array(items) => {
            commands.push(vec![
                "object".into(),
                "array".into(),
                "clear".into(),
                id.into(),
                path.into(),
            ]);
            for (index, item) in items.iter().enumerate() {
                let mut tokens = vec![
                    "object".into(),
                    "array".into(),
                    "append".into(),
                    id.into(),
                    path.into(),
                ];
                if item.is_object() {
                    leaves(item, "", &mut tokens)?;
                } else if let Value::Array(fields) = item {
                    // Fixed tuple records, such as language pairs, use the
                    // normal field editor's numeric field names.
                    for (field, value) in fields.iter().enumerate() {
                        tokens.push(format!("{field}={}", scalar(value)?));
                    }
                } else {
                    tokens.push(scalar(item)?);
                }
                commands.push(tokens);
                emit_nested_arrays(id, &format!("{path}.{index}"), item, commands)?;
            }
        }
        _ => commands.push(vec![
            "object".into(),
            "set".into(),
            id.into(),
            format!("{path}={}", scalar(value)?),
        ]),
    }
    Ok(())
}

fn emit_nested_arrays(
    id: &str,
    path: &str,
    value: &Value,
    commands: &mut Vec<Tokens>,
) -> Result<(), String> {
    if let Value::Object(fields) = value {
        for (name, value) in fields {
            let path = format!("{path}.{name}");
            if value.is_array() {
                emit_property_value(id, &path, value, commands)?;
            } else if value.is_object() {
                emit_nested_arrays(id, &path, value, commands)?;
            }
        }
    }
    Ok(())
}

fn initial_workflow(
    project: &EditorProject,
    directories: &BTreeMap<u16, String>,
) -> Result<Workflow, String> {
    let mut workflow = Workflow::default();
    let types = project
        .get_pool()
        .objects()
        .iter()
        .map(|o| (o.id().value(), o.object_type()))
        .collect();
    let mut names = Names::default();
    for object in project.get_pool().objects() {
        let id = object.id().value();
        let display_name = object_name(project, id);
        if project.get_object_info(object).id_overridden {
            workflow.fixed_ids.insert(id);
        }
        if let Some(other) = workflow
            .named_targets
            .insert(crate::smart_naming::normalized_name(&display_name), id)
        {
            return Err(format!(
                "Pool object name '{}' is ambiguous with object {}",
                display_name, other
            ));
        }
        let mut dependencies: BTreeSet<_> = object
            .referenced_objects()
            .into_iter()
            .map(|id| id.value())
            .collect();
        dependencies.extend(
            project
                .get_object_info(object)
                .allowed_references
                .iter()
                .map(|id| id.value()),
        );
        if let Some(bindings) = macro_refs(object) {
            dependencies.extend(bindings.iter().map(|binding| binding.macro_id));
        }
        if matches!(object, Object::PictureGraphic(_)) {
            // Indexed PNG import compares against the effective palette, which
            // is selected by special controls rather than the picture itself.
            if let Some(controls) = project
                .get_pool()
                .objects()
                .iter()
                .filter(|o| matches!(o, Object::WorkingSetSpecialControls(_)))
                .min_by_key(|o| o.id().value())
            {
                dependencies.insert(controls.id().value());
                dependencies.extend(
                    controls
                        .referenced_objects()
                        .into_iter()
                        .map(|id| id.value()),
                );
            }
        }
        if let Object::Macro(mac) = object {
            if let Ok(ids) = crate::macro_id_remap::macro_object_references(
                &mac.commands,
                project.vt_version(),
                &types,
            ) {
                dependencies.extend(ids);
            }
        }
        let directory = directories[&id].clone();
        let stem = names.allocate(&directory, Some(&display_name), &id.to_string());
        workflow.objects.insert(
            id,
            ObjectCommands {
                directory,
                stem,
                dependencies,
                commands: Vec::new(),
            },
        );
    }
    let layout = project.soft_key_layout();
    workflow.command(
        0,
        "shared",
        [
            "project".into(),
            "settings".into(),
            "set".into(),
            format!("mask_size={}", project.mask_size),
            format!("vt_version={}", u8::from(project.vt_version())),
            format!("soft_key_width={}", project.soft_key_size().0),
            format!("soft_key_height={}", project.soft_key_size().1),
            format!("arrangement={:?}", layout.arrangement),
            format!("physical_key_count={}", layout.physical_key_count),
            format!("keys_per_line={}", layout.keys_per_line),
            format!("navigation_key_count={}", layout.navigation_key_count),
        ],
    );
    emit_id_range_settings(&mut workflow, project)?;
    Ok(workflow)
}

fn emit_project_object_part(
    workflow: &mut Workflow,
    project: &EditorProject,
    object: &Object,
    dir: &str,
    part: usize,
) -> Result<bool, String> {
    if part == 0 {
        let id = object_token(project, object.id().value());
        let object_type =
            scalar(&serde_json::to_value(object.object_type()).map_err(|e| e.to_string())?)?;
        let info = project.get_object_info(object);
        let mut tokens = vec![
            "object".into(),
            "create".into(),
            object_type,
            info.name.clone().unwrap_or_else(|| id.clone()),
        ];
        if info.id_overridden {
            tokens.extend(["--override-id".into(), object.id().value().to_string()]);
        }
        workflow.command(10, dir, tokens);
        return Ok(false);
    }
    let descriptors = get_property_descriptors(object.object_type());
    if let Some(descriptor) = descriptors.get(part - 1) {
        let id = object_token(project, object.id().value());

        let key = descriptor.name;
        let value =
            get_object_property(object, key).map_err(|e| format!("Object {id}, {key}: {e}"))?;
        if matches!(object, Object::PictureGraphic(_))
            && matches!(
                key,
                PropertyKey::Data
                    | PropertyKey::Format
                    | PropertyKey::ActualWidth
                    | PropertyKey::ActualHeight
            )
        {
            return Ok(false);
        }
        if matches!(object, Object::Macro(_)) && key == PropertyKey::Commands {
            return Ok(false);
        }
        if matches!(key, PropertyKey::Data | PropertyKey::Commands) {
            return Err(format!(
                "Object {id}, {key}: binary content has no supported readable representation"
            ));
        }
        if !descriptor.writable {
            return Err(format!(
                "Object {id}, {key}: read-only content cannot be exported"
            ));
        }
        if let Some((encoding, text)) = crate::cli::string_authoring::export_value(&value)
            .map_err(|e| format!("Object {id}, {key}: {e}"))?
        {
            workflow.command(
                20,
                dir,
                vec![
                    "object".into(),
                    "text-set".into(),
                    id.clone(),
                    key.to_string(),
                    text,
                    "--encoding".into(),
                    encoding,
                ],
            );
            return Ok(false);
        }
        if matches!(
            descriptor.semantic,
            PropertySemantic::ObjectReference { .. }
        ) {
            workflow.command(
                30,
                dir,
                vec![
                    "reference".into(),
                    "set".into(),
                    id.clone(),
                    key.to_string(),
                    object_reference_name(project, &value)?,
                ],
            );
        } else {
            for tokens in property_commands(&id, key, &value)
                .map_err(|e| format!("Object {id}, {key}: {e}"))?
            {
                workflow.command(20, dir, tokens);
            }
        }
        return Ok(false);
    }
    emit_object_extras(workflow, project, object, dir)?;
    Ok(true)
}

fn finish_workflow(mut workflow: Workflow, project: &EditorProject) -> Result<Workflow, String> {
    let mut order = vec!["object".into(), "order".into()];
    order.extend(
        project
            .get_pool()
            .objects()
            .iter()
            .map(|o| object_token(project, o.id().value())),
    );
    if order.len() > 2 {
        workflow.command(90, "shared", order);
    }
    workflow.order_objects()?;
    Ok(workflow)
}

struct GenerationState {
    workflow: Workflow,
    directories: BTreeMap<u16, String>,
    object_index: usize,
    object_part: usize,
    assets: std::vec::IntoIter<crate::graphics::GraphicAssetId>,
    actions: std::vec::IntoIter<crate::cli::component_authoring::ComponentAuthoringAction>,
    components: ComponentEmission,
    completed: usize,
    total: usize,
}

impl GenerationState {
    fn new(project: &EditorProject) -> Result<Self, String> {
        let actions = crate::cli::component_authoring::export_actions(project.component_source())?;
        let assets: Vec<_> = project.graphic_library().assets.keys().copied().collect();
        let total = project
            .get_pool()
            .objects()
            .iter()
            .map(|object| get_property_descriptors(object.object_type()).len() + 2)
            .sum::<usize>()
            + assets.len()
            + actions.len()
            + 1;
        let directories = object_directories(project);
        Ok(Self {
            workflow: initial_workflow(project, &directories)?,
            directories,
            object_index: 0,
            object_part: 0,
            assets: assets.into_iter(),
            actions: actions.into_iter(),
            components: ComponentEmission::default(),
            completed: 0,
            total,
        })
    }

    fn fraction(&self) -> f32 {
        self.completed as f32 / self.total as f32
    }

    fn step(&mut self, project: &EditorProject) -> Result<bool, String> {
        if let Some(object) = project.get_pool().objects().get(self.object_index) {
            self.workflow.current_object = Some(object.id().value());
            let result = emit_project_object_part(
                &mut self.workflow,
                project,
                object,
                &self.directories[&object.id().value()],
                self.object_part,
            );
            self.workflow.current_object = None;
            let done = result?;
            self.object_part += 1;
            if done {
                self.object_index += 1;
                self.object_part = 0;
            }
        } else if let Some(id) = self.assets.next() {
            emit_graphic_asset(&mut self.workflow, &project.graphic_library().assets[&id])?;
        } else if let Some(action) = self.actions.next() {
            self.components
                .emit(&mut self.workflow, project, &self.directories, action)?;
        } else {
            self.workflow = finish_workflow(std::mem::take(&mut self.workflow), project)?;
            self.completed += 1;
            return Ok(true);
        }
        self.completed += 1;
        Ok(false)
    }
}

fn build_workflow(project: &EditorProject) -> Result<Workflow, String> {
    let mut state = GenerationState::new(project)?;
    while !state.step(project)? {}
    Ok(state.workflow)
}

fn generate_archive(project: &EditorProject) -> Result<CliProjectArchive, String> {
    validate(project)?;
    build_workflow(project)?.finish(project)
}

/// Generate and execute a readable workflow against a stable project copy.
/// Unsupported content, invalid projects and persisted or IOP mismatches fail
/// before an archive is returned. Source content and dirty state are unchanged.
pub fn generate(project: &EditorProject) -> Result<CliProjectArchive, String> {
    generate_archive(&project.clone())
}

fn object_directories(project: &EditorProject) -> BTreeMap<u16, String> {
    // Traverse the materialized relationships so instance overrides and external
    // component references participate in resource ownership as ordinary edges.
    let materialized = project
        .materialize_shared()
        .expect("generation validated the snapshot");
    let pool = &materialized.pool;
    let object_types = pool
        .objects()
        .iter()
        .map(|o| (o.id().value(), o.object_type()))
        .collect();
    let masks: Vec<_> = project
        .get_pool()
        .objects()
        .iter()
        .filter(|o| {
            matches!(
                o.object_type(),
                ObjectType::DataMask | ObjectType::AlarmMask
            )
        })
        .collect();
    let mut owners: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for mask in &masks {
        let mask_id = mask.id().value();
        let mut pending = vec![mask.id()];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id.value()) {
                continue;
            }
            if let Some(object) = pool.object_by_id(id) {
                if id != mask.id()
                    && matches!(
                        object.object_type(),
                        ObjectType::DataMask | ObjectType::AlarmMask | ObjectType::WorkingSet
                    )
                {
                    continue;
                }
                owners.entry(id.value()).or_default().insert(mask_id);
                pending.extend(object.referenced_objects());
                if let Some(bindings) = macro_refs(object) {
                    pending.extend(bindings.iter().filter_map(|binding| {
                        ag_iso_stack::object_pool::ObjectId::new(binding.macro_id).ok()
                    }));
                }
                if let Object::Macro(mac) = object {
                    if let Ok(references) = crate::macro_id_remap::macro_object_references(
                        &mac.commands,
                        project.vt_version(),
                        &object_types,
                    ) {
                        pending.extend(
                            references
                                .into_iter()
                                .filter_map(|id| ag_iso_stack::object_pool::ObjectId::new(id).ok()),
                        );
                    }
                }
                if let Some(primitive) = project.get_pool().object_by_id(id) {
                    pending.extend(project.get_object_info(primitive).allowed_references);
                }
            }
        }
    }
    let mut names = Names::default();
    let mask_dirs: BTreeMap<_, _> = masks
        .iter()
        .map(|mask| {
            let id = mask.id().value();
            let stem = names.allocate(
                "masks",
                project.get_object_info(mask).name.as_deref(),
                &id.to_string(),
            );
            (id, format!("masks/{stem}"))
        })
        .collect();
    project
        .get_pool()
        .objects()
        .iter()
        .map(|object| {
            let id = object.id().value();
            let directory = if object.object_type() == ObjectType::WorkingSet {
                "shared".into()
            } else if let Some(dir) = mask_dirs.get(&id) {
                dir.clone()
            } else {
                owners
                    .get(&id)
                    .filter(|owners| owners.len() == 1)
                    .and_then(|owners| mask_dirs.get(owners.first().unwrap()))
                    .cloned()
                    .unwrap_or_else(|| "shared".into())
            };
            (id, directory)
        })
        .collect()
}

fn emit_object_extras(
    workflow: &mut Workflow,
    project: &EditorProject,
    object: &Object,
    dir: &str,
) -> Result<(), String> {
    let id = object_token(project, object.id().value());
    if let Some(children) = object_refs(object) {
        for child in children {
            workflow.command(
                40,
                dir,
                vec![
                    "placement".into(),
                    "add".into(),
                    id.clone(),
                    object_token(project, child.id.value()),
                    child.offset.x.to_string(),
                    child.offset.y.to_string(),
                ],
            );
        }
    }
    if let Some(list) = object_list(object) {
        let mut tokens = vec!["list".into(), "set".into(), id.clone()];
        tokens.extend(match list {
            ObjectReferenceList::Required(items) => items
                .into_iter()
                .map(|id| object_token(project, id.value()))
                .collect::<Vec<_>>(),
            ObjectReferenceList::Nullable(items) => items
                .into_iter()
                .map(|id| {
                    id.0.map(|id| object_token(project, id.value()))
                        .unwrap_or_else(|| "null".into())
                })
                .collect(),
        });
        workflow.command(40, dir, tokens);
    }
    if let Some(bindings) = macro_refs(object) {
        if !bindings.is_empty() {
            let mut tokens = vec!["event".into(), "set".into(), id.clone()];
            for binding in bindings {
                tokens.push(format!(
                    "{}={}",
                    scalar(&serde_json::to_value(binding.event_id).map_err(|e| e.to_string())?)?,
                    object_token(project, binding.macro_id)
                ));
            }
            workflow.command(40, dir, tokens);
        }
    }
    let info = project.get_object_info(object);
    if !info.allowed_references.is_empty() {
        let mut tokens = vec!["pointer".into(), "set".into(), id.clone()];
        tokens.extend(
            info.allowed_references
                .iter()
                .map(|id| object_token(project, id.value())),
        );
        workflow.command(40, dir, tokens);
    }
    if let Object::ObjectLabelReferenceList(labels) = object {
        for label in &labels.object_labels {
            workflow.command(
                40,
                dir,
                vec![
                    "label".into(),
                    "add".into(),
                    id.clone(),
                    format!("id={}", object_token(project, label.id.value())),
                    format!(
                        "string={}",
                        label
                            .string_variable_reference
                            .0
                            .map(|id| object_token(project, id.value()))
                            .unwrap_or_else(|| "null".into())
                    ),
                    format!(
                        "graphic={}",
                        label
                            .graphic_representation
                            .0
                            .map(|id| object_token(project, id.value()))
                            .unwrap_or_else(|| "null".into())
                    ),
                    format!("font={}", label.font_type),
                ],
            );
        }
    }
    emit_content(workflow, project, object, dir)
}

fn emit_content(
    workflow: &mut Workflow,
    project: &EditorProject,
    object: &Object,
    dir: &str,
) -> Result<(), String> {
    let id = object_token(project, object.id().value());
    let info = project.get_object_info(object);
    workflow.command(
        80,
        dir,
        crate::cli::commands::object_metadata_tokens(&id, &info),
    );
    match object {
        Object::PictureGraphic(picture) => {
            let png = crate::graphics::indexed_png::picture_png(picture, project.get_pool())
                .map_err(|e| format!("Object {id}, data: {e}"))?;
            crate::graphics::indexed_png::verify_reproduces(
                picture,
                project.get_pool(),
                &png,
                picture.format,
                picture.options.data_code_type,
            )
            .map_err(|e| {
                format!("Object {id}, data: original picture encoding cannot be reproduced: {e}")
            })?;
            let stem = workflow.asset_names.allocate(
                "assets/graphics",
                project.get_object_info(object).name.as_deref(),
                &id,
            );
            let path = format!("assets/graphics/{stem}.png");
            workflow.files.insert(path.clone(), png);
            let relative = if dir.starts_with("masks/") {
                format!("../../{path}")
            } else {
                format!("../{path}")
            };
            use ag_iso_stack::object_pool::object_attributes::{
                DataCodeType, PictureGraphicFormat,
            };
            let format = match picture.format {
                PictureGraphicFormat::Monochrome => "monochrome",
                PictureGraphicFormat::FourBit => "four_bit",
                PictureGraphicFormat::EightBit => "eight_bit",
            };
            let encoding = match picture.options.data_code_type {
                DataCodeType::Raw => "raw",
                DataCodeType::RunLength => "run_length",
            };
            workflow.command(
                50,
                dir,
                vec![
                    "graphic".into(),
                    "assign-file".into(),
                    object_token(project, object.id().value()),
                    relative,
                    "--format".into(),
                    format.into(),
                    "--encoding".into(),
                    encoding.into(),
                ],
            );
        }
        Object::Macro(mac) => {
            let types = project
                .get_pool()
                .objects()
                .iter()
                .map(|o| (o.id().value(), o.object_type()))
                .collect();
            for instruction in
                crate::macro_authoring::decode_macro_commands(&mac.commands, project.vt_version())
                    .map_err(|e| format!("Object {id}, commands: {e}"))?
            {
                let bytes = crate::macro_authoring::encode_macro_instruction(
                    &instruction.name,
                    &instruction.arguments,
                    project.vt_version(),
                )
                .map_err(|e| format!("Object {id}, commands: {e}"))?;
                let targets = crate::macro_id_remap::macro_object_references(
                    &bytes,
                    project.vt_version(),
                    &types,
                )
                .map_err(|e| format!("Object {id}, commands: {e}"))?;
                let instruction = macro_name_arguments(
                    project,
                    &instruction,
                    &targets.iter().copied().collect(),
                )?;
                let tokens = crate::cli::commands::macro_instruction_tokens(&id, &instruction);
                workflow.command(50, dir, tokens.clone());
                workflow
                    .command_targets
                    .insert(tokens, targets.into_iter().collect());
            }
        }
        _ => (),
    }
    Ok(())
}

fn emit_id_range_settings(workflow: &mut Workflow, project: &EditorProject) -> Result<(), String> {
    let mut tokens = vec!["project".into(), "settings".into(), "set".into()];
    for (ty, range) in project.object_id_ranges().iter() {
        tokens.push(format!(
            "id_range.{}={}:{}",
            scalar(&serde_json::to_value(ty).map_err(|e| e.to_string())?)?,
            range.start,
            range.end
        ));
    }
    if tokens.len() > 3 {
        workflow.command(0, "shared", tokens);
    }
    Ok(())
}

fn emit_graphic_asset(
    workflow: &mut Workflow,
    asset: &crate::graphics::GraphicAsset,
) -> Result<(), String> {
    let png = asset
        .as_png()
        .map_err(|e| format!("Graphic {}, original: {e}", asset.id))?
        .original_bytes;
    let stem =
        workflow
            .asset_names
            .allocate("assets/graphics", Some(&asset.name), &asset.id.to_string());
    let path = format!("assets/graphics/{stem}.png");
    workflow.files.insert(path.clone(), png);
    let mut tokens = vec![
        "graphic".into(),
        "import".into(),
        format!("../{path}"),
        "--name".into(),
        asset.name.clone(),
    ];
    if let Some(description) = &asset.description {
        tokens.extend(["--description".into(), description.clone()]);
    }
    for tag in &asset.tags {
        tokens.extend(["--tag".into(), tag.clone()]);
    }
    if let Some(name) = &asset.original_file_name {
        tokens.extend(["--original-name".into(), name.clone()]);
    } else {
        tokens.push("--clear-original-name".into());
    }
    workflow.command(5, "shared", tokens);
    Ok(())
}

fn component_directory(project: &EditorProject, id: &str) -> String {
    let mut names = Names::default();
    for (key, definition) in &project.component_source().definitions {
        let stem = names.allocate("components", Some(&definition.name), &key.to_string());
        if key.to_string() == id {
            return format!("components/{stem}");
        }
    }
    unreachable!("exported component exists")
}

fn component_name(project: &EditorProject, id: crate::components::ComponentId) -> &str {
    &project.component_source().definitions[&id].name
}

fn component_node_name(
    project: &EditorProject,
    component_id: crate::components::ComponentId,
    node_id: crate::components::ComponentNodeId,
) -> &str {
    &project.component_source().definitions[&component_id]
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .expect("exported component node exists")
        .name
}

#[derive(Default)]
struct ComponentEmission {
    sequence: usize,
    content: BTreeMap<String, Vec<Tokens>>,
}

impl ComponentEmission {
    fn emit(
        &mut self,
        workflow: &mut Workflow,
        project: &EditorProject,
        directories: &BTreeMap<u16, String>,
        action: crate::cli::component_authoring::ComponentAuthoringAction,
    ) -> Result<(), String> {
        use crate::cli::component_authoring::{
            export_command_tokens, ComponentAuthoringAction as A,
        };
        let (phase, dir) = match &action {
            A::InstanceCreate { parent, .. } => (
                70,
                directories
                    .get(parent)
                    .cloned()
                    .unwrap_or_else(|| "shared".into()),
            ),
            A::DefinitionCreate { id, .. } => (60, component_directory(project, &id.to_string())),
            _ => {
                // All other exported definition actions carry component_id.
                let value = serde_json::to_value(&action).map_err(|e| e.to_string())?;
                let id = value["component_id"]
                    .as_str()
                    .ok_or("Missing component identity in export action")?;
                (60, component_directory(project, id))
            }
        };
        if let A::NodePropertySet {
            component_id,
            node_id,
            property,
            value,
        } = &action
        {
            if let Some((encoding, text)) = crate::cli::string_authoring::export_value(value)
                .map_err(|e| format!("Component {component_id}, node {node_id}, {property}: {e}"))?
            {
                workflow.command(
                    phase,
                    &dir,
                    vec![
                        "component".into(),
                        "node-text-set".into(),
                        component_name(project, *component_id).into(),
                        component_node_name(project, *component_id, *node_id).into(),
                        property.to_string(),
                        text,
                        "--encoding".into(),
                        encoding,
                    ],
                );
                return Ok(());
            }
            if matches!(property, PropertyKey::Data | PropertyKey::Commands) {
                let node = project.component_source().definitions[component_id]
                    .nodes
                    .iter()
                    .find(|node| node.id == *node_id)
                    .unwrap();
                let context = format!("Component {component_id}, node {node_id}, {property}");
                if node.object_type == ObjectType::PictureGraphic && *property == PropertyKey::Data
                {
                    let picture = crate::graphics::indexed_png::picture_from_component_properties(
                        &node.properties,
                    )
                    .map_err(|e| format!("{context}: {e}"))?;
                    let png =
                        crate::graphics::indexed_png::picture_png(&picture, project.get_pool())
                            .map_err(|e| format!("{context}: {e}"))?;
                    crate::graphics::indexed_png::verify_reproduces(
                        &picture,
                        project.get_pool(),
                        &png,
                        picture.format,
                        picture.options.data_code_type,
                    )
                    .map_err(|e| format!("{context}: {e}"))?;
                    let stem = workflow.asset_names.allocate(
                        "assets/graphics",
                        Some(&node.name),
                        &format!("{component_id}_{node_id}"),
                    );
                    let path = format!("assets/graphics/{stem}.png");
                    workflow.files.insert(path.clone(), png);
                    use ag_iso_stack::object_pool::object_attributes::{
                        DataCodeType, PictureGraphicFormat,
                    };
                    let format = match picture.format {
                        PictureGraphicFormat::Monochrome => "monochrome",
                        PictureGraphicFormat::FourBit => "four_bit",
                        PictureGraphicFormat::EightBit => "eight_bit",
                    };
                    let encoding = match picture.options.data_code_type {
                        DataCodeType::Raw => "raw",
                        DataCodeType::RunLength => "run_length",
                    };
                    self.content.entry(dir.clone()).or_default().push(vec![
                        "component".into(),
                        "node-assign-file".into(),
                        component_name(project, *component_id).into(),
                        component_node_name(project, *component_id, *node_id).into(),
                        format!("../../{path}"),
                        "--format".into(),
                        format.into(),
                        "--encoding".into(),
                        encoding.into(),
                    ]);
                } else if node.object_type == ObjectType::Macro
                    && *property == PropertyKey::Commands
                {
                    let bytes: Vec<u8> = serde_json::from_value(value.clone())
                        .map_err(|e| format!("{context}: {e}"))?;
                    let instructions =
                        crate::macro_authoring::decode_macro_commands(&bytes, project.vt_version())
                            .map_err(|e| format!("{context}: {e}"))?;
                    // An explicit clear also preserves a deliberately empty property.
                    self.content.entry(dir.clone()).or_default().push(vec![
                        "component".into(),
                        "node-macro-clear".into(),
                        component_name(project, *component_id).into(),
                        component_node_name(project, *component_id, *node_id).into(),
                    ]);
                    for instruction in instructions {
                        let bytes = crate::macro_authoring::encode_macro_instruction(
                            &instruction.name,
                            &instruction.arguments,
                            project.vt_version(),
                        )
                        .map_err(|e| format!("{context}: {e}"))?;
                        let types = project
                            .get_pool()
                            .objects()
                            .iter()
                            .map(|object| (object.id().value(), object.object_type()))
                            .collect();
                        let references = crate::macro_id_remap::macro_object_references(
                            &bytes,
                            project.vt_version(),
                            &types,
                        )
                        .map_err(|e| format!("{context}: {e}"))?;
                        let instruction = macro_name_arguments(
                            project,
                            &instruction,
                            &references.into_iter().collect(),
                        )?;
                        let mut tokens = vec![
                            "component".into(),
                            "node-macro-append".into(),
                            component_name(project, *component_id).into(),
                            component_node_name(project, *component_id, *node_id).into(),
                            instruction.name,
                        ];
                        tokens.extend(instruction.arguments);
                        self.content.entry(dir.clone()).or_default().push(tokens);
                    }
                } else {
                    return Err(format!(
                        "{context}: binary content has no supported readable representation"
                    ));
                }
                return Ok(());
            }
        }
        if matches!(action, A::DefinitionMetadata { .. }) {
            for tokens in self.content.remove(&dir).unwrap_or_default() {
                workflow.command(phase, &dir, tokens);
            }
        }
        if phase == 70 {
            self.sequence += 1;
            workflow
                .phases
                .entry((phase, self.sequence, dir))
                .or_default()
                .push(export_command_tokens(&action, project)?);
        } else {
            workflow.command(phase, &dir, export_command_tokens(&action, project)?);
        }
        Ok(())
    }
}

fn first_difference(left: &Value, right: &Value, path: &str) -> Option<String> {
    if left == right {
        return None;
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                if let Some(path) = first_difference(
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                    &format!("{path}/{key}"),
                ) {
                    return Some(path);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (left, right)) in a.iter().zip(b).enumerate() {
                let label = if path == "project/objects" {
                    left.as_object()
                        .and_then(|o| o.values().next())
                        .and_then(|o| o.get("id"))
                        .map(|id| format!("object {id}"))
                        .unwrap_or_else(|| index.to_string())
                } else {
                    index.to_string()
                };
                if let Some(path) = first_difference(left, right, &format!("{path}/{label}")) {
                    return Some(path);
                }
            }
        }
        _ => (),
    }
    Some(path.to_owned())
}

fn validate(project: &EditorProject) -> Result<(), String> {
    let result = crate::cli::automation_service::execute(
        project,
        crate::cli::protocol::AutomationAction::ValidateDesign,
        serde_json::json!({}),
    )
    .map_err(|e| e.message)?;
    if result["valid"] != true {
        return Err(format!(
            "Project validation failed: {}",
            result["diagnostics"]
        ));
    }
    Ok(())
}

const BUILD_START: &str = r##"#!/usr/bin/env bash
set -euo pipefail
script=${BASH_SOURCE[0]}
if [[ $script == */* ]]; then cd -- "${script%/*}"; fi
AITD=${AITD:-aitd}
# Use this Bash process as the CLI session owner; do not invoke through pipes.
if [[ -e build || -L build ]]; then
  printf '%s\n' 'Refusing to overwrite build/. Move or remove it before rebuilding.' >&2
  exit 1
fi
cleanup() {
  "$AITD" project close --discard >/dev/null 2>&1 || true
}
trap cleanup EXIT
"$AITD" project new
"##;

const BUILD_END: &str = r##""$AITD" validate
# Both artifacts become visible only after every command succeeds.
"$AITD" export project --output-dir build
printf '%s\n' 'Built build/project.aitp and build/project.iop'
"##;

const README: &str = "CLI authoring project\n\nPrerequisites: Bash and an installed aitd matching the exporting editor.\nExtract the ZIP, then run: bash build.sh\nFrom elsewhere: bash /path/to/extracted/build.sh\nUse AITD=/path/to/aitd bash build.sh to select a CLI.\n\nThe script owns a temporary CLI session, stops on the first error, and validates\nbefore publishing build/project.aitp and build/project.iop. Existing build/\noutputs are never overwritten. Move or remove build/ before rebuilding.\n\nEdit the readable .aitd command files or PNG assets, then rebuild. Commands use\nthe ordinary CLI authoring language; aitd --help describes available commands.\nshared/ contains working sets, shared resources, and unused objects. masks/\ncontains each data or alarm mask's exclusive objects. components/ holds reusable\ndefinitions. build.sh explicitly lists dependency-first execution order.\nEach object file creates and configures one object. References whose targets\nare not yet available are emitted immediately after the target is configured,\nwith a comment identifying their source. There is no bootstrap file.\nCommand files hold at most 256 actions. Text may span quoted lines.\nassets/graphics/ holds indexed picture PNGs and converted available originals.\nOriginal graphic file encoding is replaced with PNG; decoded pixels are retained.\n";

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::ObjectPool;

    fn variables(count: u16) -> EditorProject {
        let mut pool = ObjectPool::default();
        for id in 1..=count {
            let mut object = crate::default_object(ObjectType::NumberVariable);
            object.mut_id().set_value(id).unwrap();
            if let Object::NumberVariable(value) = &mut object {
                value.value = u32::from(id) * 17;
            }
            pool.add(object);
        }
        EditorProject::from(pool)
    }

    #[test]
    fn fixed_ids_are_reserved_before_automatic_objects_with_dependencies() {
        let first = crate::ObjectIdRanges::defaults()
            .range_for(ObjectType::ObjectPointer)
            .start;
        let mut pool = ObjectPool::default();
        for id in [first, first + 1] {
            let mut object = crate::default_object(ObjectType::ObjectPointer);
            object.mut_id().set_value(id).unwrap();
            if id == first {
                let Object::ObjectPointer(pointer) = &mut object else {
                    unreachable!()
                };
                pointer.value.0 =
                    Some(ag_iso_stack::object_pool::ObjectId::new(first + 1).unwrap());
            }
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        assert!(project.set_object_id_overridden(
            ag_iso_stack::object_pool::ObjectId::new(first).unwrap(),
            true
        ));
        // Reconstruction must create the fixed pointer first despite its forward
        // reference, otherwise the automatic pointer would consume its ID.
        let archive = generate(&project).unwrap();
        assert!(archive.files.values().any(
            |bytes| String::from_utf8_lossy(bytes).contains(&format!("--override-id {first}"))
        ));
    }

    #[test]
    fn macro_names_preserve_numeric_values_and_reserved_object_names() {
        let mut pool = ObjectPool::default();
        for (ty, id) in [
            (ObjectType::NumberVariable, 10),
            (ObjectType::LineAttributes, 11),
            (ObjectType::ObjectPointer, 12),
            (ObjectType::DataMask, 13),
            (ObjectType::Macro, 14),
            (ObjectType::SoftKeyMask, 15),
            (ObjectType::Macro, 16),
        ] {
            let mut object = crate::default_object(ty);
            object.mut_id().set_value(id).unwrap();
            pool.add(object);
        }
        let instructions: &[(&str, &[&str])] = &[
            ("change-numeric-value", &["10", "10"]),
            ("change-line-attributes", &["11", "2", "3", "11"]),
            ("change-numeric-value", &["12", "10"]),
            ("change-attribute", &["13", "2", "15"]),
            ("execute-macro", &["16"]),
        ];
        let version = ag_iso_stack::object_pool::vt_version::VtVersion::Version6;
        let mut commands = Vec::new();
        for (name, args) in instructions {
            commands.extend(
                crate::macro_authoring::encode_macro_instruction(
                    name,
                    &args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>(),
                    version,
                )
                .unwrap(),
            );
        }
        let Object::Macro(mac) = pool
            .object_mut_by_id(ag_iso_stack::object_pool::ObjectId::new(14).unwrap())
            .unwrap()
        else {
            unreachable!()
        };
        mac.commands = commands;
        let mut project = EditorProject::from(pool);
        for (id, name) in [
            (10, "null"),
            (11, "name:Stroke"),
            (12, "Target Pointer"),
            (13, "Main Mask"),
            (14, "Update"),
            (15, "Side Keys"),
            (16, "Helper"),
        ] {
            assert!(project.rename_object(
                ag_iso_stack::object_pool::ObjectId::new(id).unwrap(),
                name.into()
            ));
        }
        let archive = generate(&project).unwrap();
        let text = archive
            .files
            .iter()
            .filter(|(path, _)| path.ends_with(".aitd"))
            .map(|(_, bytes)| std::str::from_utf8(bytes).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("change-numeric-value name:null 10"), "{text}");
        assert!(
            text.contains("change-line-attributes name:name:stroke 2 3 11"),
            "{text}"
        );
        assert!(
            text.contains("change-numeric-value target-pointer name:null"),
            "{text}"
        );
        assert!(
            text.contains("change-attribute main-mask 2 side-keys"),
            "{text}"
        );
        assert!(text.contains("execute-macro helper"), "{text}");
    }

    #[test]
    fn named_files_do_not_overwrite_shared_phases_or_each_other() {
        let mut project = variables(4);
        for (id, name) in [
            (1, "project_settings"),
            (2, "object_order"),
            (3, "A / B"),
            (4, "a & b"),
        ] {
            assert!(project.rename_object(
                ag_iso_stack::object_pool::ObjectId::new(id).unwrap(),
                name.into()
            ));
        }
        let archive = generate(&project).unwrap();
        for path in [
            "shared/project_settings.aitd",
            "shared/object_order.aitd",
            "shared/project_settings_1.aitd",
            "shared/object_order_2.aitd",
            "shared/A___B.aitd",
            "shared/a___b_4.aitd",
        ] {
            assert!(archive.files.contains_key(path), "missing {path}");
        }
        let settings = std::str::from_utf8(&archive.files["shared/project_settings.aitd"]).unwrap();
        assert!(settings.contains("project settings set"));
        let script = std::str::from_utf8(&archive.files["build.sh"]).unwrap();
        for path in archive.files.keys().filter(|path| path.ends_with(".aitd")) {
            assert!(
                script.contains(&crate::cli::commands::quote_token(path)),
                "unreachable {path}"
            );
        }
    }

    #[test]
    fn unnamed_objects_with_the_same_type_name_are_rejected_as_ambiguous() {
        let mut project = variables(2);
        for id in 1..=2 {
            let temporary_name = format!("Temporary {id}");
            assert!(project.rename_object(
                ag_iso_stack::object_pool::ObjectId::new(id).unwrap(),
                temporary_name.clone()
            ));
            let actions = crate::cli::commands::parse_commands(
                &format!("object metadata '{temporary_name}' --clear-name"),
                Path::new("."),
                &|_| unreachable!(),
            )
            .unwrap();
            crate::cli::editing::execute_batch(
                &mut project,
                serde_json::json!({"actions": actions}),
            )
            .unwrap();
        }
        let error = generate(&project).err().unwrap();
        assert!(error.contains("ambiguous"), "{error}");
        assert!(error.contains("Number Variable"), "{error}");
    }

    #[test]
    fn long_object_files_split_without_hiding_part_suffixes() {
        let mut project = variables(1);
        let mut pool = project.get_pool().clone();
        let mut mac = crate::default_object(ObjectType::Macro);
        mac.mut_id().set_value(10).unwrap();
        if let Object::Macro(mac) = &mut mac {
            let instruction = crate::macro_authoring::encode_macro_instruction(
                "change-numeric-value",
                &["1".into(), "42".into()],
                project.vt_version(),
            )
            .unwrap();
            mac.commands = instruction.repeat(300);
        }
        pool.add(mac);
        project = EditorProject::from(pool);
        let name = "M".repeat(64);
        project.rename_object(
            ag_iso_stack::object_pool::ObjectId::new(10).unwrap(),
            name.clone(),
        );
        let archive = generate(&project).unwrap();
        for part in 1..=2 {
            let path = format!("shared/{name}_part_{part}.aitd");
            let text = std::str::from_utf8(&archive.files[&path]).unwrap();
            let actions = crate::cli::commands::parse_commands(
                text,
                Path::new("shared"),
                &|_| unreachable!(),
            )
            .unwrap();
            assert!(actions.len() <= crate::cli::editing::MAX_EDIT_ACTIONS);
        }
    }

    #[test]
    fn independent_pending_links_resolve_at_their_own_target() {
        let tokens = |items: &[&str]| {
            items
                .iter()
                .map(|item| item.to_string())
                .collect::<Tokens>()
        };
        let mut workflow = Workflow::default();
        workflow.named_targets =
            BTreeMap::from([("a".into(), 1), ("b".into(), 2), ("c".into(), 3)]);
        // Every object is in a cycle. A's independent links must resolve at C
        // and B respectively, even though the B link was emitted first.
        for (id, deps, commands) in [
            (
                1,
                vec![2, 3],
                vec![
                    (30, tokens(&["reference", "set", "A", "first", "B"])),
                    (30, tokens(&["reference", "set", "A", "second", "C"])),
                ],
            ),
            (2, vec![1, 3, 4], vec![]),
            (3, vec![1, 2, 4], vec![]),
            (4, vec![1, 2, 3], vec![]),
        ] {
            workflow.objects.insert(
                id,
                ObjectCommands {
                    directory: "shared".into(),
                    stem: id.to_string(),
                    dependencies: deps.into_iter().collect(),
                    commands,
                },
            );
        }
        // Pick A to break the initial cycle, then C, then B.
        workflow.objects.get_mut(&2).unwrap().dependencies.insert(5);
        workflow.order_objects().unwrap();
        let resolved: Vec<_> = workflow
            .object_batches
            .iter()
            .flat_map(|(target, _, _, commands)| {
                commands
                    .iter()
                    .filter(|command| command[0] == "reference")
                    .map(move |command| (*target, command[3].as_str()))
            })
            .collect();
        assert_eq!(resolved, [(3, "second"), (2, "first")]);
    }

    #[test]
    fn deferred_picture_paths_follow_the_target_directory() {
        let mut workflow = Workflow::default();
        let command = [
            "graphic",
            "assign-file",
            "1",
            "../../assets/graphics/Picture.png",
            "--format",
            "eight_bit",
            "--encoding",
            "raw",
        ]
        .map(str::to_owned)
        .to_vec();
        workflow.objects.insert(
            1,
            ObjectCommands {
                directory: "masks/Main".into(),
                stem: "Picture".into(),
                dependencies: BTreeSet::from([2]),
                commands: vec![(50, command)],
            },
        );
        workflow.objects.insert(
            2,
            ObjectCommands {
                directory: "shared".into(),
                stem: "Palette".into(),
                dependencies: BTreeSet::from([1]),
                commands: vec![],
            },
        );
        workflow.order_objects().unwrap();
        let commands = &workflow
            .object_batches
            .iter()
            .find(|(id, _, _, _)| *id == 2)
            .unwrap()
            .3;
        let command = commands
            .iter()
            .find(|tokens| tokens[0] == "graphic")
            .unwrap();
        assert_eq!(command[3], "../assets/graphics/Picture.png");
        assert_eq!(command[4], "--format");
    }

    #[test]
    fn unresolved_targets_are_reported_and_label_fonts_are_not_targets() {
        let mut workflow = Workflow::default();
        workflow.named_targets = BTreeMap::from([("object-1".into(), 1), ("object-2".into(), 2)]);
        workflow.objects.insert(
            1,
            ObjectCommands {
                directory: "shared".into(),
                stem: "1".into(),
                dependencies: BTreeSet::from([2]),
                commands: vec![(
                    30,
                    vec![
                        "reference".into(),
                        "set".into(),
                        "Object_1".into(),
                        "value".into(),
                        "Object_2".into(),
                    ],
                )],
            },
        );
        let error = workflow.order_objects().unwrap_err();
        assert!(error.contains("Object 1") && error.contains('2'), "{error}");

        let project = variables(1);
        let mut pool = project.get_pool().clone();
        let mut labels = crate::default_object(ObjectType::ObjectLabelReferenceList);
        labels.mut_id().set_value(2).unwrap();
        if let Object::ObjectLabelReferenceList(labels) = &mut labels {
            labels
                .object_labels
                .push(ag_iso_stack::object_pool::object_attributes::ObjectLabel {
                    id: ag_iso_stack::object_pool::ObjectId::new(1).unwrap(),
                    string_variable_reference: ag_iso_stack::object_pool::NullableObjectId(None),
                    graphic_representation: ag_iso_stack::object_pool::NullableObjectId(None),
                    font_type: 0,
                });
        }
        pool.add(labels);
        generate(&EditorProject::from(pool)).unwrap();
    }

    #[test]
    fn large_exports_keep_objects_together_and_leave_source_untouched() {
        let project = variables(300);
        let before = project.save_project().unwrap();
        let revision = project.revision();
        let archive = generate(&project).unwrap();
        assert_eq!(before, project.save_project().unwrap());
        assert_eq!(revision, project.revision());
        let creation: Vec<_> = archive
            .files
            .keys()
            .filter(|name| {
                name.starts_with("shared/")
                    && std::str::from_utf8(&archive.files[*name])
                        .unwrap()
                        .lines()
                        .any(|line| line.contains("object create"))
            })
            .collect();
        assert_eq!(creation.len(), 300);
        assert_eq!(
            archive.to_zip().unwrap(),
            generate(&project).unwrap().to_zip().unwrap()
        );
    }

    #[test]
    fn incremental_job_produces_the_same_zip_and_monotonic_progress() {
        let project = variables(8);
        let expected = generate(&project).unwrap().to_zip().unwrap();
        let mut job = ZipExportJob::new(project);
        let mut previous = 0.0;
        let mut stages = Vec::new();
        let mut actual = None;
        for _ in 0..10_000 {
            let progress = job.progress();
            assert!(progress.fraction >= previous);
            assert!(progress.fraction < 1.0);
            if stages.last() != Some(&progress.status) {
                stages.push(progress.status);
            }
            previous = progress.fraction;
            if let Some(bytes) = job.advance().unwrap() {
                assert_eq!(job.progress().fraction, 1.0);
                actual = Some(bytes);
                break;
            }
        }
        assert_eq!(actual.unwrap(), expected);
        assert_eq!(
            stages,
            [
                "Preparing project…",
                "Generating commands and assets…",
                "Checking generated commands…",
                "Verifying rebuilt project…",
                "Creating ZIP archive…",
            ]
        );
        assert!(job.advance().is_err());
    }

    #[test]
    fn cancelling_at_each_stage_preserves_the_source_and_allows_retry() {
        let project = variables(8);
        let before = project.save_project().unwrap();
        let revision = project.revision();
        let dirty = project.is_dirty();
        for stage in [
            "Preparing project…",
            "Generating commands and assets…",
            "Checking generated commands…",
            "Verifying rebuilt project…",
            "Creating ZIP archive…",
        ] {
            let mut job = ZipExportJob::new(project.clone());
            let mut reached = false;
            for _ in 0..10_000 {
                if job.progress().status == stage {
                    reached = true;
                    break;
                }
                assert!(job.advance().unwrap().is_none());
            }
            assert!(reached, "did not reach {stage}");
            assert!(job.advance().unwrap().is_none());
            drop(job);
            assert_eq!(project.save_project().unwrap(), before);
            assert_eq!(project.revision(), revision);
            assert_eq!(project.is_dirty(), dirty);
        }
        let mut retry = ZipExportJob::new(project.clone());
        let bytes = loop {
            if let Some(bytes) = retry.advance().unwrap() {
                break bytes;
            }
        };
        assert_eq!(bytes, generate(&project).unwrap().to_zip().unwrap());
    }

    #[test]
    fn invalid_drafts_and_opaque_binary_properties_are_rejected() {
        let mut pool = ObjectPool::default();
        let mut picture = crate::default_object(ObjectType::PictureGraphic);
        picture.mut_id().set_value(7).unwrap();
        pool.add(picture);
        assert!(generate(&EditorProject::from(pool)).is_err());

        let mut pool = ObjectPool::default();
        let mut graphic = crate::default_object(ObjectType::GraphicData);
        graphic.mut_id().set_value(9).unwrap();
        pool.add(graphic);
        let error = generate(&EditorProject::from(pool)).err().unwrap();
        assert!(error.contains("graphic-data"), "{error}");
    }

    #[test]
    fn pictures_macros_and_encoded_strings_have_readable_representations_including_definitions() {
        use crate::components::*;
        use ag_iso_stack::object_pool::{
            object::VtString,
            object_attributes::{DataCodeType, PictureGraphicFormat},
        };
        let mut project = variables(1);
        let mut pool = project.get_pool().clone();
        let mut palette = crate::default_object(ObjectType::ColourPalette);
        palette.mut_id().set_value(40).unwrap();
        if let Object::ColourPalette(palette) = &mut palette {
            palette.colours = (0..=255)
                .map(|id| {
                    let mut colour = ag_iso_stack::object_pool::Colour::RED;
                    colour.id = id;
                    colour
                })
                .collect();
        }
        pool.add(palette);
        let mut controls = crate::default_object(ObjectType::WorkingSetSpecialControls);
        controls.mut_id().set_value(41).unwrap();
        if let Object::WorkingSetSpecialControls(controls) = &mut controls {
            controls.id_of_colour_palette = ag_iso_stack::object_pool::NullableObjectId(
                ag_iso_stack::object_pool::ObjectId::new(40).ok(),
            );
        }
        pool.add(controls);
        let mut picture = crate::default_object(ObjectType::PictureGraphic);
        picture.mut_id().set_value(20).unwrap();
        if let Object::PictureGraphic(picture) = &mut picture {
            picture.width = 2;
            picture.actual_width = 2;
            picture.actual_height = 1;
            picture.format = PictureGraphicFormat::EightBit;
            picture.options.data_code_type = DataCodeType::Raw;
            picture.data = vec![1, 2];
        }
        pool.add(picture.clone());
        let mut mac = crate::default_object(ObjectType::Macro);
        mac.mut_id().set_value(10).unwrap();
        if let Object::Macro(mac) = &mut mac {
            mac.commands = crate::macro_authoring::encode_macro_instruction(
                "change-numeric-value",
                &["1".into(), "42".into()],
                project.vt_version(),
            )
            .unwrap();
        }
        pool.add(mac.clone());
        let mut text = crate::default_object(ObjectType::StringVariable);
        text.mut_id().set_value(30).unwrap();
        if let Object::StringVariable(text) = &mut text {
            text.value = VtString::Encoded {
                encoded_bytes: vec![233, 10, 39, 0],
            };
        }
        pool.add(text);
        project = EditorProject::from(pool);
        for object in [picture, mac] {
            let id = ComponentId::default();
            let node_id = ComponentNodeId::default();
            let properties = get_property_descriptors(object.object_type())
                .into_iter()
                .map(|descriptor| {
                    (
                        descriptor.name,
                        get_object_property(&object, descriptor.name).unwrap(),
                    )
                })
                .collect();
            project
                .edit_components("test definition", |_, source| {
                    source.definitions.insert(
                        id,
                        ComponentDefinition {
                            schema_version: 1,
                            id,
                            name: format!("{:?} definition", object.object_type()),
                            description: String::new(),
                            category_path: Vec::new(),
                            tags: Vec::new(),
                            revision: 99,
                            root_node: node_id,
                            nodes: vec![ComponentNode {
                                id: node_id,
                                name: "Root".into(),
                                object_type: object.object_type(),
                                properties,
                                children: Vec::new(),
                                references: BTreeMap::new(),
                                macro_references: Vec::new(),
                                object_list: None,
                            }],
                            parameters: Vec::new(),
                        },
                    );
                    Ok(())
                })
                .unwrap();
        }
        let archive = generate(&project).unwrap();
        let commands = archive
            .files
            .iter()
            .filter(|(name, _)| name.ends_with(".aitd"))
            .map(|(_, bytes)| std::str::from_utf8(bytes).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(commands.contains("node-assign-file"));
        assert!(commands.contains("node-macro-append"));
        assert!(commands.contains("change-numeric-value"));
        assert!(commands.contains("text-set"));
        assert!(!commands.contains("encoded_bytes"));
        assert!(!commands.contains("bytes_base64"));
    }

    #[test]
    fn cleared_metadata_survives_cli_serialization_and_reopening() {
        let mut project = variables(1);
        assert!(project.rename_object(
            ag_iso_stack::object_pool::ObjectId::new(1).unwrap(),
            "Temporary Name".into()
        ));
        for command in [
            "object metadata 'Temporary Name' --name '-Name' --description 'Details' --notes 'Notes' --tag field",
            "object metadata '-Name' --clear-name --clear-description --clear-notes --clear-tags",
        ] {
            let actions =
                crate::cli::commands::parse_commands(command, Path::new("."), &|_| unreachable!())
                    .unwrap();
            crate::cli::editing::execute_batch(
                &mut project,
                serde_json::json!({"actions":actions}),
            )
            .unwrap();
            project = EditorProject::load_project(project.save_project().unwrap()).unwrap();
        }
        let info = project.get_object_info(&project.get_pool().objects()[0]);
        assert_eq!(info.name, None);
        assert_eq!(info.description, None);
        assert_eq!(info.notes, None);
        assert!(info.tags.is_empty());
        let archive = generate(&project).unwrap();
        assert!(archive.files.contains_key("shared/Number_Variable.aitd"));
    }

    #[test]
    fn macro_reference_cycles_are_linked_after_creation() {
        use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef};
        let mut pool = variables(1).get_pool().clone();
        let mut mask = crate::default_object(ObjectType::DataMask);
        mask.mut_id().set_value(1000).unwrap();
        if let Object::DataMask(mask) = &mut mask {
            mask.macro_refs.push(MacroRef {
                macro_id: 10,
                event_id: Event::OnShow,
            });
        }
        pool.add(mask);
        let mut mac = crate::default_object(ObjectType::Macro);
        mac.mut_id().set_value(10).unwrap();
        if let Object::Macro(mac) = &mut mac {
            mac.commands = crate::macro_authoring::encode_macro_instruction(
                "change-background-colour",
                &["1000".into(), "3".into()],
                ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
            )
            .unwrap();
            let mut prefix = crate::macro_authoring::encode_macro_instruction(
                "change-numeric-value",
                &["1".into(), "42".into()],
                ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
            )
            .unwrap();
            prefix.extend_from_slice(&mac.commands);
            mac.commands = prefix;
        }
        pool.add(mac);
        let archive = generate(&EditorProject::from(pool)).unwrap();
        let source = archive
            .files
            .iter()
            .find(|(path, bytes)| {
                path.ends_with(".aitd")
                    && std::str::from_utf8(bytes)
                        .unwrap()
                        .contains("change-numeric-value")
                    && !std::str::from_utf8(bytes)
                        .unwrap()
                        .contains("change-background-colour")
            })
            .unwrap();
        let target = archive
            .files
            .iter()
            .find(|(path, bytes)| {
                path.ends_with(".aitd")
                    && std::str::from_utf8(bytes)
                        .unwrap()
                        .contains("change-background-colour")
            })
            .unwrap();
        let source_text = std::str::from_utf8(source.1).unwrap();
        let target_text = std::str::from_utf8(target.1).unwrap();
        assert!(source_text.contains("change-numeric-value"));
        assert!(!source_text.contains("change-background-colour"));
        assert!(target_text.contains("change-background-colour"));
        assert!(target_text.contains("Resolve pending command from object macro"));
        assert!(
            target_text.find("object create").unwrap() < target_text.find("macro append").unwrap()
        );
        let script = std::str::from_utf8(&archive.files["build.sh"]).unwrap();
        assert!(script.find(source.0).unwrap() < script.find(target.0).unwrap());
    }
}
