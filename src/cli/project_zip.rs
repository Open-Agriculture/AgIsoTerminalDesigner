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
    io::{Cursor, Write},
    path::{Component, Path},
};

type Tokens = Vec<String>;

/// A deterministic archive's uncompressed entries, also useful for inspecting
/// or testing the exact workflow before ZIP serialization.
pub struct CliProjectArchive {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl CliProjectArchive {
    /// Serialize entries with fixed timestamps, stable order and no compression.
    pub fn to_zip(&self) -> Result<Vec<u8>, String> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        for (path, bytes) in &self.files {
            writer
                .start_file(path, options)
                .map_err(|e| e.to_string())?;
            writer.write_all(bytes).map_err(|e| e.to_string())?;
        }
        Ok(writer.finish().map_err(|e| e.to_string())?.into_inner())
    }
}

#[derive(Default)]
struct Workflow {
    files: BTreeMap<String, Vec<u8>>,
    phases: BTreeMap<(u8, usize, String), Vec<Tokens>>,
}

impl Workflow {
    fn command(
        &mut self,
        phase: u8,
        directory: &str,
        tokens: impl IntoIterator<Item = impl ToString>,
    ) {
        self.phases
            .entry((phase, 0, directory.to_owned()))
            .or_default()
            .push(tokens.into_iter().map(|s| s.to_string()).collect());
    }

    fn finish(mut self, source: &EditorProject) -> Result<CliProjectArchive, String> {
        let mut candidate = EditorProject::new_pool();
        let mut script = String::from(BUILD_START);
        for ((phase, sequence, directory), commands) in &self.phases {
            // Every generated command is one edit; parsing checks that invariant
            // and the normal CLI's action limit before executing each batch.
            for (part, chunk) in commands
                .chunks(crate::cli::editing::MAX_EDIT_ACTIONS)
                .enumerate()
            {
                let path = format!("{directory}/{phase:02}_{sequence:04}_{part:04}.aitd");
                let text = chunk
                    .iter()
                    .map(|tokens| crate::cli::commands::serialize_command(tokens))
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n";
                let actions =
                    crate::cli::commands::parse_commands(&text, Path::new(directory), &|path| {
                        self.files
                            .get(&normalize(path)?)
                            .cloned()
                            .ok_or_else(|| format!("Missing asset {}", path.display()))
                    })
                    .map_err(|error| format!("{path}: {error}"))?;
                crate::cli::editing::execute_batch(
                    &mut candidate,
                    serde_json::json!({"actions": actions}),
                )
                .map_err(|error| format!("{path}: {error}"))?;
                // Native CLI invocations persist and reopen between files.
                // Include that boundary in preflight, not just in-memory edits.
                candidate = EditorProject::load_project(
                    candidate.save_project().map_err(|e| e.to_string())?,
                )
                .map_err(|error| format!("{path}: cannot reopen rebuilt project: {error}"))?;
                script.push_str(&format!(
                    "\"$AITD\" run --file {}\n",
                    crate::cli::commands::quote_token(&path)
                ));
                self.files.insert(path, text.into_bytes());
            }
        }
        compare_projects(source, &candidate)?;
        script.push_str(BUILD_END);
        self.files.insert("build.sh".into(), script.into_bytes());
        self.files
            .insert("README.txt".into(), README.as_bytes().to_vec());
        Ok(CliProjectArchive { files: self.files })
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

/// Generate and execute a readable workflow against a stable copy. Unsupported
/// content, invalid projects and any persisted or IOP mismatch fail before save.
pub fn generate(project: &EditorProject) -> Result<CliProjectArchive, String> {
    let project = project.clone();
    validate(&project)?;
    let mut workflow = Workflow::default();
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
    let directories = object_directories(&project);
    for object in project.get_pool().objects() {
        let id = object.id().value().to_string();
        let dir = &directories[&object.id().value()];
        let object_type =
            scalar(&serde_json::to_value(object.object_type()).map_err(|e| e.to_string())?)?;
        let info = project.get_object_info(object);
        workflow.command(
            10,
            dir,
            vec![
                "object".into(),
                "create".into(),
                object_type,
                "--name".into(),
                info.name.clone().unwrap_or_else(|| format!("Object_{id}")),
                "--id".into(),
                id.clone(),
            ],
        );
        for descriptor in get_property_descriptors(object.object_type()) {
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
                continue;
            }
            if matches!(object, Object::Macro(_)) && key == PropertyKey::Commands {
                continue;
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
                continue;
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
                        scalar(&value)?,
                    ],
                );
            } else {
                for tokens in property_commands(&id, key, &value)
                    .map_err(|e| format!("Object {id}, {key}: {e}"))?
                {
                    workflow.command(20, dir, tokens);
                }
            }
        }
        emit_object_extras(&mut workflow, &project, object, dir)?;
    }
    emit_components_and_graphics(&mut workflow, &project)?;
    // ID ranges are applied together: intermediate individual changes may overlap.
    emit_id_ranges(&mut workflow, &project)?;
    let mut order = vec!["object".into(), "order".into()];
    order.extend(
        project
            .get_pool()
            .objects()
            .iter()
            .map(|o| o.id().value().to_string()),
    );
    if order.len() > 2 {
        workflow.command(90, "shared", order);
    }
    workflow.finish(&project)
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
    let mask_dirs: BTreeMap<_, _> = masks
        .iter()
        .map(|mask| {
            let name = project.get_object_info(mask).get_name(mask);
            let slug: String = name
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
            (
                mask.id().value(),
                format!("masks/{}_{}", mask.id().value(), slug),
            )
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
    let id = object.id().value().to_string();
    if let Some(children) = object_refs(object) {
        for child in children {
            workflow.command(
                40,
                dir,
                vec![
                    "placement".into(),
                    "add".into(),
                    id.clone(),
                    child.id.value().to_string(),
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
                .map(|id| id.value().to_string())
                .collect::<Vec<_>>(),
            ObjectReferenceList::Nullable(items) => items
                .into_iter()
                .map(|id| {
                    id.0.map(|id| id.value().to_string())
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
                    binding.macro_id
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
                .map(|id| id.value().to_string()),
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
                    format!("id={}", label.id.value()),
                    format!(
                        "string={}",
                        label
                            .string_variable_reference
                            .0
                            .map(|id| id.value().to_string())
                            .unwrap_or_else(|| "null".into())
                    ),
                    format!(
                        "graphic={}",
                        label
                            .graphic_representation
                            .0
                            .map(|id| id.value().to_string())
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
    let id = object.id().value().to_string();
    workflow.command(
        80,
        dir,
        crate::cli::commands::object_metadata_tokens(
            object.id().value(),
            &project.get_object_info(object),
        ),
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
            let path = format!("assets/graphics/picture_{id}.png");
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
                    id,
                    relative,
                    "--format".into(),
                    format.into(),
                    "--encoding".into(),
                    encoding.into(),
                ],
            );
        }
        Object::Macro(mac) => {
            for instruction in
                crate::macro_authoring::decode_macro_commands(&mac.commands, project.vt_version())
                    .map_err(|e| format!("Object {id}, commands: {e}"))?
            {
                workflow.command(
                    50,
                    dir,
                    crate::cli::commands::macro_instruction_tokens(
                        object.id().value(),
                        &instruction,
                    ),
                );
            }
        }
        _ => (),
    }
    Ok(())
}

fn emit_id_ranges(workflow: &mut Workflow, project: &EditorProject) -> Result<(), String> {
    let mut tokens = vec!["project".into(), "settings".into(), "set".into()];
    for (ty, range) in project.object_id_ranges().iter() {
        tokens.push(format!(
            "id_range.{}={}:{}",
            scalar(&serde_json::to_value(ty).map_err(|e| e.to_string())?)?,
            range.start,
            range.end
        ));
    }
    workflow.command(85, "shared", tokens);
    Ok(())
}

fn emit_components_and_graphics(
    workflow: &mut Workflow,
    project: &EditorProject,
) -> Result<(), String> {
    for asset in project.ai_design_state().graphics.assets.values() {
        let png = asset
            .as_png()
            .map_err(|e| format!("Graphic {}, original: {e}", asset.id))?
            .original_bytes;
        let path = format!("assets/graphics/asset_{}.png", asset.id);
        workflow.files.insert(path.clone(), png);
        let mut tokens = vec![
            "graphic".into(),
            "import".into(),
            format!("../{path}"),
            "--id".into(),
            asset.id.to_string(),
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
    }
    emit_components(workflow, project)
}

fn emit_components(workflow: &mut Workflow, project: &EditorProject) -> Result<(), String> {
    use crate::cli::component_authoring::{
        export_actions, export_command_tokens, ComponentAuthoringAction as A,
    };
    let mut sequence = 0;
    let mut content: BTreeMap<String, Vec<Tokens>> = BTreeMap::new();
    for action in export_actions(project.component_source())? {
        let (phase, dir) = match &action {
            A::InstanceCreate { parent, .. } => (
                70,
                object_directories(project)
                    .get(parent)
                    .cloned()
                    .unwrap_or_else(|| "shared".into()),
            ),
            A::DefinitionCreate { id, .. } => (60, format!("components/{id}")),
            _ => {
                // All other exported definition actions carry component_id.
                let value = serde_json::to_value(&action).map_err(|e| e.to_string())?;
                let id = value["component_id"]
                    .as_str()
                    .ok_or("Missing component identity in export action")?;
                (60, format!("components/{id}"))
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
                        component_id.to_string(),
                        node_id.to_string(),
                        property.to_string(),
                        text,
                        "--encoding".into(),
                        encoding,
                    ],
                );
                continue;
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
                    let path = format!("assets/graphics/node_{component_id}_{node_id}.png");
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
                    content.entry(dir.clone()).or_default().push(vec![
                        "component".into(),
                        "node-assign-file".into(),
                        component_id.to_string(),
                        node_id.to_string(),
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
                    content.entry(dir.clone()).or_default().push(vec![
                        "component".into(),
                        "node-macro-clear".into(),
                        component_id.to_string(),
                        node_id.to_string(),
                    ]);
                    for instruction in instructions {
                        let mut tokens = vec![
                            "component".into(),
                            "node-macro-append".into(),
                            component_id.to_string(),
                            node_id.to_string(),
                            instruction.name,
                        ];
                        tokens.extend(instruction.arguments);
                        content.entry(dir.clone()).or_default().push(tokens);
                    }
                } else {
                    return Err(format!(
                        "{context}: binary content has no supported readable representation"
                    ));
                }
                continue;
            }
        }
        if matches!(action, A::DefinitionMetadata { .. }) {
            for tokens in content.remove(&dir).unwrap_or_default() {
                workflow.command(phase, &dir, tokens);
            }
        }
        if phase == 70 {
            sequence += 1;
            workflow
                .phases
                .entry((phase, sequence, dir))
                .or_default()
                .push(export_command_tokens(&action)?);
        } else {
            workflow.command(phase, &dir, export_command_tokens(&action)?);
        }
    }
    Ok(())
}

fn persisted_content(project: &EditorProject) -> Result<Value, String> {
    let mut value: Value =
        serde_json::from_slice(&project.save_project().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    value["settings"]
        .as_object_mut()
        .unwrap()
        .remove("last_selected");
    // An absent metadata record is equivalent to a persisted default record.
    if let Some(metadata) = value["object_metadata"].as_object_mut() {
        metadata.retain(|_, record| {
            record.as_object().is_some_and(|fields| {
                fields.values().any(|v| match v {
                    Value::Null => false,
                    Value::Bool(false) => false,
                    Value::Array(items) if items.is_empty() => false,
                    _ => true,
                })
            })
        });
    }
    // The only intentional persistence difference is original graphic encoding.
    if let Some(assets) = value
        .pointer_mut("/graphics/assets")
        .and_then(Value::as_object_mut)
    {
        for asset in assets.values_mut() {
            let original: Vec<u8> = serde_json::from_value(asset["original_bytes"].clone())
                .map_err(|e| e.to_string())?;
            let decoded = image::load_from_memory(&original)
                .map_err(|e| e.to_string())?
                .to_rgba16();
            asset["original_bytes"] = serde_json::json!({"width": decoded.width(), "height": decoded.height(), "rgba": decoded.as_raw()});
            asset["format"] = Value::String("png".into());
        }
    }
    Ok(value)
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

fn compare_projects(source: &EditorProject, rebuilt: &EditorProject) -> Result<(), String> {
    validate(rebuilt)?;
    if let Some(path) = first_difference(
        &persisted_content(source)?,
        &persisted_content(rebuilt)?,
        "project",
    ) {
        return Err(format!("CLI reconstruction differs at {path}; this authoring content has no supported readable representation"));
    }
    if source.export_iop().map_err(|e| e.to_string())?
        != rebuilt.export_iop().map_err(|e| e.to_string())?
    {
        return Err("CLI reconstruction produced different IOP bytes".into());
    }
    Ok(())
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

const README: &str = "CLI authoring project\n\nPrerequisites: Bash and an installed aitd matching the exporting editor.\nExtract the ZIP, then run: bash build.sh\nFrom elsewhere: bash /path/to/extracted/build.sh\nUse AITD=/path/to/aitd bash build.sh to select a CLI.\n\nThe script owns a temporary CLI session, stops on the first error, and validates\nbefore publishing build/project.aitp and build/project.iop. Existing build/\noutputs are never overwritten. Move or remove build/ before rebuilding.\n\nEdit the readable .aitd command files or PNG assets, then rebuild. Commands use\nthe ordinary CLI authoring language; aitd --help describes available commands.\nshared/ contains working sets, shared resources, and unused objects. masks/\ncontains each data or alarm mask's exclusive objects. components/ holds reusable\ndefinitions. Numeric phase prefixes determine global execution order in build.sh:\nsettings, creation, properties, references, placements, instances, metadata.\nCommand files hold at most 256 actions. Text may span quoted lines.\nassets/graphics/ holds indexed picture PNGs and converted available originals.\nOriginal graphic file encoding is replaced with PNG; decoded pixels are retained.\n";

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
    fn large_exports_split_batches_and_leave_source_untouched() {
        let project = variables(300);
        let before = project.save_project().unwrap();
        let revision = project.revision();
        let archive = generate(&project).unwrap();
        assert_eq!(before, project.save_project().unwrap());
        assert_eq!(revision, project.revision());
        let creation: Vec<_> = archive
            .files
            .keys()
            .filter(|name| name.starts_with("shared/10_"))
            .collect();
        assert_eq!(creation.len(), 2);
        assert_eq!(
            archive.to_zip().unwrap(),
            generate(&project).unwrap().to_zip().unwrap()
        );
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
        assert!(error.contains("9"), "{error}");
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
                            name: "Binary content".into(),
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
        for command in [
            "object metadata 1 --name '-Name' --description 'Details' --notes 'Notes' --tag field",
            "object metadata 1 --clear-name --clear-description --clear-notes --clear-tags",
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
        generate(&project).unwrap();
    }

    #[test]
    fn macro_reference_cycles_are_linked_after_creation() {
        use ag_iso_stack::object_pool::object_attributes::{Event, MacroRef};
        let mut pool = ObjectPool::default();
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
        }
        pool.add(mac);
        let archive = generate(&EditorProject::from(pool)).unwrap();
        assert!(archive
            .files
            .keys()
            .any(|path| path.starts_with("masks/1000_") && path.ends_with(".aitd")));
    }
}
