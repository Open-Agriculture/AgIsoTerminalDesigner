//! Shared project inspection and export operations used by the standalone CLI.
use super::protocol::AutomationAction;
use crate::cli;
use ag_iso_stack::object_pool::ObjectType;
use serde_json::{json, Value};

#[derive(Debug)]
pub struct DomainError {
    pub code: &'static str,
    pub message: String,
}

impl DomainError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn inspection_error(error: cli::AiToolError) -> DomainError {
    match error {
        cli::AiToolError::ObjectNotFound(id) => {
            DomainError::new("OBJECT_NOT_FOUND", format!("Object {id} was not found"))
        }
        cli::AiToolError::RequestTooLarge => DomainError::new(
            "INVALID_ARGUMENTS",
            "Object request exceeds the supported limit",
        ),
        cli::AiToolError::InvalidScope => {
            DomainError::new("INVALID_ARGUMENTS", "Invalid design inspection scope")
        }
    }
}

pub fn execute(
    project: &crate::EditorProject,
    tool: AutomationAction,
    args: Value,
) -> Result<Value, DomainError> {
    let state = &project.ai_design_state();
    let mut args = if args.is_null() { json!({}) } else { args };
    match tool {
        AutomationAction::ExportObjectBytes => {
            let target: crate::ObjectTarget = serde_json::from_value(args["object"].clone())
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let id = crate::operations::resolve_target(&target, &state.pool, &state.object_info)
                .map_err(|e| DomainError::new("OBJECT_NOT_FOUND", e.to_string()))?;
            let property = args["property"]
                .as_str()
                .ok_or_else(|| DomainError::new("INVALID_ARGUMENTS", "Provide a property name"))?;
            let property = property
                .parse::<crate::object_properties::PropertyKey>()
                .map_err(|error| DomainError::new("INVALID_ARGUMENTS", error.to_string()))?;
            let value = crate::object_properties::get_object_property(
                state.pool.object_by_id(id).unwrap(),
                property,
            )
            .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let bytes = value
                .as_array()
                .ok_or_else(|| {
                    DomainError::new("INVALID_ARGUMENTS", "This property is not a byte array")
                })?
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or_else(|| {
                            DomainError::new(
                                "INVALID_ARGUMENTS",
                                "This property is not a byte array",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(
                json!({"object_id":id.value(),"property":property,"bytes_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes)}),
            )
        }
        AutomationAction::GetObjectTypes => Ok(
            json!({"types":ObjectType::values().into_iter().map(|ty|json!({
            "type":format!("{ty:?}"),"supported_for_target":crate::vt_compatibility::supported_object_type(ty,state.vt_version)
        })).collect::<Vec<_>>()}),
        ),
        AutomationAction::GetDesignContext => {
            if let Some(scope) = args["scope"].as_str() {
                args["scope"] = json!(match scope {
                    "active_mask" => "ActiveMask",
                    "selection" => "Selection",
                    "pool" | "entire_pool" => "EntirePool",
                    other => other,
                });
            }
            // A root inside a component instance shows its own contents;
            // otherwise generated objects are summarised as instances.
            let generated_root = args["root_id"].as_u64().is_some_and(|id| {
                crate::components::source_map(&state.components)
                    .by_object_id
                    .contains_key(&(id as u16))
            });
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let context = cli::get_design_context(state, request).map_err(inspection_error)?;
            let context = if generated_root {
                serde_json::to_value(context).expect("snapshot serializes")
            } else {
                cli::tools::component_design_context(state, context)
            };
            Ok(json!({ "context": context }))
        }
        AutomationAction::GetObjects => {
            if let Some(targets) = args.get("targets").and_then(Value::as_array) {
                let ids = targets
                    .iter()
                    .map(|target| {
                        let target: crate::ObjectTarget = serde_json::from_value(target.clone())
                            .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
                        crate::operations::operation::resolve_target(
                            &target,
                            &state.pool,
                            &state.object_info,
                        )
                        .map(|id| id.value())
                        .or_else(|error| {
                            // Names win; a number that names nothing is an object
                            // ID. This also reaches component instance objects,
                            // which have no names of their own.
                            let (
                                crate::ObjectTarget::Name { name },
                                crate::operations::OperationError::ObjectNameNotFound(_),
                            ) = (&target, &error)
                            else {
                                return Err(error);
                            };
                            name.parse::<u16>()
                                .ok()
                                .and_then(|id| ag_iso_stack::object_pool::ObjectId::new(id).ok())
                                .filter(|id| state.pool.object_by_id(*id).is_some())
                                .map(|id| id.value())
                                .or_else(|| cli::names::readable_object_id(project, name).ok())
                                .ok_or(error)
                        })
                        .map_err(|e| DomainError::new("OBJECT_NOT_FOUND", e.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                args["ids"] = json!(ids);
                args.as_object_mut().unwrap().remove("targets");
            }
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let objects = cli::get_objects(state, request).map_err(inspection_error)?;
            let sources = objects
                .iter()
                .filter_map(|o| cli::tools::source_context(&state.components, o.object.id))
                .collect::<Vec<_>>();
            let values = serde_json::to_value(objects).unwrap();
            Ok(json!({"sources":sources,"objects":values}))
        }
        AutomationAction::ListObjects => {
            let query = args["query"].as_str().unwrap_or("").to_lowercase();
            let type_filter = args
                .get("object_type")
                .or_else(|| args.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let parent = args["parent_id"]
                .as_u64()
                .and_then(|id| u16::try_from(id).ok());
            if !args["parent_id"].is_null() && parent.is_none() {
                return Err(DomainError::new(
                    "INVALID_ARGUMENTS",
                    "parent_id must be a valid object ID",
                ));
            }
            if let Some(parent_id) = parent {
                if ag_iso_stack::object_pool::ObjectId::new(parent_id).is_err() {
                    return Err(DomainError::new(
                        "INVALID_ARGUMENTS",
                        "parent_id is not a valid object ID",
                    ));
                }
            }
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 200) as usize;
            let unused = args["unused"].as_bool().unwrap_or(false);
            let columns = list_columns(&args["columns"])?;
            let c_identifiers = if columns.contains(&"c_ident") {
                Some(
                    cli::export::c_identifiers(project)
                        .map_err(|e| DomainError::new("EXPORT_FAILED", e))?,
                )
            } else {
                None
            };
            let usage = unused.then(|| object_usage(state));
            let parent_children = parent.map(|parent_id| {
                ag_iso_stack::object_pool::ObjectId::new(parent_id)
                    .ok()
                    .map(|root| {
                        cli::snapshot::layout_instances(
                            &state.pool,
                            &state.object_info,
                            Some(root),
                            cli::snapshot::MAX_SNAPSHOT_DEPTH,
                        )
                        .into_iter()
                        .filter(|placement| placement.parent.id == parent_id)
                        .map(|placement| placement.object.id)
                        .collect::<std::collections::HashSet<_>>()
                    })
                    .unwrap_or_default()
            });
            let mut matches = Vec::new();
            for object in state.pool.objects() {
                let id = object.id().value();
                let object_type = format!("{:?}", object.object_type());
                let name = state
                    .object_info
                    .get(&object.id())
                    .map(|info| info.get_name(object))
                    .unwrap_or_else(|| {
                        crate::smart_naming::get_object_type_name(object.object_type()).to_owned()
                    });
                if !type_filter.is_empty() && !object_type.to_lowercase().contains(&type_filter) {
                    continue;
                }
                if !query.is_empty()
                    && !format!("{name} {object_type} {id}")
                        .to_lowercase()
                        .contains(&query)
                {
                    continue;
                }
                if parent.is_some() {
                    if !parent_children
                        .as_ref()
                        .is_some_and(|children| children.contains(&id))
                    {
                        continue;
                    }
                }
                if let Some(usage) = &usage {
                    if usage.referenced.contains(&id) || is_top_level(object.object_type()) {
                        continue;
                    }
                }
                let row = columns
                    .iter()
                    .map(|column| {
                        let value = match *column {
                            "id" => json!(id),
                            "name" => json!(name),
                            "type" => json!(object_type),
                            "c_ident" => json!(c_identifiers.as_ref().and_then(|c| c.get(&id))),
                            _ => unreachable!("columns are validated"),
                        };
                        ((*column).to_owned(), value)
                    })
                    .collect::<serde_json::Map<_, _>>();
                matches.push((id, Value::Object(row)));
            }
            matches.sort_by_key(|(id, _)| *id);
            let total = matches.len();
            let objects = matches
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|(_, row)| row)
                .collect::<Vec<_>>();
            let has_more = offset.saturating_add(objects.len()) < total;
            let mut result = json!({"objects":objects,"columns":columns,"total":total,"offset":offset,"limit":limit,"has_more":has_more});
            if let Some(usage) = usage.filter(|usage| !usage.unresolved_macros.is_empty()) {
                // Macros whose commands could not be decoded may reference
                // objects listed here, so name them instead of guessing.
                result["unresolved_macros"] = json!(usage.unresolved_macros);
            }
            Ok(result)
        }
        AutomationAction::GetObjectSchema => {
            let names: Vec<&str> = match args.get("object_types").and_then(Value::as_array) {
                Some(types) => types
                    .iter()
                    .map(|v| {
                        v.as_str().ok_or_else(|| {
                            DomainError::new(
                                "INVALID_ARGUMENTS",
                                "object_types must contain strings",
                            )
                        })
                    })
                    .collect::<Result<_, _>>()?,
                None => vec![args["object_type"].as_str().ok_or_else(|| {
                    DomainError::new("INVALID_ARGUMENTS", "Provide object_type or object_types")
                })?],
            };
            if names.is_empty() || names.len() > 10 {
                return Err(DomainError::new(
                    "INVALID_ARGUMENTS",
                    "Provide 1..10 object types",
                ));
            }
            let mut schemas = Vec::new();
            for name in names {
                let ty = ObjectType::values()
                    .into_iter()
                    .find(|v| format!("{v:?}") == name)
                    .ok_or_else(|| {
                        DomainError::new(
                            "INVALID_ARGUMENTS",
                            format!("Unknown object type: {name}"),
                        )
                    })?;
                let mut schema = cli::get_object_schema(state, ty);
                if let Some(properties) = args["properties"].as_array().filter(|p| !p.is_empty()) {
                    if let Some(fields) = schema["properties"].as_array_mut() {
                        fields.retain(|field| properties.contains(&field["name"]));
                    }
                }
                schemas.push(schema);
            }
            Ok(json!({"schemas": schemas}))
        }
        AutomationAction::ListComponents => {
            let query = args["query"].as_str().unwrap_or("").to_lowercase();
            let category = args["category"].as_str().unwrap_or("").to_lowercase();
            let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
            let catalog = cli::tools::component_catalog(state);
            let matching: Vec<_> = catalog
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| {
                    (query.is_empty() || item.to_string().to_lowercase().contains(&query))
                        && (category.is_empty()
                            || item["category"].as_array().is_some_and(|parts| {
                                parts
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("/")
                                    .to_lowercase()
                                    .starts_with(&category)
                            }))
                })
                .cloned()
                .collect();
            Ok(
                json!({"components":matching.iter().take(limit).collect::<Vec<_>>(),"total":matching.len()}),
            )
        }
        AutomationAction::GetComponent => {
            let id = serde_json::from_value::<crate::components::ComponentId>(
                args["component_id"].clone(),
            )
            .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let component =
                state.components.definitions.get(&id).ok_or_else(|| {
                    DomainError::new("COMPONENT_NOT_FOUND", "Component not found")
                })?;
            let mut metadata = cli::tools::component_catalog(state)
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == json!(id))
                .cloned()
                .ok_or_else(|| DomainError::new("COMPONENT_NOT_FOUND", "Component not found"))?;
            if args["include_nodes"] == true {
                metadata["nodes"] = json!(component.nodes);
            }
            Ok(json!({"component":metadata}))
        }
        AutomationAction::GetComponentInstances => {
            let instances = &state.components.instances;
            let requested = args.get("instance_id").filter(|v| !v.is_null());
            let matches: Vec<_> = instances
                .iter()
                .filter(|instance| requested.is_none_or(|id| id == &json!(instance.id)))
                .collect();
            if requested.is_some() && matches.is_empty() {
                return Err(DomainError::new(
                    "INSTANCE_NOT_FOUND",
                    "Component instance was not found",
                ));
            }
            Ok(json!({"instances":matches,"total":matches.len()}))
        }
        AutomationAction::ProjectSettings => Ok(json!({"settings":{
            "mask_size":state.mask_size,
            "soft_key_size":state.soft_key_size,
            "soft_key_layout":state.soft_key_layout,
            "vt_version":u8::from(state.vt_version),
            "object_id_ranges":state.object_id_ranges.iter().map(|(object_type, range)| json!({"object_type":format!("{object_type:?}"),"start":range.start,"end":range.end})).collect::<Vec<_>>(),
        }})),
        AutomationAction::ValidateDesign => {
            let mut diagnostics = match project.compatibility_diagnostics() {
                Ok(diagnostics) => diagnostics,
                Err(error) => vec![crate::pool_validation::ValidationDiagnostic {
                    severity: crate::pool_validation::ValidationSeverity::Error,
                    message: error.message.clone(),
                    affected_objects: error.object_id.map(|id| vec![id]),
                    code: error.code.clone(),
                }],
            };
            if let Err(error) = project.export_iop() {
                if !diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == error.code)
                {
                    diagnostics.push(crate::pool_validation::ValidationDiagnostic {
                        severity: crate::pool_validation::ValidationSeverity::Error,
                        message: error.message,
                        affected_objects: error.object_id.map(|id| vec![id]),
                        code: error.code,
                    });
                }
            }
            let valid = diagnostics.iter().all(|diagnostic| {
                diagnostic.severity != crate::pool_validation::ValidationSeverity::Error
            });
            Ok(json!({"valid":valid,"diagnostics":diagnostics}))
        }
        AutomationAction::PreviewDesign => {
            let mask_id = args
                .get("mask_id")
                .and_then(Value::as_u64)
                .map(|value| {
                    u16::try_from(value).map_err(|_| {
                        DomainError::new("INVALID_ARGUMENTS", "mask_id must be between 0 and 65534")
                    })
                })
                .transpose()?;
            let preview = cli::preview::render_design_preview(state, mask_id)
                .map_err(|error| DomainError::new("INSPECTION_FAILED", format!("{error:?}")))?;
            Ok(
                json!({"width":preview.width,"height":preview.height,"mask_id":preview.mask_id,"png_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,&preview.image.bytes)}),
            )
        }
        AutomationAction::GetGraphic => {
            let id = args["id"].as_str().ok_or_else(|| {
                DomainError::new(
                    "INVALID_ARGUMENTS",
                    "id must be a graphic UUID or PictureGraphic object ID",
                )
            })?;
            let export = args["export"].as_bool().unwrap_or(false);
            if let Ok(asset_id) = id.parse::<uuid::Uuid>() {
                let asset_id = crate::graphics::GraphicAssetId(asset_id);
                let asset = state.graphics.assets.get(&asset_id).ok_or_else(|| {
                    DomainError::new("GRAPHIC_NOT_FOUND", "Graphic asset was not found")
                })?;
                let mut value = json!({"id":asset.id,"name":asset.name,"format":asset.format,"original_file_name":asset.original_file_name,"description":asset.description,"tags":asset.tags});
                if export {
                    let image = image::load_from_memory(&asset.original_bytes)
                        .map_err(|e| DomainError::new("GRAPHIC_EXPORT_FAILED", e.to_string()))?;
                    let mut png = std::io::Cursor::new(Vec::new());
                    image
                        .write_to(&mut png, image::ImageFormat::Png)
                        .map_err(|e| DomainError::new("GRAPHIC_EXPORT_FAILED", e.to_string()))?;
                    value["png_base64"] = json!(base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        png.into_inner()
                    ));
                }
                Ok(json!({"graphic":value}))
            } else {
                let object_id = id.parse::<u16>().map_err(|_| {
                    DomainError::new(
                        "INVALID_ARGUMENTS",
                        "id must be a graphic UUID or PictureGraphic object ID",
                    )
                })?;
                let object_id =
                    ag_iso_stack::object_pool::ObjectId::new(object_id).map_err(|_| {
                        DomainError::new("INVALID_ARGUMENTS", "Invalid PictureGraphic object ID")
                    })?;
                let object = state.pool.object_by_id(object_id).ok_or_else(|| {
                    DomainError::new("GRAPHIC_NOT_FOUND", "PictureGraphic was not found")
                })?;
                let ag_iso_stack::object_pool::object::Object::PictureGraphic(picture) = object
                else {
                    return Err(DomainError::new(
                        "INVALID_ARGUMENTS",
                        "Object ID is not a PictureGraphic",
                    ));
                };
                let mut value = json!({"id":id,"name":state.object_info.get(&object_id).map(|info|info.get_name(object)).unwrap_or_else(|| format!("PictureGraphic {}", object_id.value())),"type":"PictureGraphic"});
                if export {
                    let png = crate::graphics::preview::picture_png(picture, &state.pool)
                        .map_err(|e| DomainError::new("GRAPHIC_EXPORT_FAILED", e))?;
                    value["png_base64"] = json!(base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        png
                    ));
                }
                Ok(json!({"graphic":value}))
            }
        }
        AutomationAction::ListGraphics => {
            Ok(json!({"graphics":crate::graphics::integration::catalog(
            &state.pool,
            &state.graphics,
            &state.object_info,
            args["query"].as_str().unwrap_or(""), args["status"].as_str().unwrap_or("all"),
            args["limit"].as_u64().unwrap_or(20).clamp(1,50) as usize)}))
        }
        AutomationAction::ResolveColours => {
            let colours: Vec<String> = serde_json::from_value(args["colours"].clone())
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            if colours.is_empty() || colours.len() > 16 {
                return Err(DomainError::new(
                    "INVALID_ARGUMENTS",
                    "Provide 1..16 colour indices, names, or #RRGGBB values",
                ));
            }
            Ok(
                json!({"results":colours.iter().map(|query|match crate::effective_colour::resolve(&state.pool,query) {
                Ok(colour)=>json!({"query":query,"resolved":colour}),Err(message)=>json!({"query":query,"error":message})
            }).collect::<Vec<_>>(),"source":crate::effective_colour::source(&state.pool)}),
            )
        }
        AutomationAction::InspectDesignRegion => {
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let region = cli::design_inspection::inspect_design_region(state, request)
                .map_err(|e| DomainError::new("INSPECTION_FAILED", format!("{e:?}")))?;
            Ok(json!({"region":region}))
        }
    }
}

/// Columns `list_objects` can return, in their default order.
const LIST_COLUMNS: [&str; 4] = ["id", "name", "type", "c_ident"];

fn list_columns(value: &Value) -> Result<Vec<&'static str>, DomainError> {
    let Some(requested) = value.as_array() else {
        if value.is_null() {
            return Ok(LIST_COLUMNS[..3].to_vec());
        }
        return Err(DomainError::new(
            "INVALID_ARGUMENTS",
            "columns must be a list of column names",
        ));
    };
    let mut columns = Vec::new();
    for column in requested {
        let column = column.as_str().map(str::trim).unwrap_or_default();
        let known = LIST_COLUMNS
            .iter()
            .find(|known| **known == column)
            .ok_or_else(|| {
                DomainError::new(
                    "INVALID_ARGUMENTS",
                    format!(
                        "Unknown column '{column}'. Choose from {}",
                        LIST_COLUMNS.join(", ")
                    ),
                )
            })?;
        if !columns.contains(known) {
            columns.push(*known);
        }
    }
    if columns.is_empty() {
        return Err(DomainError::new(
            "INVALID_ARGUMENTS",
            "Provide at least one column",
        ));
    }
    Ok(columns)
}

/// Types the VT finds by type rather than through a parent or reference
/// (ISO 11783-6), so having no inbound reference does not make them unused.
fn is_top_level(object_type: ObjectType) -> bool {
    matches!(
        object_type,
        ObjectType::WorkingSet
            | ObjectType::WorkingSetSpecialControls
            | ObjectType::WindowMask
            | ObjectType::KeyGroup
            | ObjectType::ObjectLabelReferenceList
            | ObjectType::ExternalObjectDefinition
            | ObjectType::AuxiliaryFunctionType1
            | ObjectType::AuxiliaryInputType1
            | ObjectType::AuxiliaryFunctionType2
            | ObjectType::AuxiliaryInputType2
    )
}

struct ObjectUsage {
    /// IDs that another object points to: children, attribute and variable
    /// references, pointer values and allowed pointer targets, event macros,
    /// and macro command targets.
    referenced: std::collections::HashSet<u16>,
    unresolved_macros: Vec<u16>,
}

fn object_usage(state: &cli::AiDesignState) -> ObjectUsage {
    use ag_iso_stack::object_pool::object::Object;
    let types = state
        .pool
        .objects()
        .iter()
        .map(|object| (object.id().value(), object.object_type()))
        .collect();
    let mut referenced = std::collections::HashSet::new();
    let mut unresolved_macros = Vec::new();
    for object in state.pool.objects() {
        let own = object.id().value();
        // Same references the delete operation checks, so an object listed
        // here can be deleted.
        let mut targets: Vec<u16> = object
            .referenced_objects()
            .into_iter()
            .map(|id| id.value())
            .collect();
        if let Some(macros) = crate::operations::operation::macro_refs(object) {
            targets.extend(macros.iter().map(|r| r.macro_id));
        }
        if let Object::Macro(mac) = object {
            match crate::macro_id_remap::macro_object_references(
                &mac.commands,
                state.vt_version,
                &types,
            ) {
                Ok(ids) => targets.extend(ids),
                Err(_) => unresolved_macros.push(own),
            }
        }
        // A pointer may show any of its allowed targets at runtime.
        if let Some(info) = state.object_info.get(&object.id()) {
            targets.extend(info.allowed_references.iter().map(|id| id.value()));
        }
        referenced.extend(targets.into_iter().filter(|id| *id != own));
    }
    unresolved_macros.sort_unstable();
    ObjectUsage {
        referenced,
        unresolved_macros,
    }
}
