//! Domain operations exposed through the authenticated automation bridge.
use crate::ai::{self, AiDesignState, AiProposal};
use ag_iso_stack::object_pool::ObjectType;
use serde_json::{json, Value};

pub struct DomainResult {
    pub value: Value,
    pub proposal: Option<AiProposal>,
}

#[derive(Debug)]
pub struct DomainError {
    pub code: &'static str,
    pub message: String,
}

/// Compare two explicitly selected project states for the bridge's draft and
/// proposal review responses.
pub fn diff_states(before: &AiDesignState, after: &AiDesignState) -> Value {
    ai::design_analysis::diff_states(before, after)
}

impl DomainError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn result(value: Value) -> Result<DomainResult, DomainError> {
    Ok(DomainResult {
        value,
        proposal: None,
    })
}

fn inspection_error(error: ai::AiToolError) -> DomainError {
    match error {
        ai::AiToolError::ObjectNotFound(id) => {
            DomainError::new("OBJECT_NOT_FOUND", format!("Object {id} was not found"))
        }
        ai::AiToolError::RequestTooLarge => DomainError::new(
            "INVALID_ARGUMENTS",
            "Object request exceeds the supported limit",
        ),
        ai::AiToolError::InvalidScope => {
            DomainError::new("INVALID_ARGUMENTS", "Invalid design inspection scope")
        }
    }
}

pub fn execute(
    state: &AiDesignState,
    pending: Option<&AiProposal>,
    tool: &str,
    args: Value,
) -> Result<DomainResult, DomainError> {
    let mut args = if args.is_null() { json!({}) } else { args };
    // View selection belongs to the bridge, not the domain request DTOs.
    if let Some(fields) = args.as_object_mut() {
        fields.remove("view");
        fields.remove("draft_id");
        fields.remove("generation");
        if !matches!(
            tool,
            "preview_proposal" | "propose_transaction" | "propose_component_actions"
        ) {
            fields.remove("proposal_id");
        }
    }
    let selected = |id: &str| -> Result<&AiProposal, DomainError> {
        let proposal = pending
            .ok_or_else(|| DomainError::new("PROPOSAL_STALE", "No proposal is awaiting review"))?;
        if args["proposal_id"].as_str() != Some(id) {
            return Err(DomainError::new(
                "PROPOSAL_STALE",
                "Proposal ID does not match the staged proposal",
            ));
        }
        Ok(proposal)
    };
    match tool {
        "export_object_bytes" => {
            let target: crate::ObjectTarget = serde_json::from_value(args["object"].clone())
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let id = crate::operations::resolve_target(&target, &state.pool, &state.object_info)
                .map_err(|e| DomainError::new("OBJECT_NOT_FOUND", e.to_string()))?;
            let property = args["property"]
                .as_str()
                .ok_or_else(|| DomainError::new("INVALID_ARGUMENTS", "Provide a property name"))?;
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
            result(
                json!({"object_id":id.value(),"property":property,"bytes_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes)}),
            )
        }
        "get_object_types" => result(
            json!({"types":ObjectType::values().into_iter().map(|ty|json!({
            "type":format!("{ty:?}"),"supported_for_target":crate::vt_compatibility::supported_object_type(ty,state.vt_version)
        })).collect::<Vec<_>>()}),
        ),
        "get_design_context" => {
            if let Some(scope) = args["scope"].as_str() {
                args["scope"] = json!(match scope {
                    "active_mask" => "ActiveMask",
                    "selection" => "Selection",
                    "pool" | "entire_pool" => "EntirePool",
                    other => other,
                });
            }
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let context = ai::get_design_context(state, request).map_err(inspection_error)?;
            result(json!({"context": ai::tools::component_design_context(state, context)}))
        }
        "get_objects" => {
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
                        .map_err(|e| DomainError::new("OBJECT_NOT_FOUND", e.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                args["ids"] = json!(ids);
                args.as_object_mut().unwrap().remove("targets");
            }
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let objects = ai::get_objects(state, request).map_err(inspection_error)?;
            let sources = objects
                .iter()
                .filter_map(|o| ai::tools::source_context(&state.components, o.object.id))
                .collect::<Vec<_>>();
            let mut values = ai::wire::objects(objects);
            for value in values.as_array_mut().unwrap() {
                if let Some(object) = value["id"]
                    .as_u64()
                    .and_then(|id| u16::try_from(id).ok())
                    .and_then(|id| ag_iso_stack::object_pool::ObjectId::new(id).ok())
                    .and_then(|id| state.pool.object_by_id(id))
                {
                    if let ag_iso_stack::object_pool::object::Object::ObjectLabelReferenceList(
                        labels,
                    ) = object
                    {
                        value["labels"] = json!(labels.object_labels);
                    }
                    if object.object_type() == ObjectType::ObjectPointer {
                        value["allowed_references"] = json!(state
                            .object_info
                            .get(&object.id())
                            .map(|info| info
                                .allowed_references
                                .iter()
                                .map(|id| id.value())
                                .collect::<Vec<_>>())
                            .unwrap_or_default());
                    }
                }
            }
            result(json!({"sources":sources,"objects":values}))
        }
        "list_objects" => {
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
            let parent_children = parent.map(|parent_id| {
                ag_iso_stack::object_pool::ObjectId::new(parent_id)
                    .ok()
                    .map(|root| {
                        ai::snapshot::layout_instances(
                            &state.pool,
                            &state.object_info,
                            Some(root),
                            ai::snapshot::MAX_SNAPSHOT_DEPTH,
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
                matches.push(json!({"id":id,"name":name,"type":object_type}));
            }
            matches.sort_by_key(|item| item["id"].as_u64().unwrap_or(u64::MAX));
            let total = matches.len();
            let objects = matches
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            let has_more = offset.saturating_add(objects.len()) < total;
            result(
                json!({"objects":objects,"total":total,"offset":offset,"limit":limit,"has_more":has_more}),
            )
        }
        "get_object_schema" => {
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
                let schema = ai::get_object_schema(state, ty)
                    .map_err(|e| DomainError::new("INSPECTION_FAILED", format!("{e:?}")))?;
                schemas.push(ai::wire::schema(schema));
            }
            result(json!({"schemas": schemas}))
        }
        "list_components" => {
            let query = args["query"].as_str().unwrap_or("").to_lowercase();
            let category = args["category"].as_str().unwrap_or("").to_lowercase();
            let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
            let catalog = ai::tools::component_catalog(state);
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
            result(
                json!({"components":matching.iter().take(limit).collect::<Vec<_>>(),"total":matching.len()}),
            )
        }
        "get_component" => {
            let id = serde_json::from_value::<crate::components::ComponentId>(
                args["component_id"].clone(),
            )
            .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let component =
                state.components.definitions.get(&id).ok_or_else(|| {
                    DomainError::new("COMPONENT_NOT_FOUND", "Component not found")
                })?;
            let mut metadata = ai::tools::component_catalog(state)
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == json!(id))
                .cloned()
                .ok_or_else(|| DomainError::new("COMPONENT_NOT_FOUND", "Component not found"))?;
            if args["include_nodes"] == true {
                metadata["nodes"] = json!(component.nodes);
            }
            result(json!({"component":metadata}))
        }
        "get_component_instances" => {
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
            result(json!({"instances":matches,"total":matches.len()}))
        }
        "project_settings" => result(json!({"settings":{
            "mask_size":state.mask_size,
            "soft_key_size":state.soft_key_size,
            "soft_key_layout":state.soft_key_layout,
            "vt_version":u8::from(state.vt_version),
            "object_id_ranges":state.object_id_ranges.iter().map(|(object_type, range)| json!({"object_type":format!("{object_type:?}"),"start":range.start,"end":range.end})).collect::<Vec<_>>(),
        }})),
        "validate_design" => {
            let project = crate::EditorProject::from_ai_source(state);
            let materialized = project.materialize_shared();
            let mut diagnostics = match &materialized {
                Ok(materialized) => crate::pool_validation::validate_pool_state_for_version(
                    &materialized.pool,
                    state.vt_version,
                ),
                Err(error) => vec![crate::pool_validation::ValidationDiagnostic {
                    severity: crate::pool_validation::ValidationSeverity::Error,
                    message: error.message.clone(),
                    affected_objects: error.object_id.map(|id| vec![id]),
                    code: error.code.clone(),
                }],
            };
            let compatibility_pool = materialized
                .as_ref()
                .map(|materialized| &materialized.pool)
                .unwrap_or(&state.pool);
            let compatibility = crate::vt_compatibility::validate_pool_compatibility(
                compatibility_pool,
                state.vt_version,
            );
            diagnostics.extend(compatibility);
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
            result(json!({"valid":valid,"diagnostics":diagnostics}))
        }
        "preview_design" => {
            let mask_id = args
                .get("mask_id")
                .and_then(Value::as_u64)
                .map(|value| {
                    u16::try_from(value).map_err(|_| {
                        DomainError::new("INVALID_ARGUMENTS", "mask_id must be between 0 and 65534")
                    })
                })
                .transpose()?;
            let preview = ai::preview::render_design_preview(state, mask_id)
                .map_err(|error| DomainError::new("INSPECTION_FAILED", format!("{error:?}")))?;
            result(
                json!({"width":preview.width,"height":preview.height,"mask_id":preview.mask_id,"diagnostics":preview.diagnostics,"colour_summary":preview.colour_summary,"png_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,&preview.image.bytes)}),
            )
        }
        "get_graphic" => {
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
                result(json!({"graphic":value}))
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
                result(json!({"graphic":value}))
            }
        }
        "list_graphics" => result(json!({"graphics":crate::graphics::integration::catalog(
            &state.pool,
            &state.graphics,
            &state.object_info,
            args["query"].as_str().unwrap_or(""), args["status"].as_str().unwrap_or("all"),
            args["limit"].as_u64().unwrap_or(20).clamp(1,50) as usize)})),
        "resolve_colours" => {
            let queries = if let Some(queries) = args["queries"].as_array() {
                queries.clone()
            } else if let Some(indexes) = args["indexes"].as_array() {
                indexes.clone()
            } else {
                return Err(DomainError::new(
                    "INVALID_ARGUMENTS",
                    "Provide 1..16 queries or indexes",
                ));
            };
            if queries.is_empty() || queries.len() > 16 {
                return Err(DomainError::new(
                    "INVALID_ARGUMENTS",
                    "Provide 1..16 queries or indexes",
                ));
            }
            let pool = &state.pool;
            result(
                json!({"results":queries.iter().map(|q| match ai::visual_intent::resolve(pool,q) {
                Ok(c) => json!({"query":q,"resolved":c}), Err(e) => json!({"query":q,"diagnostic":e})
            }).collect::<Vec<_>>(),"source":crate::effective_colour::source(pool)}),
            )
        }
        "inspect_design_region" => {
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let region = ai::design_inspection::inspect_design_region(state, request)
                .map_err(|e| DomainError::new("INSPECTION_FAILED", format!("{e:?}")))?;
            result(json!({"region":region}))
        }
        "analyze_design" => {
            let scope = match args["scope"].as_str() {
                Some("proposal") => ai::DesignAnalysisScope::Proposal,
                Some("active_mask") | None => ai::DesignAnalysisScope::ActiveMask,
                Some(_) => {
                    return Err(DomainError::new(
                        "INVALID_ARGUMENTS",
                        "scope must be active_mask or proposal",
                    ))
                }
            };
            let analysis = ai::analyze_design(state, None, scope)
                .map_err(|e| DomainError::new("INSPECTION_FAILED", format!("{e:?}")))?;
            result(json!({"analysis":analysis}))
        }
        "propose_transaction" => {
            let base = match pending {
                Some(p) => Some(selected(&p.proposal_id)?),
                None if args.get("proposal_id").is_some() => {
                    return Err(DomainError::new(
                        "PROPOSAL_STALE",
                        "No proposal matches this ID",
                    ))
                }
                None => None,
            };
            let mut tx_value = args.get("transaction").cloned().unwrap_or(args);
            if !tx_value.is_object() {
                return Err(DomainError::new(
                    "INVALID_ARGUMENTS",
                    "transaction must be an object with an operations array",
                ));
            }
            tx_value["schema_version"] = json!(crate::operations::CURRENT_OPERATION_SCHEMA_VERSION);
            let candidate = base.map(|p| AiDesignState {
                pool: p.prepared.pool.clone(),
                object_info: p.prepared.object_info.clone(),
                ..state.clone()
            });
            let (normalized, colours) =
                ai::visual_intent::compile(candidate.as_ref().unwrap_or(state), tx_value)
                    .map_err(|e| DomainError::new("VALIDATION_FAILED", format!("{e:?}")))?;
            let tx = serde_json::from_value(normalized)
                .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e.to_string()))?;
            let proposal = match base {
                Some(p) => ai::propose_refinement(state, p, tx),
                None => ai::propose_transaction(state, tx),
            }
            .map_err(|e| {
                DomainError::new(
                    "VALIDATION_FAILED",
                    serde_json::to_string(&e).unwrap_or_else(|_| format!("{e:?}")),
                )
            })?;
            let analysis =
                ai::analyze_design(state, Some(&proposal), ai::DesignAnalysisScope::Proposal).ok();
            let value = json!({"proposal_id":proposal.proposal_id,"base_revision":proposal.base_revision,"summary":{"operation_count":proposal.summary.operation_count,"affected_objects":proposal.summary.affected_objects},"refinement_round":proposal.refinement_round,"diagnostics":analysis.map(|a|a.diagnostics),"colour_resolutions":colours});
            Ok(DomainResult {
                value,
                proposal: Some(proposal),
            })
        }
        "propose_component_actions" => {
            let base = match pending {
                Some(p) => Some(selected(&p.proposal_id)?),
                None if args.get("proposal_id").is_some() => {
                    return Err(DomainError::new(
                        "PROPOSAL_STALE",
                        "No proposal matches this ID",
                    ))
                }
                None => None,
            };
            if base.is_some_and(|p| !p.transaction.operations.is_empty()) {
                return Err(DomainError::new(
                    "VALIDATION_FAILED",
                    "Apply the primitive proposal before staging component actions",
                ));
            }
            let source = base.map(|p| &p.components).unwrap_or(&state.components);
            let mut intents = base
                .map(|p| p.component_intents.clone())
                .unwrap_or_default();
            intents.extend(
                ai::tools::parse_component_actions(source, args["actions"].clone())
                    .map_err(|e| DomainError::new("INVALID_ARGUMENTS", e))?,
            );
            let proposal =
                ai::proposal::propose_component_intents(state, intents).map_err(|e| {
                    DomainError::new(
                        "VALIDATION_FAILED",
                        serde_json::to_string(&e).unwrap_or_else(|_| format!("{e:?}")),
                    )
                })?;
            let value = json!({"proposal_id":proposal.proposal_id,"base_revision":proposal.base_revision,"summary":{"operation_count":proposal.summary.operation_count,"affected_objects":proposal.summary.affected_objects},"component_instances":proposal.components.instances});
            Ok(DomainResult {
                value,
                proposal: Some(proposal),
            })
        }
        "preview_proposal" => {
            let proposal = selected(
                pending
                    .ok_or_else(|| {
                        DomainError::new("PROPOSAL_STALE", "No proposal is awaiting review")
                    })?
                    .proposal_id
                    .as_str(),
            )?;
            let preview = ai::render_proposal_preview(state, proposal)
                .map_err(|e| DomainError::new("INSPECTION_FAILED", format!("{e:?}")))?;
            result(
                json!({"proposal_id":preview.proposal_id,"width":preview.width,"height":preview.height,"mask_id":preview.mask_id,"diagnostics":preview.diagnostics,"colour_summary":preview.colour_summary,"png_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,&preview.image.bytes)}),
            )
        }
        "proposal_status" => result(
            json!({"proposal":pending.map(|p|json!({"proposal_id":p.proposal_id,"base_revision":p.base_revision,"summary":{"operation_count":p.summary.operation_count,"affected_objects":p.summary.affected_objects},"refinement_round":p.refinement_round}))}),
        ),
        _ => Err(DomainError::new(
            "UNSUPPORTED_OPERATION",
            format!("Unsupported tool: {tool}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_object, EditorProject};
    use ag_iso_stack::object_pool::{ObjectPool, ObjectType};

    fn project() -> EditorProject {
        let mut pool = ObjectPool::default();
        let mut button = default_object(ObjectType::Button);
        button.mut_id().set_value(10).unwrap();
        pool.add(button);
        EditorProject::from(pool)
    }

    #[test]
    fn proposal_requires_matching_id_and_does_not_mutate_live_project() {
        let project = project();
        let original = project.get_pool().as_iop();
        let state = project.ai_design_state();
        assert!(execute(
            &state,
            None,
            "propose_transaction",
            json!({"transaction": []})
        )
        .is_err());
        let transaction = json!({"operations":[{"type":"SetProperty","data":{"object":{"id":10},"property":"width","value":120}}]});
        let first = execute(&state, None, "propose_transaction", transaction.clone()).unwrap();
        let proposal = first.proposal.unwrap();
        assert_eq!(project.get_pool().as_iop(), original);
        assert!(execute(
            &state,
            Some(&proposal),
            "propose_transaction",
            transaction.clone()
        )
        .is_err());
        assert!(execute(
            &state,
            Some(&proposal),
            "preview_proposal",
            json!({"proposal_id":"wrong"})
        )
        .is_err());
        let status = execute(&state, Some(&proposal), "proposal_status", json!({})).unwrap();
        assert_eq!(
            status.value["proposal"]["proposal_id"],
            proposal.proposal_id
        );
    }

    #[test]
    fn selected_state_inspection_and_diff_are_consistent() {
        let state = project().ai_design_state();
        let listed = execute(
            &state,
            None,
            "list_objects",
            json!({"query":"button","object_type":"Button","offset":0,"limit":10}),
        )
        .unwrap();
        assert_eq!(listed.value["total"], 1);
        assert_eq!(listed.value["objects"][0]["id"], 10);

        let settings = execute(&state, None, "project_settings", json!({})).unwrap();
        assert_eq!(settings.value["settings"]["mask_size"], state.mask_size);
        let instances = execute(&state, None, "get_component_instances", json!({})).unwrap();
        assert_eq!(instances.value["total"], 0);

        assert_eq!(diff_states(&state, &state)["changed"], false);
    }
}
