//! Shared export generation for the CLI and editor.
use crate::EditorProject;
use ag_iso_stack::object_pool::{object::Object, ObjectPool};
use std::collections::HashMap;

/// Raw artifacts generated for one export request. Encoding belongs to the
/// transport that serializes these bytes.
#[derive(Debug, Default)]
pub struct ExportArtifacts {
    pub iop: Option<Vec<u8>>,
    pub header: Option<Vec<u8>>,
    pub vt_version: u8,
}

/// Generate requested project artifacts from one materialized project snapshot.
/// `format` is `iop`, `header`, or `bundle`.
pub fn artifacts(project: &EditorProject, format: &str) -> Result<ExportArtifacts, String> {
    if !matches!(format, "iop" | "header" | "bundle") {
        return Err(format!(
            "Unknown export format '{format}'. Choose iop, header, or bundle."
        ));
    }
    let materialized = project
        .materialize_shared()
        .map_err(|error| error.to_string())?;
    let header = if matches!(format, "header" | "bundle") {
        let info = project.component_object_info();
        Some(generate_header(&materialized.pool, |object| {
            object_name(project, &info, &materialized.source_map, object)
        })?)
    } else {
        None
    };
    let iop = if matches!(format, "iop" | "bundle") {
        Some(project.export_iop().map_err(|error| error.to_string())?)
    } else {
        None
    };

    Ok(ExportArtifacts {
        iop,
        header: header.map(String::into_bytes),
        vt_version: u8::from(project.vt_version),
    })
}

/// Generate a C object ID header for the editor's current materialized project.
pub fn header_for_project(project: &EditorProject) -> Result<Vec<u8>, String> {
    artifacts(project, "header")?
        .header
        .ok_or_else(|| "Header export did not produce an artifact".to_owned())
}

fn object_name(
    project: &EditorProject,
    info: &HashMap<ag_iso_stack::object_pool::ObjectId, crate::ObjectInfo>,
    source_map: &crate::components::MaterializedSourceMap,
    object: &Object,
) -> String {
    let id = object.id();
    if let Some(info) = info.get(&id) {
        return info.get_name(object);
    }
    if let Some(crate::components::MaterializedSource::ComponentNode {
        instance_id,
        node_id,
    }) = source_map.by_object_id.get(&id.value())
    {
        if let Some(instance) = project
            .component_source()
            .instances
            .iter()
            .find(|i| i.id == *instance_id)
        {
            if let Some(definition) = project
                .component_source()
                .definitions
                .get(&instance.component_id)
            {
                if let Some(node) = definition.nodes.iter().find(|n| n.id == *node_id) {
                    return format!("{}_{}_{}", definition.name, node.name, id.value());
                }
            }
        }
    }
    crate::smart_naming::get_object_type_name(object.object_type()).to_owned()
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
