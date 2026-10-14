//! Editor-owned candidate projects for atomic CLI authoring.

use super::{snapshot::ObjectSummary, AiDesignState, AiProposal, AiProposalSummary};
use crate::{
    components::{ComponentIntent, ComponentService},
    object_properties::{get_object_property, get_property_descriptors, PropertySemantic},
    operations::{
        AppliedTransaction, ObjectTarget, Operation, OperationTransaction, PreparedTransaction,
        CURRENT_OPERATION_SCHEMA_VERSION,
    },
    EditorProject, ObjectIdRanges,
};
use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectPool};
use serde_json::Value;
use std::collections::HashMap;

/// Complete authoring content carried by an uncommitted draft.
#[derive(Debug, Clone)]
pub struct ProjectContent {
    pub pool: ObjectPool,
    pub object_info: HashMap<ag_iso_stack::object_pool::ObjectId, crate::ObjectInfo>,
    pub components: crate::components::ComponentSource,
    pub graphics: crate::graphics::GraphicLibrary,
    pub mask_size: u16,
    pub soft_key_size: (u16, u16),
    pub soft_key_layout: crate::SoftKeyLayout,
    pub vt_version: VtVersion,
    pub object_id_ranges: ObjectIdRanges,
}

/// A mutable candidate bound to the revision from which it was created.
#[derive(Debug, Clone)]
pub struct Draft {
    pub id: String,
    pub generation: u64,
    pub base_revision: u64,
    pub name: Option<String>,
    pub content: ProjectContent,
    pub action_count: usize,
    pub summary: AiProposalSummary,
    pub affected_objects: Vec<ObjectSummary>,
}

impl ProjectContent {
    /// Copy canonical authoring content, excluding UI preferences and history.
    pub fn from_state(state: &AiDesignState) -> Self {
        Self {
            pool: state
                .canonical_pool
                .clone()
                .unwrap_or_else(|| state.pool.clone()),
            object_info: state.object_info.clone(),
            components: state.components.clone(),
            graphics: state.graphics.clone(),
            mask_size: state.mask_size,
            soft_key_size: state.soft_key_size,
            soft_key_layout: state.soft_key_layout,
            vt_version: state.vt_version,
            object_id_ranges: state.object_id_ranges.clone(),
        }
    }

    pub(crate) fn into_editor_project(
        self,
        _object_id_format: crate::ObjectIdFormat,
    ) -> EditorProject {
        let state = AiDesignState {
            revision: 0,
            canonical_pool: Some(self.pool.clone()),
            pool: self.pool,
            object_info: self.object_info,
            components: self.components,
            graphics: self.graphics,
            mask_size: self.mask_size,
            soft_key_size: self.soft_key_size,
            soft_key_layout: self.soft_key_layout,
            vt_version: self.vt_version,
            object_id_ranges: self.object_id_ranges,
            selected: None,
        };
        EditorProject::from_ai_source(&state)
    }

    fn design_state(&self, live: &AiDesignState) -> AiDesignState {
        let mut state = live.clone();
        state.canonical_pool = Some(self.pool.clone());
        state.components = self.components.clone();
        state.graphics = self.graphics.clone();
        state.object_id_ranges = self.object_id_ranges.clone();
        state.mask_size = self.mask_size;
        state.soft_key_size = self.soft_key_size;
        state.soft_key_layout = self.soft_key_layout;
        state.vt_version = self.vt_version;
        state.object_info = self.object_info.clone();
        let project = self
            .clone()
            .into_editor_project(crate::ObjectIdFormat::default());
        state.pool = project
            .materialize()
            .map(|m| m.pool)
            .unwrap_or_else(|_| self.pool.clone());
        state
    }
}

impl Draft {
    /// Materialize a validated candidate for inspection, retaining live selection.
    pub fn design_state(&self, live: &AiDesignState) -> AiDesignState {
        self.content.design_state(live)
    }
}

/// Capture an inspected project revision as an editor-owned, volatile draft.
pub fn begin(state: &AiDesignState, name: Option<String>) -> Draft {
    Draft {
        id: uuid::Uuid::new_v4().to_string(),
        generation: 0,
        base_revision: state.revision,
        name,
        content: ProjectContent::from_state(state),
        action_count: 0,
        summary: AiProposalSummary::default(),
        affected_objects: Vec::new(),
    }
}

/// Apply a batch to a temporary candidate. A failure leaves the caller's draft intact.
pub fn edit(state: &AiDesignState, draft: &Draft, actions: Vec<Value>) -> Result<Draft, String> {
    if draft.base_revision != state.revision {
        return Err("Draft is stale because the project changed".into());
    }
    if actions.is_empty() || draft.action_count + actions.len() > 256 {
        return Err("A draft can contain 1..256 edit actions".into());
    }
    let batch_action_count = actions.len();
    let mut summary = draft.summary.clone();
    summary.operation_count += batch_action_count;
    for action in &actions {
        summarize_action(action, &mut summary);
    }
    let mut candidate = draft
        .content
        .clone()
        .into_editor_project(crate::ObjectIdFormat::default());
    candidate.set_defer_candidate_validation(true);
    let reserved_ids = actions
        .iter()
        .filter_map(|action| {
            if action.get("kind").and_then(Value::as_str) != Some("operation") {
                return None;
            }
            let operation = action.get("operation")?;
            if operation.get("type").and_then(Value::as_str) != Some("CreateObject") {
                return None;
            }
            operation
                .get("data")?
                .get("object_id")?
                .as_u64()
                .and_then(|id| u16::try_from(id).ok())
        })
        .collect();
    candidate.set_ai_draft_reserved_ids(reserved_ids);
    let mut setting_values = serde_json::Map::new();
    for (index, action) in actions.iter().enumerate() {
        if action.get("kind").and_then(Value::as_str) == Some("settings") {
            let values = action
                .get("values")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("action {}: settings action requires values", index + 1))?;
            setting_values.extend(values.clone());
        }
    }
    if !setting_values.is_empty() {
        apply_settings(&mut candidate, &setting_values)
            .map_err(|error| format!("settings: {error}"))?;
    }
    for (index, action) in actions.into_iter().enumerate() {
        let location = action["_line"]
            .as_u64()
            .map(|line| format!("line {line}"))
            .unwrap_or_else(|| format!("action {}", index + 1));
        let result = (|| -> Result<(), String> {
            let kind = action
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("Draft action requires a kind")?;
            match kind {
                "operation" => {
                    let operation = serde_json::from_value::<Operation>(
                        action
                            .get("operation")
                            .cloned()
                            .ok_or("operation action requires operation")?,
                    )
                    .map_err(|e| e.to_string())?;
                    candidate.apply_ai_draft_operation(&operation)?;
                    Ok(())
                }
                "component" => {
                    let intent = serde_json::from_value::<ComponentIntent>(
                        action
                            .get("action")
                            .cloned()
                            .ok_or("component action requires action")?,
                    )
                    .map_err(|e| e.to_string())?;
                    ComponentService::apply_intents(&mut candidate, &[intent])
                        .map_err(|e| e.to_string())?;
                    Ok(())
                }
                "set_properties" => apply_set_properties(&mut candidate, &action),
                "settings" => Ok(()),
                _ => apply_action(&mut candidate, action),
            }
        })();
        result.map_err(|error| format!("{location}: {error}"))?;
    }
    // Materialization is the final-batch integrity boundary. It permits edits
    // that only become valid once all operations have been applied.
    candidate.materialize().map_err(|e| e.to_string())?;
    let candidate_content = ProjectContent::from_state(&candidate.ai_design_state());
    EditorProject::from_ai_source(state)
        .validate_ai_draft_content(candidate_content.clone())
        .map_err(|error| format!("final validation against live base: {error}"))?;
    draft
        .content
        .clone()
        .into_editor_project(crate::ObjectIdFormat::default())
        .validate_ai_draft_content(candidate_content.clone())
        .map_err(|error| format!("final validation: {error}"))?;
    let mut next = draft.clone();
    next.generation += 1;
    next.action_count += batch_action_count;
    next.content = candidate_content;
    let changed = changed_objects(&draft.content, &next.content);
    summary.created += changed
        .iter()
        .filter(|(before, after, _)| before.is_none() && after.is_some())
        .count();
    summary.deleted += changed
        .iter()
        .filter(|(before, after, _)| before.is_some() && after.is_none())
        .count();
    for (_, _, item) in changed {
        if !next
            .affected_objects
            .iter()
            .any(|existing| existing.id == item.id)
        {
            next.affected_objects.push(item);
        }
    }
    next.summary = summary;
    Ok(next)
}

/// Freeze a candidate for GUI review, rejecting stale or newly invalid content.
/// Does not mutate the live project or its undo history.
pub fn submit(state: &AiDesignState, draft: &Draft) -> Result<AiProposal, String> {
    if draft.base_revision != state.revision {
        return Err("Draft is stale because the project changed".into());
    }
    EditorProject::from_ai_source(state).validate_ai_draft_content(draft.content.clone())?;
    let materialized = crate::components::materialize_source_with_graphics(
        &draft.content.pool,
        &draft.content.components,
        draft.content.vt_version,
        &draft.content.object_info,
        &draft.content.graphics,
        &draft.content.object_id_ranges,
    )
    .map_err(|e| e.to_string())?;
    let transaction = OperationTransaction {
        schema_version: CURRENT_OPERATION_SCHEMA_VERSION,
        description: draft.name.clone(),
        operations: Vec::new(),
    };
    let prepared = PreparedTransaction {
        transaction: transaction.clone(),
        pool: materialized.pool,
        object_info: draft.content.object_info.clone(),
        applied: AppliedTransaction {
            transaction_id: uuid::Uuid::new_v4().to_string(),
            description: draft.name.clone(),
            forward_operations: Vec::new(),
            inverse_operations: Vec::new(),
            affected_objects: Vec::new(),
            execution_time_ms: 0,
        },
        diagnostics: Vec::new(),
    };
    let mut summary = draft.summary.clone();
    summary.affected_objects = draft.affected_objects.clone();
    Ok(AiProposal {
        proposal_id: uuid::Uuid::new_v4().to_string(),
        canonical_pool: draft.content.pool.clone(),
        component_intents: Vec::new(),
        components: draft.content.components.clone(),
        graphics: draft.content.graphics.clone(),
        source_map: materialized.source_map,
        base_revision: draft.base_revision,
        transaction,
        prepared,
        summary,
        refinement_round: 0,
        soft_key_layout: draft.content.soft_key_layout,
        candidate_content: Some(draft.content.clone()),
    })
}

fn summarize_action(action: &Value, summary: &mut AiProposalSummary) {
    let kind = action.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind == "operation" {
        let Ok(operation) = action
            .get("operation")
            .cloned()
            .ok_or(())
            .and_then(|v| serde_json::from_value::<Operation>(v).map_err(|_| ()))
        else {
            return;
        };
        match operation {
            Operation::CreateObject { .. } | Operation::DeleteObject { .. } => {}
            Operation::RenameObject { .. } => summary.renamed += 1,
            Operation::SetProperty { .. }
            | Operation::SetObjectReference { .. }
            | Operation::SetObjectPointerAllowedReferences { .. } => summary.property_changes += 1,
            Operation::AddChild { .. }
            | Operation::RemoveChild { .. }
            | Operation::SetChildPosition { .. }
            | Operation::SetChildren { .. }
            | Operation::SetObjectList { .. }
            | Operation::SetMacroReferences { .. }
            | Operation::ReorderObjects { .. }
            | Operation::RemapObjectIds { .. }
            | Operation::ChangeObjectId { .. }
            | Operation::SetObjectIdAssignment { .. } => summary.structural_changes += 1,
            _ => {}
        }
    } else if kind == "set_properties" || kind == "settings" {
        summary.property_changes += action
            .get("values")
            .and_then(Value::as_object)
            .map_or(1, |values| values.len());
    } else if kind.starts_with("graphic_")
        || kind.starts_with("component_")
        || kind == "component"
        || kind.starts_with("id_")
    {
        summary.structural_changes += 1;
    }
}

fn changed_objects(
    before: &ProjectContent,
    after: &ProjectContent,
) -> Vec<(Option<u16>, Option<u16>, ObjectSummary)> {
    let mut ids: std::collections::BTreeSet<u16> = before
        .pool
        .objects()
        .iter()
        .map(|object| object.id().value())
        .collect();
    ids.extend(
        after
            .pool
            .objects()
            .iter()
            .map(|object| object.id().value()),
    );
    ids.into_iter()
        .filter_map(|id| {
            let old = before
                .pool
                .objects()
                .iter()
                .find(|object| object.id().value() == id);
            let new = after
                .pool
                .objects()
                .iter()
                .find(|object| object.id().value() == id);
            let old_info = old.and_then(|object| before.object_info.get(&object.id()));
            let new_info = new.and_then(|object| after.object_info.get(&object.id()));
            let metadata_changed = match (old_info, new_info) {
                (Some(a), Some(b)) => !a.content_eq(b),
                (None, None) => false,
                _ => true,
            };
            let object_changed = match (old, new) {
                (Some(a), Some(b)) => serde_json::to_value(a).ok() != serde_json::to_value(b).ok(),
                _ => true,
            };
            if !object_changed && !metadata_changed {
                return None;
            }
            let object = new.or(old)?;
            let name = new_info
                .and_then(|info| info.name.clone())
                .or_else(|| old_info.and_then(|info| info.name.clone()))
                .unwrap_or_else(|| format!("Object_{id}"));
            Some((
                old.map(|_| id),
                new.map(|_| id),
                ObjectSummary {
                    id,
                    name,
                    object_type: format!("{:?}", object.object_type()),
                },
            ))
        })
        .collect()
}

fn apply_set_properties(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let target: ObjectTarget = serde_json::from_value(
        action
            .get("object")
            .cloned()
            .ok_or("set_properties requires object")?,
    )
    .map_err(|e| e.to_string())?;
    let values = action
        .get("values")
        .and_then(Value::as_object)
        .ok_or("set_properties requires values")?;
    let state = project.ai_draft_state();
    let object_id = match &target {
        ObjectTarget::Id { id } => *id,
        ObjectTarget::Name { name } => {
            let matches: Vec<_> = state
                .pool
                .objects()
                .iter()
                .filter_map(|object| {
                    let object_name = state
                        .object_info
                        .get(&object.id())
                        .and_then(|info| info.name.as_deref())
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            crate::smart_naming::get_object_type_name(object.object_type())
                                .to_owned()
                        });
                    (object_name == *name).then_some(object.id().value())
                })
                .collect();
            if matches.len() != 1 {
                return Err(if matches.is_empty() {
                    format!("Unknown object name '{name}'")
                } else {
                    format!("Ambiguous object name '{name}'")
                });
            }
            matches[0]
        }
    };
    let object = state
        .pool
        .objects()
        .iter()
        .find(|o| o.id().value() == object_id)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    let descriptors = get_property_descriptors(object.object_type());
    for (property, raw) in values {
        let lexical = raw
            .as_str()
            .ok_or_else(|| format!("{property} must be supplied as a lexical string"))?;
        let descriptor = descriptors
            .iter()
            .find(|d| d.name == property)
            .ok_or_else(|| format!("Unknown property '{property}'"))?;
        if !descriptor.writable {
            return Err(format!("Property '{property}' is read-only"));
        }
        if let PropertySemantic::ObjectReference { .. } = &descriptor.semantic {
            let reference = if lexical == "null" {
                None
            } else {
                let target = if let Some(hex) = lexical.strip_prefix("0x") {
                    ObjectTarget::id(
                        u16::from_str_radix(hex, 16)
                            .map_err(|_| format!("Invalid object ID '{lexical}'"))?,
                    )
                } else if let Ok(id) = lexical.parse::<u16>() {
                    ObjectTarget::id(id)
                } else {
                    ObjectTarget::Name {
                        name: lexical.to_owned(),
                    }
                };
                Some(target)
            };
            project.apply_ai_draft_operation(&Operation::SetObjectReference {
                object: ObjectTarget::id(object_id),
                property: property.clone(),
                reference,
            })?;
            continue;
        }
        let default = get_object_property(object, property).map_err(|e| e.to_string())?;
        let value = match &descriptor.semantic {
            PropertySemantic::Colour => {
                let input = if let Ok(index) = lexical.parse::<u8>() {
                    serde_json::json!(index)
                } else if lexical.starts_with('#') {
                    serde_json::json!({"rgb":lexical})
                } else {
                    serde_json::json!({"name":lexical})
                };
                let resolved = super::visual_intent::resolve(&state.pool, &input)
                    .map_err(|e| e.to_string())?;
                serde_json::json!(resolved.index)
            }
            PropertySemantic::Enum if !descriptor.enum_choices.is_empty() => descriptor
                .enum_choices
                .iter()
                .find(|choice| choice.as_str() == Some(lexical))
                .cloned()
                .or_else(|| {
                    serde_json::from_str::<Value>(lexical)
                        .ok()
                        .filter(|candidate| descriptor.enum_choices.contains(candidate))
                })
                .ok_or_else(|| {
                    format!(
                        "Invalid value '{lexical}' for {property}; choose one of {}",
                        descriptor
                            .enum_choices
                            .iter()
                            .map(Value::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?,
            _ => match default {
                Value::String(_) => Value::String(lexical.into()),
                Value::Bool(_) => match lexical {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => return Err(format!("{property} must be true or false")),
                },
                Value::Number(ref n) if n.is_i64() || n.is_u64() => {
                    serde_json::from_str::<Value>(lexical)
                        .ok()
                        .filter(Value::is_number)
                        .ok_or_else(|| format!("{property} must be an integer"))?
                }
                Value::Number(_) => serde_json::from_str::<Value>(lexical)
                    .ok()
                    .filter(Value::is_number)
                    .ok_or_else(|| format!("{property} must be a number"))?,
                _ => {
                    return Err(format!(
                        "Property '{property}' requires a structured-value command"
                    ))
                }
            },
        };
        project.apply_ai_draft_operation(&Operation::SetProperty {
            object: ObjectTarget::id(object_id),
            property: property.clone(),
            value,
        })?;
    }
    Ok(())
}

fn apply_settings(
    project: &mut EditorProject,
    values: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    let allowed = [
        "mask_size",
        "soft_key_width",
        "soft_key_height",
        "vt_version",
        "soft_key_layout",
        "arrangement",
        "physical_key_count",
        "keys_per_line",
        "navigation_key_count",
    ];
    if let Some(key) = values.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unknown project setting '{key}'"));
    }
    let get_u16 = |key: &str, current: u16| -> Result<u16, String> {
        values
            .get(key)
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| format!("{key} must be an integer from 0 to 65535"))
            })
            .unwrap_or(Ok(current))
    };
    let mask = get_u16("mask_size", project.mask_size)?;
    let width = get_u16("soft_key_width", project.soft_key_size().0)?;
    let height = get_u16("soft_key_height", project.soft_key_size().1)?;
    let version = values
        .get("vt_version")
        .map(|v| {
            let raw = v.as_u64().ok_or("vt_version must be an integer")?;
            if !(2..=6).contains(&raw) {
                return Err("vt_version must be between 2 and 6");
            }
            VtVersion::try_from(
                u8::try_from(raw).map_err(|_| "vt_version must be between 2 and 6")?,
            )
            .map_err(|_| "vt_version must be between 2 and 6")
        })
        .transpose()?
        .unwrap_or_else(|| project.vt_version());
    let mut layout: crate::SoftKeyLayout = values
        .get("soft_key_layout")
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or_else(|| project.soft_key_layout());
    if let Some(arrangement) = values.get("arrangement") {
        layout.arrangement = match arrangement.as_str() {
            Some("right" | "Right") => crate::soft_key_layout::SoftKeyArrangement::Right,
            Some("bottom" | "Bottom") => crate::soft_key_layout::SoftKeyArrangement::Bottom,
            _ => return Err("arrangement must be right or bottom".into()),
        };
    }
    for (key, slot) in [
        ("physical_key_count", &mut layout.physical_key_count),
        ("keys_per_line", &mut layout.keys_per_line),
        ("navigation_key_count", &mut layout.navigation_key_count),
    ] {
        if let Some(value) = values.get(key) {
            *slot = value
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .ok_or_else(|| format!("{key} must be an integer from 0 to 255"))?;
        }
    }
    let ranges = project.object_id_ranges().clone();
    project.set_ai_draft_settings_unchecked(mask, (width, height), version, layout, ranges);
    Ok(())
}

fn apply_action(project: &mut EditorProject, action: Value) -> Result<(), String> {
    match action.get("kind").and_then(Value::as_str).unwrap_or("") {
        "id_range_reset" => {
            project.set_ai_draft_settings_unchecked(
                project.mask_size,
                project.soft_key_size(),
                project.vt_version(),
                project.soft_key_layout(),
                crate::ObjectIdRanges::defaults(),
            );
            Ok(())
        }
        "id_range_set" => {
            let mut ranges = project.object_id_ranges().clone();
            let object_type: ag_iso_stack::object_pool::ObjectType = serde_json::from_value(
                action
                    .get("object_type")
                    .cloned()
                    .ok_or("id_range_set requires object_type")?,
            )
            .map_err(|e| e.to_string())?;
            let start = action
                .get("start")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .ok_or("id_range_set requires start")?;
            let end = action
                .get("end")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .ok_or("id_range_set requires end")?;
            ranges.set_range(object_type, crate::ObjectIdRange { start, end });
            project.set_ai_draft_settings_unchecked(
                project.mask_size,
                project.soft_key_size(),
                project.vt_version(),
                project.soft_key_layout(),
                ranges,
            );
            Ok(())
        }
        "graphic_import" => {
            let filename = action
                .get("filename")
                .and_then(Value::as_str)
                .ok_or("graphic_import requires filename")?;
            let encoded = action
                .get("bytes_base64")
                .and_then(Value::as_str)
                .ok_or("graphic_import requires bytes_base64")?;
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                .map_err(|e| e.to_string())?;
            let id = crate::graphics::integration::GraphicService::upload(project, filename, bytes)
                .map_err(|e| e.to_string())?;
            if let Some(name) = action.get("name").and_then(Value::as_str) {
                crate::graphics::integration::GraphicService::rename_available_asset(
                    project,
                    id,
                    name.into(),
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        "graphic_rename" => {
            let id = graphic_id(&action)?;
            let name = action
                .get("name")
                .and_then(Value::as_str)
                .ok_or("graphic_rename requires name")?;
            crate::graphics::integration::GraphicService::rename_available_asset(
                project,
                id,
                name.into(),
            )
            .map_err(|e| e.to_string())
        }
        "graphic_delete" => crate::graphics::integration::GraphicService::delete_available_asset(
            project,
            graphic_id(&action)?,
        )
        .map_err(|e| e.to_string()),
        "graphic_assign" => {
            let id = graphic_id(&action)?;
            let object = action
                .get("object")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .ok_or("graphic_assign requires numeric object")?;
            crate::graphics::integration::GraphicService::convert_asset_into_picture_graphic(
                project, id, object,
            )
            .map_err(|e| e.to_string())
        }
        "graphic_create_picture" => {
            crate::graphics::integration::GraphicService::create_picture_graphic_from_asset(
                project,
                graphic_id(&action)?,
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        }
        _ => super::draft_actions::apply(project, &action),
    }
}

fn graphic_id(action: &Value) -> Result<crate::graphics::GraphicAssetId, String> {
    serde_json::from_value(
        action
            .get("asset_id")
            .cloned()
            .ok_or("graphic action requires asset_id")?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_draft_applies_and_undoes_as_one_content_checkpoint() {
        let mut project = EditorProject::default();
        let state = project.ai_design_state();
        let draft = begin(&state, Some("mixed edit".into()));
        let edited = edit(&state, &draft, vec![
            serde_json::json!({"kind":"operation", "operation":{"type":"CreateObject", "data":{"object_id":null,"object_type":"Button","name":"New button"}}}),
            serde_json::json!({"kind":"settings", "values":{"mask_size":640}}),
        ]).unwrap();
        assert_eq!(edited.generation, 1);
        let proposal = submit(&state, &edited).unwrap();
        project.apply_ai_proposal(proposal).unwrap();
        assert_eq!(project.mask_size, 640);
        assert_eq!(project.ai_design_state().pool.objects().len(), 1);
        assert!(project.undo_operation());
        assert_eq!(project.mask_size, 500);
        assert!(project.ai_design_state().pool.objects().is_empty());
        assert!(project.redo_operation());
        assert_eq!(project.mask_size, 640);
        assert_eq!(project.ai_design_state().pool.objects().len(), 1);
    }

    #[test]
    fn failed_batch_does_not_advance_or_change_source_draft() {
        let project = EditorProject::default();
        let state = project.ai_design_state();
        let draft = begin(&state, None);
        let result = edit(
            &state,
            &draft,
            vec![
                serde_json::json!({"kind":"settings", "values":{"mask_size":720}}),
                serde_json::json!({"kind":"unsupported"}),
            ],
        );
        assert!(result.is_err());
        assert_eq!(draft.generation, 0);
        assert_eq!(draft.content.mask_size, 500);
    }
}
