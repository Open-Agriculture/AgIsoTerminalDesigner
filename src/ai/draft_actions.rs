//! Draft-only authoring actions that sit above primitive project operations.
//!
//! The CLI keeps its wire format private; these helpers translate its more
//! expressive commands to the same editor services used by the GUI.

use crate::{
    components::{
        plan_component_capture, ComponentDefinition, ComponentId, ComponentInstanceId,
        ComponentNodeId, ComponentParameter, ComponentParameterId, ComponentParameterKind,
        ComponentService,
    },
    object_info::ObjectInfo,
    object_properties::{get_object_property, get_property_descriptors, PropertySemantic},
    operations::{
        object_list, object_refs, ChildPlacement, MacroReferenceTarget, ObjectReferenceList,
        ObjectTarget, ObjectTargetList, Operation,
    },
    EditorProject,
};
use ag_iso_stack::object_pool::{
    object::Object,
    object_attributes::{Event, ObjectLabel},
    NullableObjectId, ObjectId,
};
use serde_json::{json, Value};

/// Apply an action owned by the extended draft command surface.
///
/// Actions already handled by the draft foundation return an explicit
/// `Unhandled draft action` error so the caller can continue its dispatcher.
pub(crate) fn apply(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let kind = action
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("Draft action requires a kind")?;
    match kind {
        "component_capture" => component_capture(project, action),
        "component_move" => ComponentService::move_instance(
            project,
            component_instance_id(action, "instance_id")?,
            i32_value(action, "x")?,
            i32_value(action, "y")?,
        )
        .map_err(|e| e.to_string()),
        "component_delete" => ComponentService::delete_component_instance(
            project,
            component_instance_id(action, "instance_id")?,
        )
        .map_err(|e| e.to_string()),
        "component_duplicate" => ComponentService::duplicate_component_instance(
            project,
            component_instance_id(action, "instance_id")?,
            i32_value(action, "x")?,
            i32_value(action, "y")?,
        )
        .map(|_| ())
        .map_err(|e| e.to_string()),
        "component_detach" => ComponentService::detach_component_instance(
            project,
            component_instance_id(action, "instance_id")?,
        )
        .map_err(|e| e.to_string()),
        "component_definition_rename" => {
            let id = component_id(action, "component_id")?;
            let name = string(action, "name")?;
            let mut definition = definition(project, id)?;
            definition.name = name.to_owned();
            ComponentService::edit_component_definition(project, definition)
                .map_err(|e| e.to_string())
        }
        "component_definition_duplicate" => {
            ComponentService::duplicate_definition(project, component_id(action, "component_id")?)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        "component_definition_delete" => {
            ComponentService::delete_definition(project, component_id(action, "component_id")?)
                .map_err(|e| e.to_string())
        }
        "component_node_set" => {
            let component_id = component_id(action, "component_id")?;
            let node_id = component_node_id(action, "node_id")?;
            let property = string(action, "property")?;
            let definition = definition(project, component_id)?;
            let node = definition
                .nodes
                .iter()
                .find(|node| node.id == node_id)
                .ok_or_else(|| format!("Component node {node_id} not found"))?;
            let value = if let Some(lexical) = action.get("value_lexical").and_then(Value::as_str) {
                parse_node_property_value(project, node, property, lexical)?
            } else {
                action
                    .get("value")
                    .cloned()
                    .ok_or("component_node_set requires value")?
            };
            ComponentService::apply_intents(
                project,
                &[
                    crate::components::ComponentIntent::SetComponentMasterProperty {
                        component_id,
                        node_id,
                        property: property.to_owned(),
                        value,
                        scope: crate::components::ComponentMasterScope::AllInstances,
                    },
                ],
            )
            .map_err(|e| e.to_string())
        }
        "component_node_reference" => component_node_reference(project, action),
        "component_parameter_add" => {
            let mut parameter_value = required(action, "parameter")?.clone();
            if let Some(lexical) = action
                .get("default_value_lexical")
                .and_then(Value::as_str)
                .or_else(|| {
                    parameter_value
                        .get("default_value_lexical")
                        .and_then(Value::as_str)
                })
            {
                let kind: ComponentParameterKind =
                    serde_json::from_value(required(&parameter_value, "kind")?.clone())
                        .map_err(|e| format!("Invalid parameter kind: {e}"))?;
                let node_id: ComponentNodeId =
                    serde_json::from_value(required(&parameter_value, "target_node")?.clone())
                        .map_err(|e| format!("Invalid parameter target node: {e}"))?;
                let definition = definition(project, component_id(action, "component_id")?)?;
                let node = definition
                    .nodes
                    .iter()
                    .find(|node| node.id == node_id)
                    .ok_or_else(|| format!("Component node {node_id} not found"))?;
                let property = required(&parameter_value, "property")?
                    .as_str()
                    .ok_or("Parameter property must be a string")?
                    .to_owned();
                parameter_value["default_value"] =
                    coerce_parameter_value(project, node, &property, kind, lexical)?;
            }
            if parameter_value.get("id").is_none() {
                parameter_value["id"] = serde_json::to_value(ComponentParameterId::default())
                    .map_err(|e| format!("Could not allocate parameter ID: {e}"))?;
            }
            ComponentService::add_component_parameter(
                project,
                component_id(action, "component_id")?,
                serde_json::from_value(parameter_value)
                    .map_err(|e| format!("Invalid component parameter: {e}"))?,
            )
            .map_err(|e| e.to_string())
        }
        "component_parameter_set" => {
            let parameter = parse_component_parameter(project, action)?;
            ComponentService::edit_component_parameter(
                project,
                component_id(action, "component_id")?,
                parameter,
            )
            .map_err(|e| e.to_string())
        }
        "component_parameter_remove" => ComponentService::remove_component_parameter(
            project,
            component_id(action, "component_id")?,
            component_parameter_id(action, "parameter_id")?,
        )
        .map_err(|e| e.to_string()),
        "component_override" => component_override(project, action),
        "component_reset" => component_reset(project, action),
        "object_duplicate" => object_duplicate(project, action),
        "object_id_auto" => object_id_auto(project, action),
        "object_id_reassign" => object_id_reassign(project, action),
        "object_field_set"
        | "object_array_set"
        | "object_array_append"
        | "object_array_remove"
        | "object_array_clear" => structured_property(project, action),
        "object_bytes_set" => object_bytes_set(project, action),
        "relationship_edit" => relationship_edit(project, action),
        // The primitive operation family has shared validation and reference
        // resolution in Operation::apply; adapt symbolic targets before using it.
        "relationship" => apply_relationship(project, action),
        _ => Err(format!("Unhandled draft action: {kind}")),
    }
}

fn component_capture(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let root = u16_value(action, "root")?;
    let name = string(action, "name")?;
    let plan = plan_component_capture(project, root, name.to_owned()).map_err(|e| e.to_string())?;
    let keep_external_references = action
        .get("keep_external_references")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    ComponentService::create_component_from_selection(project, plan, keep_external_references)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn component_override(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let instance_id = component_instance_id(action, "instance_id")?;
    let source = project.component_source();
    let instance = source
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| format!("Component instance {instance_id} not found"))?;
    let definition = source
        .definitions
        .get(&instance.component_id)
        .ok_or_else(|| format!("Component {} not found", instance.component_id))?;
    let parameter_key = string(action, "parameter")?;
    let parameter = find_parameter(definition, parameter_key)?;
    let lexical = action
        .get("value_lexical")
        .and_then(Value::as_str)
        .or_else(|| action.get("value").and_then(Value::as_str));
    let value = if let Some(lexical) = lexical {
        let node = definition
            .nodes
            .iter()
            .find(|node| node.id == parameter.target_node)
            .ok_or_else(|| format!("Component node {} not found", parameter.target_node))?;
        coerce_parameter_value(project, node, &parameter.property, parameter.kind, lexical)?
    } else {
        action
            .get("value")
            .cloned()
            .ok_or("component_override requires value")?
    };
    ComponentService::set_component_override(project, instance_id, parameter.id, value)
        .map_err(|e| e.to_string())
}

fn component_reset(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let instance_id = component_instance_id(action, "instance_id")?;
    let source = project.component_source();
    let instance = source
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| format!("Component instance {instance_id} not found"))?;
    let definition = source
        .definitions
        .get(&instance.component_id)
        .ok_or_else(|| format!("Component {} not found", instance.component_id))?;
    let parameter = find_parameter(definition, string(action, "parameter")?)?;
    ComponentService::reset_component_override(project, instance_id, parameter.id)
        .map_err(|e| e.to_string())
}

fn parse_component_parameter(
    project: &EditorProject,
    action: &Value,
) -> Result<ComponentParameter, String> {
    let mut parameter_value = required(action, "parameter")?.clone();
    if let Some(lexical) = action
        .get("default_value_lexical")
        .and_then(Value::as_str)
        .or_else(|| {
            parameter_value
                .get("default_value_lexical")
                .and_then(Value::as_str)
        })
    {
        let kind: ComponentParameterKind =
            serde_json::from_value(required(&parameter_value, "kind")?.clone())
                .map_err(|e| format!("Invalid parameter kind: {e}"))?;
        let node_id: ComponentNodeId =
            serde_json::from_value(required(&parameter_value, "target_node")?.clone())
                .map_err(|e| format!("Invalid parameter target node: {e}"))?;
        let component_id = component_id(action, "component_id")?;
        let definition = definition(project, component_id)?;
        let node = definition
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| format!("Component node {node_id} not found"))?;
        let property = required(&parameter_value, "property")?
            .as_str()
            .ok_or("Parameter property must be a string")?
            .to_owned();
        parameter_value["default_value"] =
            coerce_parameter_value(project, node, &property, kind, lexical)?;
    }
    serde_json::from_value(parameter_value).map_err(|e| format!("Invalid component parameter: {e}"))
}

fn find_parameter<'a>(
    definition: &'a ComponentDefinition,
    token: &str,
) -> Result<&'a ComponentParameter, String> {
    if let Ok(id) = serde_json::from_str::<ComponentParameterId>(&format!("\"{token}\"")) {
        if let Some(parameter) = definition
            .parameters
            .iter()
            .find(|parameter| parameter.id == id)
        {
            return Ok(parameter);
        }
    }
    let matches: Vec<_> = definition
        .parameters
        .iter()
        .filter(|parameter| parameter.name == token)
        .collect();
    match matches.as_slice() {
        [parameter] => Ok(parameter),
        [] => Err(format!("Unknown component parameter '{token}'")),
        _ => Err(format!("Ambiguous component parameter '{token}'")),
    }
}

fn coerce_parameter_value(
    project: &EditorProject,
    node: &crate::components::ComponentNode,
    property: &str,
    kind: ComponentParameterKind,
    lexical: &str,
) -> Result<Value, String> {
    match kind {
        ComponentParameterKind::Text => Ok(Value::String(lexical.to_owned())),
        ComponentParameterKind::Number => serde_json::from_str::<Value>(lexical)
            .ok()
            .filter(Value::is_number)
            .ok_or_else(|| format!("'{lexical}' must be a number")),
        ComponentParameterKind::Boolean => match lexical {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!("'{lexical}' must be true or false")),
        },
        ComponentParameterKind::Graphic => lexical
            .parse::<u16>()
            .map(|id| json!(id))
            .map_err(|_| format!("'{lexical}' must be a graphic ID")),
        ComponentParameterKind::ObjectReference => {
            if lexical == "null" {
                Ok(Value::Null)
            } else {
                serde_json::to_value(object_target(project, lexical)?)
                    .map_err(|e| format!("Invalid object reference: {e}"))
            }
        }
        ComponentParameterKind::Colour => {
            let input = if let Ok(index) = lexical.parse::<u8>() {
                json!(index)
            } else if lexical.starts_with('#') {
                json!({"rgb":lexical})
            } else {
                json!({"name":lexical})
            };
            let resolved = super::visual_intent::resolve(&project.get_pool(), &input)
                .map_err(|e| e.to_string())?;
            Ok(json!(resolved.index))
        }
        ComponentParameterKind::Enum => parse_node_property_value(project, node, property, lexical),
    }
}

fn parse_node_property_value(
    project: &EditorProject,
    node: &crate::components::ComponentNode,
    property: &str,
    lexical: &str,
) -> Result<Value, String> {
    let descriptor = get_property_descriptors(node.object_type)
        .into_iter()
        .find(|descriptor| descriptor.name == property)
        .ok_or_else(|| format!("Unknown component property '{property}'"))?;
    if !descriptor.writable {
        return Err(format!("Component property '{property}' is read-only"));
    }
    match descriptor.semantic {
        PropertySemantic::ObjectReference { .. } => Err(format!(
            "'{property}' is a reference; use component node-reference"
        )),
        PropertySemantic::Colour => {
            let input = if let Ok(index) = lexical.parse::<u8>() {
                json!(index)
            } else if lexical.starts_with('#') {
                json!({"rgb":lexical})
            } else {
                json!({"name":lexical})
            };
            let resolved = super::visual_intent::resolve(&project.get_pool(), &input)
                .map_err(|e| e.to_string())?;
            Ok(json!(resolved.index))
        }
        PropertySemantic::Enum => descriptor
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
            }),
        PropertySemantic::Normal => {
            let default = node
                .properties
                .get(property)
                .cloned()
                .or_else(|| {
                    get_object_property(&crate::default_object(node.object_type), property).ok()
                })
                .ok_or_else(|| format!("No value schema for '{property}'"))?;
            match default {
                Value::String(_) => Ok(Value::String(lexical.to_owned())),
                Value::Bool(_) => match lexical {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    _ => Err(format!("'{property}' must be true or false")),
                },
                Value::Number(ref number) if number.is_i64() || number.is_u64() => {
                    serde_json::from_str::<Value>(lexical)
                        .ok()
                        .filter(Value::is_number)
                        .ok_or_else(|| format!("'{property}' must be an integer"))
                }
                Value::Number(_) => serde_json::from_str::<Value>(lexical)
                    .ok()
                    .filter(Value::is_number)
                    .ok_or_else(|| format!("'{property}' must be a number")),
                _ => Err(format!("'{property}' requires a structured value")),
            }
        }
        PropertySemantic::Bitflags => serde_json::from_str::<Value>(lexical)
            .ok()
            .filter(Value::is_number)
            .ok_or_else(|| format!("'{property}' requires a numeric flag value")),
    }
}

fn component_node_reference(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    use crate::components::ComponentReference;
    let component_id = component_id(action, "component_id")?;
    let node_id = component_node_id(action, "node_id")?;
    let property = string(action, "property")?;
    let target = string(action, "target")?;
    let mut definition = definition(project, component_id)?;
    let node = definition
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .ok_or("Unknown component node")?;
    let descriptor = get_property_descriptors(node.object_type)
        .into_iter()
        .find(|descriptor| descriptor.name == property)
        .ok_or("Unknown node property")?;
    if !descriptor.writable
        || !matches!(
            descriptor.semantic,
            PropertySemantic::ObjectReference { .. }
        )
    {
        return Err(format!("'{property}' is not a writable reference"));
    }
    let reference = if target == "null" {
        ComponentReference::Null
    } else if let Some(id) = target.strip_prefix("node:") {
        let id = crate::components::ComponentNodeId(
            uuid::Uuid::parse_str(id).map_err(|e| e.to_string())?,
        );
        if !definition.nodes.iter().any(|node| node.id == id) {
            return Err("Unknown internal target node".into());
        }
        ComponentReference::Internal(id)
    } else {
        ComponentReference::ExternalObject(object_target(project, target)?)
    };
    let bound = definition
        .parameters
        .iter_mut()
        .find(|parameter| parameter.target_node == node_id && parameter.property == property);
    let authored = if let Some(parameter) = bound {
        parameter.default_value = match &reference {
            ComponentReference::Null => Value::Null,
            ComponentReference::ExternalObject(target) => {
                serde_json::to_value(target).map_err(|e| e.to_string())?
            }
            _ => {
                return Err("Remove the reference parameter before binding an internal node".into())
            }
        };
        ComponentReference::Parameter(parameter.id)
    } else {
        reference
    };
    let node = definition
        .nodes
        .iter_mut()
        .find(|node| node.id == node_id)
        .unwrap();
    node.properties.remove(property);
    node.references.insert(property.into(), authored);
    ComponentService::edit_component_definition(project, definition).map_err(|e| e.to_string())
}

fn object_duplicate(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let source_id = u16_value(action, "object_id")?;
    let source = project
        .get_pool()
        .object_by_id(ObjectId::new(source_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {source_id} not found"))?
        .clone();
    let object_type = source.object_type();
    let range = project.object_id_ranges().range_for(object_type);
    let reserved: std::collections::HashSet<_> = project
        .component_source()
        .instances
        .iter()
        .flat_map(|i| i.compiled_ids.values().copied())
        .collect();
    let id = (range.start..=range.end)
        .find(|id| {
            !reserved.contains(id)
                && ObjectId::new(*id)
                    .ok()
                    .is_some_and(|candidate| project.get_pool().object_by_id(candidate).is_none())
        })
        .ok_or_else(|| format!("No free {object_type:?} object ID remains in {range}"))?;
    let source_info = project.get_object_info(&source);
    let base_name = action
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Copy of {}", source_info.get_name(&source)));
    if base_name.trim().is_empty() {
        return Err("Duplicate object name cannot be empty".into());
    }
    let names: std::collections::HashSet<_> = project
        .get_pool()
        .objects()
        .iter()
        .map(|object| project.get_object_info(object).get_name(object))
        .collect();
    let mut name = base_name.clone();
    let mut suffix = 2_u32;
    while names.contains(&name) {
        name = format!("{base_name} ({suffix})");
        suffix += 1;
    }
    let id = ObjectId::new(id).map_err(|e| format!("Invalid object ID: {e:?}"))?;
    let mut duplicate = source;
    duplicate
        .mut_id()
        .set_value(id.value())
        .map_err(|e| format!("Invalid duplicate object ID: {e:?}"))?;
    let mut info = ObjectInfo::new(&duplicate);
    info.name = Some(name);
    info.description = source_info.description;
    info.tags = source_info.tags;
    info.allowed_references = source_info.allowed_references;
    project.apply_ai_draft_operation(&Operation::RestoreObject {
        object: duplicate,
        info: Some(info),
    })?;
    Ok(())
}

fn object_id_auto(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let old_id = u16_value(action, "object_id")?;
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(old_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {old_id} not found"))?;
    let range = project.object_id_ranges().range_for(object.object_type());
    let reserved: std::collections::HashSet<_> = project
        .component_source()
        .instances
        .iter()
        .flat_map(|i| i.compiled_ids.values().copied())
        .collect();
    let new_id = (range.start..=range.end)
        .find(|candidate| {
            *candidate != old_id
                && !reserved.contains(candidate)
                && ObjectId::new(*candidate)
                    .ok()
                    .is_some_and(|id| project.get_pool().object_by_id(id).is_none())
        })
        .ok_or_else(|| format!("No fresh IDs remain in the {range} range"))?;
    project.apply_ai_draft_operation(&Operation::SetObjectIdAssignment {
        object_id: old_id,
        new_id,
        overridden: false,
    })
}

fn object_id_reassign(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let plan = project.plan_id_reassignment(
        action
            .get("include_overrides")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )?;
    if let Some(expected) = action.get("mappings") {
        let supplied: Vec<(u16, u16)> = serde_json::from_value(expected.clone())
            .map_err(|e| format!("Invalid ID mappings: {e}"))?;
        if supplied != plan.mappings {
            return Err("ID reassignment plan changed; inspect the current plan and retry".into());
        }
    }
    if plan.mappings.is_empty() {
        return Ok(());
    }
    project.apply_ai_draft_operation(&Operation::RemapObjectIds {
        mappings: plan.mappings,
        target_types: plan.target_types,
    })
}

fn structured_property(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let object_target: ObjectTarget = serde_json::from_value(required(action, "object")?.clone())
        .map_err(|e| format!("Invalid object target: {e}"))?;
    let object_id = resolve_object(project, &object_target)?;
    let property = string(action, "property")?;
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    let descriptors = get_property_descriptors(object.object_type());
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.name == property)
        .ok_or_else(|| format!("Unknown property '{property}'"))?;
    if !descriptor.writable {
        return Err(format!("Property '{property}' is read-only"));
    }
    let mut value = get_object_property(object, &property).map_err(|e| e.to_string())?;
    match action["kind"].as_str().unwrap_or("") {
        "object_field_set" => {
            let path = string(action, "field")?;
            let replacement =
                coerce_to_prototype(required(action, "value")?.clone(), get_path(&value, &path)?)?;
            set_path(&mut value, &path, replacement)?;
        }
        "object_array_set" => {
            let index = usize_value(action, "index")?;
            let replacement = required(action, "value")?.clone();
            let array = value.as_array_mut().ok_or("Property is not an array")?;
            let slot = array.get_mut(index).ok_or("Array index is out of bounds")?;
            *slot = coerce_to_prototype(replacement, slot)?;
        }
        "object_array_append" => {
            let array = value.as_array_mut().ok_or("Property is not an array")?;
            let incoming = required(action, "value")?.clone();
            let incoming = if let Some(prototype) = array.first() {
                coerce_to_prototype(incoming, prototype)?
            } else {
                parse_record_value(incoming)?
            };
            array.push(incoming);
        }
        "object_array_remove" => {
            let index = usize_value(action, "index")?;
            let array = value.as_array_mut().ok_or("Property is not an array")?;
            if index >= array.len() {
                return Err("Array index is out of bounds".into());
            }
            array.remove(index);
        }
        "object_array_clear" => value
            .as_array_mut()
            .ok_or("Property is not an array")?
            .clear(),
        _ => unreachable!(),
    }
    project.apply_ai_draft_operation(&Operation::SetProperty {
        object: ObjectTarget::id(object_id),
        property: property.to_owned(),
        value,
    })
}

fn object_bytes_set(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let object_target: ObjectTarget = serde_json::from_value(required(action, "object")?.clone())
        .map_err(|e| format!("Invalid object target: {e}"))?;
    let object_id = resolve_object(project, &object_target)?;
    let property = string(action, "property")?;
    let encoded = string(action, "bytes_base64")?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|e| format!("Invalid base64 bytes: {e}"))?;
    let value = Value::Array(bytes.into_iter().map(|byte| json!(byte)).collect());
    project.apply_ai_draft_operation(&Operation::SetProperty {
        object: ObjectTarget::id(object_id),
        property: property.to_owned(),
        value,
    })
}

fn apply_relationship(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let operation_value = action
        .get("operation")
        .cloned()
        .ok_or("relationship action requires operation")?;
    let mut operation: Operation = serde_json::from_value(operation_value)
        .map_err(|e| format!("Invalid relationship operation: {e}"))?;
    if let Operation::DeleteObject { object_id } = &mut operation {
        if let Some(target) = action.get("object") {
            let target: ObjectTarget = serde_json::from_value(target.clone())
                .map_err(|e| format!("Invalid object target: {e}"))?;
            *object_id = resolve_object(project, &target)?;
        }
    }
    project.apply_ai_draft_operation(&operation)
}

fn relationship_edit(project: &mut EditorProject, action: &Value) -> Result<(), String> {
    let group = string(action, "group")?;
    let verb = string(action, "verb")?;
    let args = action
        .get("args")
        .and_then(Value::as_array)
        .ok_or("relationship_edit requires args")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or("relationship args must be strings")
        })
        .collect::<Result<Vec<_>, _>>()?;
    match group {
        "pointer" => pointer_edit(project, verb, &args),
        "list" => list_edit(project, verb, &args),
        "event" => event_edit(project, verb, &args),
        "label" => label_edit(project, verb, &args),
        "placement" => placement_edit(project, verb, &args),
        _ => Err(format!("Unknown relationship group '{group}'")),
    }
}

fn pointer_edit(project: &mut EditorProject, verb: &str, args: &[String]) -> Result<(), String> {
    let (object_token, targets) = args
        .split_first()
        .ok_or("pointer command requires an object")?;
    let object = object_target(project, object_token)?;
    let references = match verb {
        "clear" if targets.is_empty() => Vec::new(),
        "clear" => return Err("pointer clear accepts only an object".into()),
        "set" => targets
            .iter()
            .map(|target| object_target(project, target))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(format!("Unsupported pointer command '{verb}'")),
    };
    project.apply_ai_draft_operation(&Operation::SetObjectPointerAllowedReferences {
        object,
        references,
    })
}

fn list_edit(project: &mut EditorProject, verb: &str, args: &[String]) -> Result<(), String> {
    let object_token = args.first().ok_or("list command requires an object")?;
    let object = object_target(project, object_token)?;
    let object_id = resolve_object(project, &object)?;
    let list = current_object_list(project, object_id)?;
    match verb {
        "set" => {
            let mut start = 1;
            if args.get(1).is_some_and(|field| field == "objects") {
                start += 1;
            }
            if let Some(kind) = args
                .get(start)
                .filter(|value| **value == "required" || **value == "nullable")
            {
                let requested_kind = kind == "required";
                if requested_kind != matches!(&list, ObjectReferenceList::Required(_)) {
                    return Err("Requested list kind does not match this object's list type".into());
                }
                start += 1;
            }
            let targets = args[start..]
                .iter()
                .map(|target| optional_target(project, target))
                .collect::<Result<Vec<_>, _>>()?;
            let objects = match list {
                ObjectReferenceList::Required(_) => {
                    if targets.iter().any(Option::is_none) {
                        return Err("Required object lists cannot contain null".into());
                    }
                    ObjectTargetList::Required(targets.into_iter().flatten().collect())
                }
                ObjectReferenceList::Nullable(_) => ObjectTargetList::Nullable(targets),
            };
            project.apply_ai_draft_operation(&Operation::SetObjectList { object, objects })
        }
        "insert" => {
            let start = usize::from(args.get(1).is_some_and(|field| field == "objects")) + 1;
            if args.len() != start + 2 {
                return Err("list insert syntax is: OBJECT [objects] INDEX TARGET".into());
            }
            let index = parse_usize(&args[start], "list index")?;
            let target = optional_target(project, &args[start + 1])?;
            let mut targets = list_targets(&list);
            if index > targets.len() {
                return Err("List index is out of bounds".into());
            }
            if target.is_none() && matches!(&list, ObjectReferenceList::Required(_)) {
                return Err("Required object lists cannot contain null".into());
            }
            targets.insert(index, target);
            apply_object_list(project, object, list, targets)
        }
        "remove" => {
            let start = usize::from(args.get(1).is_some_and(|field| field == "objects")) + 1;
            if args.len() != start + 1 {
                return Err("list remove syntax is: OBJECT [objects] INDEX".into());
            }
            let index = parse_usize(&args[start], "list index")?;
            let mut targets = list_targets(&list);
            if index >= targets.len() {
                return Err("List index is out of bounds".into());
            }
            targets.remove(index);
            apply_object_list(project, object, list, targets)
        }
        "clear" => {
            if args.len() > 2 || (args.len() == 2 && args[1] != "objects") {
                return Err("list clear syntax is: OBJECT [objects]".into());
            }
            let objects = match list {
                ObjectReferenceList::Required(_) => ObjectTargetList::Required(Vec::new()),
                ObjectReferenceList::Nullable(_) => ObjectTargetList::Nullable(Vec::new()),
            };
            project.apply_ai_draft_operation(&Operation::SetObjectList { object, objects })
        }
        _ => Err(format!("Unsupported list command '{verb}'")),
    }
}

fn event_edit(project: &mut EditorProject, verb: &str, args: &[String]) -> Result<(), String> {
    let object_token = args.first().ok_or("event command requires an object")?;
    let object = object_target(project, object_token)?;
    let object_id = resolve_object(project, &object)?;
    let mut bindings = current_macro_refs(project, object_id)?;
    match verb {
        "clear" => {
            if args.len() != 1 {
                return Err("event clear accepts only an object".into());
            }
            bindings.clear();
        }
        "bind" | "set" => {
            if args.len() < 2 {
                return Err(format!("event {verb} requires EVENT=MACRO entries"));
            }
            let replacements = parse_macro_bindings(project, &args[1..])?;
            if verb == "set" {
                bindings = replacements;
            } else {
                for replacement in replacements {
                    if let Some(current) = bindings
                        .iter_mut()
                        .find(|current| current.event_id == replacement.event_id)
                    {
                        current.macro_object = replacement.macro_object;
                    } else {
                        bindings.push(replacement);
                    }
                }
            }
        }
        "unbind" => {
            if args.len() != 2 {
                return Err("event unbind syntax is: OBJECT EVENT".into());
            }
            let event = parse_event(&args[1])?;
            let original_len = bindings.len();
            bindings.retain(|binding| binding.event_id != event);
            if bindings.len() == original_len {
                return Err(format!("No macro is bound to event {}", args[1]));
            }
        }
        "order" => {
            if args.len() < 2 {
                return Err("event order requires the existing events in desired order".into());
            }
            let ordered_events = args[1..]
                .iter()
                .map(|event| parse_event(event))
                .collect::<Result<Vec<_>, _>>()?;
            let has_duplicate = ordered_events
                .iter()
                .enumerate()
                .any(|(index, event)| ordered_events[..index].contains(event));
            if ordered_events.len() != bindings.len() || has_duplicate {
                return Err("Event order must list every bound event exactly once".into());
            }
            let mut ordered = Vec::with_capacity(bindings.len());
            for event in ordered_events {
                let binding = bindings
                    .iter()
                    .find(|binding| binding.event_id == event)
                    .ok_or_else(|| format!("Event {event:?} is not currently bound"))?;
                ordered.push(binding.clone());
            }
            bindings = ordered;
        }
        _ => return Err(format!("Unsupported event command '{verb}'")),
    }
    project.apply_ai_draft_operation(&Operation::SetMacroReferences {
        object,
        macro_refs: bindings,
    })
}

fn label_edit(project: &mut EditorProject, verb: &str, args: &[String]) -> Result<(), String> {
    let object_token = args.first().ok_or("label command requires an object")?;
    let object_id = resolve_object(project, &object_target(project, object_token)?)?;
    let mut labels = current_labels(project, object_id)?;
    match verb {
        "clear" => {
            if args.len() != 1 {
                return Err("label clear accepts only an object".into());
            }
            labels.clear();
        }
        "remove" => {
            if args.len() != 2 {
                return Err("label remove syntax is: OBJECT LABEL_ID".into());
            }
            let id = resolve_object(project, &object_target(project, &args[1])?)?;
            let original_len = labels.len();
            labels.retain(|label| label.id.value() != id);
            if original_len == labels.len() {
                return Err(format!("Label {id} not found"));
            }
        }
        "add" | "set" => {
            if args.len() < 2 {
                return Err(format!(
                    "label {verb} requires id=… string=… graphic=… font=…"
                ));
            }
            let fields = key_value_args(&args[1..])?;
            let label_id = resolve_object(
                project,
                &object_target(project, fields.get("id").ok_or("label requires id=…")?)?,
            )?;
            let string_id = optional_object_id(project, fields.get("string").map(String::as_str))?;
            let graphic_id =
                optional_object_id(project, fields.get("graphic").map(String::as_str))?;
            let font_type = fields
                .get("font")
                .ok_or("label requires font=…")?
                .parse::<u8>()
                .map_err(|_| "label font must be an integer from 0 to 255")?;
            if fields
                .keys()
                .any(|key| !["id", "string", "graphic", "font"].contains(&key.as_str()))
            {
                return Err("label accepts only id, string, graphic, and font fields".into());
            }
            let label = ObjectLabel {
                id: ObjectId::new(label_id).map_err(|e| format!("Invalid label ID: {e:?}"))?,
                string_variable_reference: string_id
                    .map(NullableObjectId::new)
                    .unwrap_or(NullableObjectId::NULL),
                font_type,
                graphic_representation: graphic_id
                    .map(NullableObjectId::new)
                    .unwrap_or(NullableObjectId::NULL),
            };
            if let Some(existing) = labels.iter_mut().find(|existing| existing.id == label.id) {
                if verb == "add" {
                    return Err(format!("Label {} already exists", label.id.value()));
                }
                *existing = label;
            } else {
                labels.push(label);
            }
        }
        _ => return Err(format!("Unsupported label command '{verb}'")),
    }
    project.apply_ai_draft_operation(&Operation::SetObjectLabels { object_id, labels })
}

fn placement_edit(project: &mut EditorProject, verb: &str, args: &[String]) -> Result<(), String> {
    match verb {
        "add" => {
            if args.len() != 4 {
                return Err("placement add syntax is: PARENT CHILD X Y".into());
            }
            let parent = object_target(project, &args[0])?;
            let child = object_target(project, &args[1])?;
            let x = parse_i16(&args[2], "x")?;
            let y = parse_i16(&args[3], "y")?;
            project.apply_ai_draft_operation(&Operation::AddChild {
                parent,
                child,
                x,
                y,
            })
        }
        "move" | "remove" => {
            let base_len = if verb == "move" { 4 } else { 2 };
            if args.len() < base_len || args.len() > base_len + 2 {
                return Err(if verb == "move" {
                    "placement move syntax is: PARENT CHILD X Y [OCCURRENCE]".into()
                } else {
                    "placement remove syntax is: PARENT CHILD [OCCURRENCE]".into()
                });
            }
            let parent_id = resolve_object(project, &object_target(project, &args[0])?)?;
            let child_id = resolve_object(project, &object_target(project, &args[1])?)?;
            let mut placements = current_placements(project, parent_id)?;
            let occurrence = parse_occurrence(&args[base_len..])?;
            let index = find_placement_index(&placements, child_id, occurrence)?;
            if verb == "move" {
                placements[index].x = parse_i16(&args[2], "x")?;
                placements[index].y = parse_i16(&args[3], "y")?;
            } else {
                placements.remove(index);
            }
            project.apply_ai_draft_operation(&Operation::SetChildren {
                parent: ObjectTarget::id(parent_id),
                children: placements,
            })
        }
        "order" => {
            if args.len() < 2 {
                return Err("placement order syntax is: PARENT CHILD...".into());
            }
            let parent_id = resolve_object(project, &object_target(project, &args[0])?)?;
            let placements = current_placements(project, parent_id)?;
            let desired = args[1..]
                .iter()
                .map(|child| resolve_object(project, &object_target(project, child)?))
                .collect::<Result<Vec<_>, _>>()?;
            if desired.len() != placements.len() {
                return Err("placement order must list every child occurrence exactly once".into());
            }
            let mut remaining = placements;
            let mut ordered = Vec::with_capacity(desired.len());
            for child_id in desired {
                let index = remaining
                    .iter()
                    .position(|placement| placement.child == ObjectTarget::id(child_id))
                    .ok_or_else(|| {
                        format!("Child {child_id} is missing or repeated in placement order")
                    })?;
                ordered.push(remaining.remove(index));
            }
            if !remaining.is_empty() {
                return Err("placement order must list every child occurrence exactly once".into());
            }
            project.apply_ai_draft_operation(&Operation::SetChildren {
                parent: ObjectTarget::id(parent_id),
                children: ordered,
            })
        }
        _ => Err(format!("Unsupported placement command '{verb}'")),
    }
}

fn current_object_list(
    project: &EditorProject,
    object_id: u16,
) -> Result<ObjectReferenceList, String> {
    let id = ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?;
    let object = project
        .get_pool()
        .object_by_id(id)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    object_list(object)
        .ok_or_else(|| format!("Object {object_id} does not own an ordered object list"))
}

fn list_targets(list: &ObjectReferenceList) -> Vec<Option<ObjectTarget>> {
    match list {
        ObjectReferenceList::Required(ids) => ids
            .iter()
            .map(|id| Some(ObjectTarget::id(id.value())))
            .collect(),
        ObjectReferenceList::Nullable(ids) => ids
            .iter()
            .map(|id| id.0.map(|id| ObjectTarget::id(id.value())))
            .collect(),
    }
}

fn apply_object_list(
    project: &mut EditorProject,
    object: ObjectTarget,
    current: ObjectReferenceList,
    targets: Vec<Option<ObjectTarget>>,
) -> Result<(), String> {
    let objects = match current {
        ObjectReferenceList::Required(_) => {
            let ids = targets
                .into_iter()
                .map(|target| {
                    let target = target.ok_or("Required object lists cannot contain null")?;
                    resolve_object(project, &target).map(ObjectTarget::id)
                })
                .collect::<Result<Vec<_>, _>>()?;
            ObjectTargetList::Required(ids)
        }
        ObjectReferenceList::Nullable(_) => ObjectTargetList::Nullable(
            targets
                .into_iter()
                .map(|target| {
                    target
                        .map(|target| resolve_object(project, &target).map(ObjectTarget::id))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    project.apply_ai_draft_operation(&Operation::SetObjectList { object, objects })
}

fn current_macro_refs(
    project: &EditorProject,
    object_id: u16,
) -> Result<Vec<MacroReferenceTarget>, String> {
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    crate::operations::macro_refs(object)
        .map(|references| {
            references
                .iter()
                .map(|reference| MacroReferenceTarget {
                    event_id: reference.event_id,
                    macro_object: ObjectTarget::id(reference.macro_id.into()),
                })
                .collect()
        })
        .ok_or_else(|| format!("Object {object_id} does not support macro bindings"))
}

fn parse_macro_bindings(
    project: &EditorProject,
    entries: &[String],
) -> Result<Vec<MacroReferenceTarget>, String> {
    let mut bindings = Vec::with_capacity(entries.len());
    for entry in entries {
        let (event_name, target_name) = entry
            .split_once('=')
            .ok_or_else(|| format!("Expected EVENT=MACRO, got '{entry}'"))?;
        let event_id = parse_event(event_name)?;
        if bindings
            .iter()
            .any(|binding: &MacroReferenceTarget| binding.event_id == event_id)
        {
            return Err(format!("Event {event_name} appears more than once"));
        }
        bindings.push(MacroReferenceTarget {
            event_id,
            macro_object: object_target(project, target_name)?,
        });
    }
    Ok(bindings)
}

fn parse_event(value: &str) -> Result<Event, String> {
    let event: Event = serde_json::from_value(json!(value)).map_err(|_| {
        format!("Unknown event '{value}'; use a name such as OnActivate or OnKeyPress")
    })?;
    if event == Event::Reserved {
        return Err("Reserved is not an editable event".into());
    }
    Ok(event)
}

fn current_labels(project: &EditorProject, object_id: u16) -> Result<Vec<ObjectLabel>, String> {
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    match object {
        Object::ObjectLabelReferenceList(list) => Ok(list.object_labels.clone()),
        _ => Err(format!(
            "Object {object_id} is not an ObjectLabelReferenceList"
        )),
    }
}

fn current_placements(
    project: &EditorProject,
    parent_id: u16,
) -> Result<Vec<ChildPlacement>, String> {
    let parent = project
        .get_pool()
        .object_by_id(ObjectId::new(parent_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Parent object {parent_id} not found"))?;
    object_refs(parent)
        .map(|references| {
            references
                .iter()
                .map(|reference| ChildPlacement {
                    child: ObjectTarget::id(reference.id.value()),
                    x: reference.offset.x,
                    y: reference.offset.y,
                })
                .collect()
        })
        .ok_or_else(|| format!("Object {parent_id} does not own positioned children"))
}

fn find_placement_index(
    placements: &[ChildPlacement],
    child_id: u16,
    occurrence: Option<usize>,
) -> Result<usize, String> {
    let matches: Vec<_> = placements
        .iter()
        .enumerate()
        .filter_map(|(index, placement)| {
            (placement.child == ObjectTarget::id(child_id)).then_some(index)
        })
        .collect();
    match occurrence {
        Some(occurrence) => matches.get(occurrence).copied().ok_or_else(|| {
            format!("Placement occurrence {occurrence} not found for child {child_id}")
        }),
        None if matches.len() == 1 => Ok(matches[0]),
        None if matches.is_empty() => {
            Err(format!("Child {child_id} is not placed under this parent"))
        }
        None => Err(format!(
            "Child {child_id} has {} placements; specify --occurrence",
            matches.len()
        )),
    }
}

fn key_value_args(args: &[String]) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut fields = std::collections::BTreeMap::new();
    for arg in args {
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("Expected key=value, got '{arg}'"))?;
        if fields.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!("Field '{key}' was supplied more than once"));
        }
    }
    Ok(fields)
}

fn optional_object_id(project: &EditorProject, value: Option<&str>) -> Result<Option<u16>, String> {
    match value {
        Some("null") | None => Ok(None),
        Some(value) => resolve_object(project, &object_target(project, value)?).map(Some),
    }
}

fn optional_target(project: &EditorProject, value: &str) -> Result<Option<ObjectTarget>, String> {
    if value == "null" {
        Ok(None)
    } else {
        object_target(project, value).map(Some)
    }
}

fn object_target(project: &EditorProject, value: &str) -> Result<ObjectTarget, String> {
    let target = if let Some(name) = value.strip_prefix("name:") {
        ObjectTarget::Name {
            name: name.to_owned(),
        }
    } else if let Some(hex) = value.strip_prefix("0x") {
        ObjectTarget::id(
            u16::from_str_radix(hex, 16).map_err(|_| format!("Invalid object ID '{value}'"))?,
        )
    } else if let Ok(id) = value.parse::<u16>() {
        ObjectTarget::id(id)
    } else {
        ObjectTarget::Name {
            name: value.to_owned(),
        }
    };
    // Resolve names now so callers get useful ambiguity and missing-name errors
    // before the lower-level operation validator reports a generic target error.
    resolve_object(project, &target)?;
    Ok(target)
}

fn parse_i16(value: &str, field: &str) -> Result<i16, String> {
    value
        .parse()
        .map_err(|_| format!("{field} must be an integer from -32768 to 32767"))
}

fn parse_usize(value: &str, field: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("{field} must be a nonnegative integer"))
}

fn parse_occurrence(args: &[String]) -> Result<Option<usize>, String> {
    match args {
        [] => Ok(None),
        [flag, value] if flag == "--occurrence" => {
            parse_usize(value, "placement occurrence").map(Some)
        }
        _ => Err("Expected --occurrence INDEX".into()),
    }
}

fn set_path(value: &mut Value, path: &str, replacement: Value) -> Result<(), String> {
    let mut parts = path.split('.').peekable();
    let mut current = value;
    while let Some(part) = parts.next() {
        let is_last = parts.peek().is_none();
        if let Ok(index) = part.parse::<usize>() {
            let array = current
                .as_array_mut()
                .ok_or("Field path expected an array")?;
            current = array.get_mut(index).ok_or("Field index is out of bounds")?;
        } else {
            let object = current
                .as_object_mut()
                .ok_or("Field path expected an object")?;
            if is_last {
                let slot = object
                    .get_mut(part)
                    .ok_or_else(|| format!("Unknown field '{part}'"))?;
                *slot = replacement;
                return Ok(());
            }
            current = object
                .get_mut(part)
                .ok_or_else(|| format!("Unknown field '{part}'"))?;
        }
        if is_last {
            *current = replacement;
            return Ok(());
        }
    }
    Err("Field path cannot be empty".into())
}

fn get_path<'a>(value: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut current = value;
    for part in path.split('.') {
        if let Ok(index) = part.parse::<usize>() {
            current = current
                .as_array()
                .and_then(|array| array.get(index))
                .ok_or("Field index is out of bounds")?;
        } else {
            current = current
                .as_object()
                .and_then(|object| object.get(part))
                .ok_or_else(|| format!("Unknown field '{part}'"))?;
        }
    }
    Ok(current)
}

fn coerce_to_prototype(value: Value, prototype: &Value) -> Result<Value, String> {
    if matches!(prototype, Value::String(_)) {
        return Ok(match value {
            Value::String(_) => value,
            Value::Number(number) => Value::String(number.to_string()),
            Value::Bool(boolean) => Value::String(boolean.to_string()),
            Value::Null => Value::Null,
            _ => return Err("Expected a scalar string value for this field".into()),
        });
    }
    let Some(lexical) = value.as_str() else {
        if matches!(prototype, Value::Bool(_)) && value.is_boolean()
            || matches!(prototype, Value::Number(_)) && value.is_number()
            || matches!(prototype, Value::Array(_)) && value.is_array()
            || matches!(prototype, Value::Object(_)) && value.is_object()
            || matches!(prototype, Value::Null) && value.is_null()
        {
            return Ok(value);
        }
        return Err("Value type does not match the field's existing value".into());
    };
    match prototype {
        Value::String(_) => Ok(Value::String(lexical.to_owned())),
        Value::Bool(_) => match lexical {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err("Expected true or false for this field".into()),
        },
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            serde_json::from_str::<Value>(lexical)
                .ok()
                .filter(Value::is_number)
                .ok_or_else(|| "Expected an integer for this field".into())
        }
        Value::Number(_) => serde_json::from_str::<Value>(lexical)
            .ok()
            .filter(Value::is_number)
            .ok_or_else(|| "Expected a number for this field".into()),
        Value::Array(_) => serde_json::from_str::<Value>(lexical)
            .ok()
            .filter(Value::is_array)
            .ok_or_else(|| "Expected an array value".into()),
        Value::Object(_) => parse_record_value(value),
        Value::Null => Ok(value),
    }
}

fn parse_record_value(value: Value) -> Result<Value, String> {
    let Some(lexical) = value.as_str() else {
        return Ok(value);
    };
    if !lexical.contains('=') {
        return Ok(Value::String(lexical.to_owned()));
    }
    if lexical.trim().is_empty() {
        return Err("Record values must contain comma-separated field=value pairs".into());
    }
    let mut record = serde_json::Map::new();
    for assignment in lexical.split(',') {
        let (field, raw) = assignment
            .split_once('=')
            .ok_or_else(|| format!("Expected field=value in record, got '{assignment}'"))?;
        if field.trim().is_empty() {
            return Err("Record field name cannot be empty".into());
        }
        if record
            .insert(field.trim().to_owned(), parse_scalar(raw.trim()))
            .is_some()
        {
            return Err(format!(
                "Record field '{}' was supplied more than once",
                field.trim()
            ));
        }
    }
    Ok(Value::Object(record))
}

fn parse_scalar(value: &str) -> Value {
    if let Some(text) = value.strip_prefix("string:") {
        return Value::String(text.to_owned());
    }
    if value == "true" {
        return Value::Bool(true);
    }
    if value == "false" {
        return Value::Bool(false);
    }
    if value == "null" {
        return Value::Null;
    }
    serde_json::from_str::<Value>(value)
        .ok()
        .filter(|value| value.is_number())
        .unwrap_or_else(|| Value::String(value.to_owned()))
}

fn resolve_object(project: &EditorProject, target: &ObjectTarget) -> Result<u16, String> {
    match target {
        ObjectTarget::Id { id } => Ok(*id),
        ObjectTarget::Name { name } => {
            let matches: Vec<_> = project
                .get_pool()
                .objects()
                .iter()
                .filter(|object| project.get_object_info(object).get_name(object) == *name)
                .map(|object| object.id().value())
                .collect();
            match matches.as_slice() {
                [id] => Ok(*id),
                [] => Err(format!("Unknown object name '{name}'")),
                _ => Err(format!(
                    "Ambiguous object name '{name}': {}",
                    matches
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
    }
}

fn definition(project: &EditorProject, id: ComponentId) -> Result<ComponentDefinition, String> {
    project
        .component_source()
        .definitions
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("Component {id} not found"))
}

fn required<'a>(action: &'a Value, key: &str) -> Result<&'a Value, String> {
    action
        .get(key)
        .ok_or_else(|| format!("Action requires {key}"))
}
fn string<'a>(action: &'a Value, key: &str) -> Result<&'a str, String> {
    required(action, key)?
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
fn u16_value(action: &Value, key: &str) -> Result<u16, String> {
    required(action, key)?
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| format!("{key} must be an integer from 0 to 65535"))
}
fn i32_value(action: &Value, key: &str) -> Result<i32, String> {
    required(action, key)?
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| format!("{key} must be a 32-bit integer"))
}
fn usize_value(action: &Value, key: &str) -> Result<usize, String> {
    required(action, key)?
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| format!("{key} must be a nonnegative integer"))
}
fn component_id(action: &Value, key: &str) -> Result<ComponentId, String> {
    serde_json::from_value(required(action, key)?.clone())
        .map_err(|e| format!("Invalid {key}: {e}"))
}
fn component_node_id(action: &Value, key: &str) -> Result<ComponentNodeId, String> {
    serde_json::from_value(required(action, key)?.clone())
        .map_err(|e| format!("Invalid {key}: {e}"))
}
fn component_instance_id(action: &Value, key: &str) -> Result<ComponentInstanceId, String> {
    serde_json::from_value(required(action, key)?.clone())
        .map_err(|e| format!("Invalid {key}: {e}"))
}
fn component_parameter_id(action: &Value, key: &str) -> Result<ComponentParameterId, String> {
    serde_json::from_value(required(action, key)?.clone())
        .map_err(|e| format!("Invalid {key}: {e}"))
}
