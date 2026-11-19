//! Cooperative verification of the reconstructed project.
use super::{first_difference, validate};
use crate::EditorProject;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Comparison {
    phase: Phase,
    normalizing: Option<NormalizedContent>,
    source_content: Option<Value>,
    rebuilt_content: Option<Value>,
    source_iop: Option<Vec<u8>>,
    rebuilt_normalized: Option<EditorProject>,
    uuid_mapping: BTreeMap<String, String>,
}

#[derive(Default)]
enum Phase {
    #[default]
    Validate,
    SourceContent,
    RebuiltContent,
    CompareContent,
    SourceIop,
    CompareIop,
    Done,
}

impl Comparison {
    pub(super) fn fraction(&self) -> f32 {
        let normalized = self.normalizing.as_ref().map_or(0.0, |content| {
            content.next as f32 / content.asset_ids.len().max(1) as f32
        });
        match self.phase {
            Phase::Validate => 0.0,
            Phase::SourceContent => 0.1 + 0.3 * normalized,
            Phase::RebuiltContent => 0.4 + 0.3 * normalized,
            Phase::CompareContent => 0.7,
            Phase::SourceIop => 0.8,
            Phase::CompareIop => 0.9,
            Phase::Done => 1.0,
        }
    }

    /// Validate or compare one item. Drop the comparison between calls to cancel.
    pub(super) fn advance(
        &mut self,
        source: &EditorProject,
        rebuilt: &EditorProject,
    ) -> Result<bool, String> {
        match self.phase {
            Phase::Validate => {
                validate(rebuilt)?;
                let (normalized, uuid_mapping) = normalize_rebuilt_identities(source, rebuilt)?;
                self.rebuilt_normalized = Some(normalized);
                self.uuid_mapping = uuid_mapping;
                self.phase = Phase::SourceContent;
            }
            Phase::SourceContent | Phase::RebuiltContent => {
                if let Some(content) = &mut self.normalizing {
                    if content.advance()? {
                        let value = self.normalizing.take().unwrap().value;
                        if matches!(self.phase, Phase::SourceContent) {
                            self.source_content = Some(value);
                            self.phase = Phase::RebuiltContent;
                        } else {
                            self.rebuilt_content = Some(remap_uuids(value, &self.uuid_mapping));
                            self.phase = Phase::CompareContent;
                        }
                    }
                } else {
                    let project = if matches!(self.phase, Phase::SourceContent) {
                        source
                    } else {
                        self.rebuilt_normalized.as_ref().unwrap()
                    };
                    self.normalizing = Some(NormalizedContent::new(project)?);
                }
            }
            Phase::CompareContent => {
                if let Some(path) = first_difference(
                    self.source_content.as_ref().unwrap(),
                    self.rebuilt_content.as_ref().unwrap(),
                    "project",
                ) {
                    return Err(format!(
                        "CLI reconstruction differs at {path}; this authoring content has no supported readable representation"
                    ));
                }
                self.source_content = None;
                self.rebuilt_content = None;
                self.phase = Phase::SourceIop;
            }
            Phase::SourceIop => {
                self.source_iop = Some(source.export_iop().map_err(|e| e.to_string())?);
                self.phase = Phase::CompareIop;
            }
            Phase::CompareIop => {
                let rebuilt_iop = self
                    .rebuilt_normalized
                    .as_ref()
                    .unwrap()
                    .export_iop()
                    .map_err(|e| e.to_string())?;
                let source_iop = self.source_iop.take().unwrap();
                if source_iop != rebuilt_iop {
                    compare_iop_objects(source, &source_iop, &rebuilt_iop)?;
                }
                self.phase = Phase::Done;
            }
            Phase::Done => (),
        }
        Ok(matches!(self.phase, Phase::Done))
    }
}

/// IOP encoders may choose a different but equivalent byte ordering after
/// automatic IDs are reassigned. Compare the decoded protocol objects so the
/// reconstruction still has to produce the same wire-level object graph.
fn compare_iop_objects(
    project: &EditorProject,
    source_iop: &[u8],
    rebuilt_iop: &[u8],
) -> Result<(), String> {
    let decode = |bytes: &[u8]| -> Result<BTreeMap<u16, Value>, String> {
        let pool = ag_iso_stack::object_pool::ObjectPool::try_from_iop_for_version(
            bytes.to_vec(),
            project.vt_version(),
        )
        .map_err(|error| format!("Invalid reconstructed IOP: {error}"))?;
        pool.objects()
            .iter()
            .map(|object| {
                serde_json::to_value(object)
                    .map(|value| (object.id().value(), value))
                    .map_err(|error| error.to_string())
            })
            .collect()
    };
    let source = decode(source_iop)?;
    let rebuilt = decode(rebuilt_iop)?;
    if source != rebuilt {
        let id = source
            .keys()
            .chain(rebuilt.keys())
            .copied()
            .find(|id| source.get(id) != rebuilt.get(id));
        return Err(match id {
            Some(id) => {
                format!("CLI reconstruction produced different IOP object {id} after decoding")
            }
            None => "CLI reconstruction produced different IOP object counts".into(),
        });
    }
    Ok(())
}

/// Reassign rebuilt protocol IDs to their source IDs before comparing content
/// or IOP bytes. Generated component and graphic UUIDs are paired by name.
fn normalize_rebuilt_identities(
    source: &EditorProject,
    rebuilt: &EditorProject,
) -> Result<(EditorProject, BTreeMap<String, String>), String> {
    let mut mappings = Vec::new();
    let mut target_types = Vec::new();
    let source_objects = source.get_pool().objects();
    let rebuilt_objects = rebuilt.get_pool().objects();
    if source_objects.len() != rebuilt_objects.len() {
        return Err("CLI reconstruction changed the number of pool objects".into());
    }
    for (source_object, rebuilt_object) in source_objects.iter().zip(rebuilt_objects) {
        if source_object.object_type() != rebuilt_object.object_type() {
            return Err("CLI reconstruction changed pool object order or type".into());
        }
        let old = rebuilt_object.id().value();
        let new = source_object.id().value();
        if source.get_object_info(source_object).id_overridden && old != new {
            return Err(format!(
                "CLI reconstruction changed fixed ID {new} to automatic ID {old}"
            ));
        }
        if old != new {
            mappings.push((old, new));
        }
        target_types.push((old, rebuilt_object.object_type()));
    }

    let source_components = source.component_source();
    let rebuilt_components = rebuilt.component_source();
    let mut uuid_mapping = BTreeMap::new();
    for source_definition in source_components.definitions.values() {
        let definition_key = crate::smart_naming::normalized_name(&source_definition.name);
        let rebuilt_definition = rebuilt_components
            .definitions
            .values()
            .find(|candidate| {
                crate::smart_naming::normalized_name(&candidate.name) == definition_key
            })
            .ok_or_else(|| format!("Rebuilt component '{}' is missing", source_definition.name))?;
        uuid_mapping.insert(
            rebuilt_definition.id.to_string(),
            source_definition.id.to_string(),
        );

        for source_node in &source_definition.nodes {
            let node_key = crate::smart_naming::normalized_name(&source_node.name);
            let rebuilt_node = rebuilt_definition
                .nodes
                .iter()
                .find(|candidate| crate::smart_naming::normalized_name(&candidate.name) == node_key)
                .ok_or_else(|| format!("Rebuilt node '{}' is missing", source_node.name))?;
            uuid_mapping.insert(rebuilt_node.id.to_string(), source_node.id.to_string());
            for source_instance in source_components
                .instances
                .iter()
                .filter(|instance| instance.component_id == source_definition.id)
            {
                let source_id = *source_instance
                    .compiled_ids
                    .get(&source_node.id)
                    .ok_or_else(|| {
                        format!(
                            "Source compiled ID for node '{}' is missing",
                            source_node.name
                        )
                    })?;
                let rebuilt_instance = rebuilt_components
                    .instances
                    .iter()
                    .find(|candidate| {
                        crate::smart_naming::normalized_name(&candidate.name)
                            == crate::smart_naming::normalized_name(&source_instance.name)
                    })
                    .ok_or_else(|| {
                        format!(
                            "Rebuilt component instance '{}' is missing",
                            source_instance.name
                        )
                    })?;
                let rebuilt_id = *rebuilt_instance
                    .compiled_ids
                    .get(&rebuilt_node.id)
                    .ok_or_else(|| {
                        format!(
                            "Rebuilt compiled ID for node '{}' is missing",
                            source_node.name
                        )
                    })?;
                if rebuilt_id != source_id {
                    mappings.push((rebuilt_id, source_id));
                }
                target_types.push((rebuilt_id, rebuilt_node.object_type));
            }
        }
        for source_parameter in &source_definition.parameters {
            let parameter_key = crate::smart_naming::normalized_name(&source_parameter.name);
            let rebuilt_parameter = rebuilt_definition
                .parameters
                .iter()
                .find(|candidate| {
                    crate::smart_naming::normalized_name(&candidate.name) == parameter_key
                })
                .ok_or_else(|| {
                    format!("Rebuilt parameter '{}' is missing", source_parameter.name)
                })?;
            uuid_mapping.insert(
                rebuilt_parameter.id.to_string(),
                source_parameter.id.to_string(),
            );
        }
    }

    for source_instance in &source_components.instances {
        let rebuilt_instance = rebuilt_components
            .instances
            .iter()
            .find(|candidate| {
                crate::smart_naming::normalized_name(&candidate.name)
                    == crate::smart_naming::normalized_name(&source_instance.name)
            })
            .ok_or_else(|| {
                format!(
                    "Rebuilt component instance '{}' is missing",
                    source_instance.name
                )
            })?;
        uuid_mapping.insert(
            rebuilt_instance.id.to_string(),
            source_instance.id.to_string(),
        );
    }
    for source_asset in source.graphic_library().assets.values() {
        let rebuilt_asset = rebuilt
            .graphic_library()
            .assets
            .values()
            .find(|candidate| {
                crate::smart_naming::normalized_name(&candidate.name)
                    == crate::smart_naming::normalized_name(&source_asset.name)
            })
            .ok_or_else(|| format!("Rebuilt graphic '{}' is missing", source_asset.name))?;
        uuid_mapping.insert(rebuilt_asset.id.to_string(), source_asset.id.to_string());
    }

    mappings.sort_unstable();
    mappings.dedup_by_key(|(old, _)| *old);
    let mut normalized = rebuilt.clone();
    if !mappings.is_empty() {
        normalized.apply_id_reassignment(crate::IdReassignmentPlan {
            rows: Vec::new(),
            mappings,
            target_types,
            includes_overrides: true,
        })?;
    }
    Ok((normalized, uuid_mapping))
}

fn remap_uuids(value: Value, mapping: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(text) => Value::String(mapping.get(&text).cloned().unwrap_or(text)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| remap_uuids(item, mapping))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    let key = mapping.get(&key).cloned().unwrap_or(key);
                    (key, remap_uuids(value, mapping))
                })
                .collect(),
        ),
        other => other,
    }
}

/// Persisted content with irrelevant metadata normalized, one graphic per step.
struct NormalizedContent {
    value: Value,
    asset_ids: Vec<String>,
    next: usize,
}

impl NormalizedContent {
    fn new(project: &EditorProject) -> Result<Self, String> {
        let mut value: Value =
            serde_json::from_slice(&project.save_project().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        normalize_component_object_references(project, &mut value)?;
        value["settings"]
            .as_object_mut()
            .unwrap()
            .remove("last_selected");
        // Missing metadata is equivalent to a persisted default record.
        if let Some(metadata) = value["object_metadata"].as_object_mut() {
            metadata.retain(|_, record| {
                record.as_object().is_some_and(|fields| {
                    fields.values().any(|v| match v {
                        Value::Null | Value::Bool(false) => false,
                        Value::Array(items) if items.is_empty() => false,
                        _ => true,
                    })
                })
            });
        }
        let asset_ids = value
            .pointer("/graphics/assets")
            .and_then(Value::as_object)
            .map(|assets| assets.keys().cloned().collect())
            .unwrap_or_default();
        Ok(Self {
            value,
            asset_ids,
            next: 0,
        })
    }

    fn advance(&mut self) -> Result<bool, String> {
        if let Some(id) = self.asset_ids.get(self.next) {
            let asset = &mut self.value["graphics"]["assets"][id];
            // Original graphic encoding is the only intentional persistence difference.
            let original: Vec<u8> = serde_json::from_value(asset["original_bytes"].clone())
                .map_err(|e| e.to_string())?;
            let decoded = image::load_from_memory(&original)
                .map_err(|e| e.to_string())?
                .to_rgba16();
            asset["original_bytes"] = serde_json::json!({
                "width": decoded.width(), "height": decoded.height(), "rgba": decoded.as_raw()
            });
            asset["format"] = Value::String("png".into());
            self.next += 1;
        }
        Ok(self.next >= self.asset_ids.len())
    }
}

/// Component object-reference values are accepted in persisted projects as
/// numeric IDs or as CLI name targets. Compare them by their resolved object
/// identity so a readable name round-trip remains semantically exact.
fn normalize_component_object_references(
    project: &EditorProject,
    value: &mut Value,
) -> Result<(), String> {
    let Some(components) = value.get_mut("components") else {
        return Ok(());
    };
    let mut source: crate::components::ComponentSource =
        serde_json::from_value(components.clone()).map_err(|e| e.to_string())?;
    let normalize_value = |value: &mut Value| -> Result<(), String> {
        if value.is_null() {
            return Ok(());
        }
        let id = if let Some(id) = value.as_u64() {
            u16::try_from(id).map_err(|_| format!("Object reference ID {id} is out of range"))?
        } else if let Some(name) = value.as_str() {
            crate::cli::names::object_id(project, name)?
        } else {
            let target: crate::operations::ObjectTarget =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            match target {
                crate::operations::ObjectTarget::Id { id } => id,
                crate::operations::ObjectTarget::Name { name } => {
                    crate::cli::names::object_id(project, &name)?
                }
            }
        };
        *value = serde_json::json!(id);
        Ok(())
    };
    for definition in source.definitions.values_mut() {
        for parameter in &mut definition.parameters {
            if parameter.kind == crate::components::ComponentParameterKind::ObjectReference {
                normalize_value(&mut parameter.default_value)?;
            }
        }
    }
    for instance in &mut source.instances {
        let Some(definition) = source.definitions.get(&instance.component_id) else {
            continue;
        };
        for (parameter_id, override_value) in &mut instance.overrides {
            if definition.parameters.iter().any(|parameter| {
                parameter.id == *parameter_id
                    && parameter.kind == crate::components::ComponentParameterKind::ObjectReference
            }) {
                normalize_value(override_value)?;
            }
        }
    }
    *components = serde_json::to_value(source).map_err(|e| e.to_string())?;
    Ok(())
}
