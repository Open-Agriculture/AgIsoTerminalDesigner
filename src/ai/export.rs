//! Shared export generation for the CLI and editor.
use crate::{ai::AiDesignState, EditorProject};
use ag_iso_stack::object_pool::{object::Object, ObjectPool};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Generate requested project artifacts from one materialized project snapshot.
///
/// `format` is `iop`, `header`, or `bundle`. Artifact bytes are base64 encoded
/// because the automation bridge transports JSON values.
pub fn artifacts(state: &AiDesignState, format: &str) -> Result<Value, String> {
    if !matches!(format, "iop" | "header" | "bundle") {
        return Err(format!(
            "Unknown export format '{format}'. Choose iop, header, or bundle."
        ));
    }
    let project = EditorProject::from_ai_source(state);
    let materialized = project
        .materialize_shared()
        .map_err(|error| error.to_string())?;
    let header = if matches!(format, "header" | "bundle") {
        Some(generate_header(&materialized.pool, |object| {
            object_name(state, &materialized.source_map, object)
        })?)
    } else {
        None
    };
    let iop = if matches!(format, "iop" | "bundle") {
        Some(project.export_iop().map_err(|error| error.to_string())?)
    } else {
        None
    };

    let mut generated = serde_json::Map::new();
    match format {
        "iop" => {
            generated.insert("iop".into(), encode(iop.as_deref().unwrap()));
        }
        "header" => {
            generated.insert(
                "header".into(),
                encode(header.as_deref().unwrap().as_bytes()),
            );
        }
        "bundle" => {
            generated.insert("iop".into(), encode(iop.as_deref().unwrap()));
            generated.insert(
                "header".into(),
                encode(header.as_deref().unwrap().as_bytes()),
            );
        }
        _ => unreachable!("validated format above"),
    }
    Ok(json!({
        "artifacts": generated,
        "vt_version": u8::from(project.vt_version),
    }))
}

/// Generate a C object ID header for the editor's current materialized project.
pub fn header_for_project(project: &EditorProject) -> Result<Vec<u8>, String> {
    let value = artifacts(&project.ai_design_state(), "header")?;
    let encoded = value["artifacts"]["header"]
        .as_str()
        .ok_or_else(|| "Header export did not produce an artifact".to_owned())?;
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|error| format!("Could not decode generated header: {error}"))
}

fn object_name(
    state: &AiDesignState,
    source_map: &crate::components::MaterializedSourceMap,
    object: &Object,
) -> String {
    let id = object.id();
    if let Some(info) = state.object_info.get(&id) {
        return info.get_name(object);
    }
    if let Some(crate::components::MaterializedSource::ComponentNode {
        instance_id,
        node_id,
    }) = source_map.by_object_id.get(&id.value())
    {
        if let Some(instance) = state
            .components
            .instances
            .iter()
            .find(|i| i.id == *instance_id)
        {
            if let Some(definition) = state.components.definitions.get(&instance.component_id) {
                if let Some(node) = definition.nodes.iter().find(|n| n.id == *node_id) {
                    return format!("{}_{}_{}", definition.name, node.name, id.value());
                }
            }
        }
    }
    crate::smart_naming::get_object_type_name(object.object_type()).to_owned()
}

fn encode(bytes: &[u8]) -> Value {
    Value::String(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        bytes,
    ))
}

fn generate_header(
    pool: &ObjectPool,
    mut name_for: impl FnMut(&Object) -> String,
) -> Result<String, String> {
    let mut objects: Vec<(String, String, u16)> = pool
        .objects()
        .iter()
        .map(|object| {
            let original = name_for(object);
            let identifier = c_identifier(&original);
            (identifier, original, u16::from(object.id()))
        })
        .collect();
    objects.sort_by_key(|(_, _, id)| *id);

    let mut seen: HashMap<String, (String, u16)> = HashMap::new();
    for (identifier, name, id) in &objects {
        if identifier == "UNDEFINED" {
            return Err(format!(
                "Object {id} ('{name}') maps to reserved C identifier UNDEFINED. Rename the object before exporting the header."
            ));
        }
        if let Some((previous_name, previous_id)) =
            seen.insert(identifier.clone(), (name.clone(), *id))
        {
            return Err(format!(
                "Objects {previous_id} ('{previous_name}') and {id} ('{name}') both map to C identifier {identifier}. Rename one object before exporting the header."
            ));
        }
    }

    let mut header = String::from("// Object IDs for the objects in the materialized object pool.\n\n#pragma once\n#define UNDEFINED 65535\n");
    for (identifier, _, id) in objects {
        header.push_str(&format!("#define {identifier} {id}\n"));
    }
    Ok(header)
}

fn c_identifier(name: &str) -> String {
    let mut identifier: String = name
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' => character.to_ascii_uppercase(),
            _ => '_',
        })
        .collect();
    if identifier.is_empty() {
        identifier.push('_');
    }
    if identifier.as_bytes()[0].is_ascii_digit() {
        identifier.insert(0, '_');
    }
    identifier
}

#[cfg(test)]
mod tests {
    use super::{c_identifier, generate_header};
    use ag_iso_stack::object_pool::{ObjectPool, ObjectType};

    #[test]
    fn c_names_are_ascii_uppercase_and_start_with_a_non_digit() {
        assert_eq!(c_identifier("Start button"), "START_BUTTON");
        assert_eq!(c_identifier("42"), "_42");
        assert_eq!(c_identifier("é"), "_");
    }

    #[test]
    fn header_rejects_colliding_and_reserved_names() {
        let mut pool = ObjectPool::new();
        pool.add(crate::default_object(ObjectType::Button));
        pool.add(crate::default_object(ObjectType::Button));
        let mut names = ["Start button", "Start-button"].into_iter();
        let collision = generate_header(&pool, |_| names.next().unwrap().to_owned()).unwrap_err();
        assert!(collision.contains("both map to C identifier START_BUTTON"));

        let reserved = generate_header(&pool, |_| "UNDEFINED".to_owned()).unwrap_err();
        assert!(reserved.contains("reserved C identifier UNDEFINED"));
    }
}
