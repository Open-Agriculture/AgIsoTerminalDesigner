//! Domain operations exposed through the desktop MCP bridge.
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

pub fn execute(
    state: &AiDesignState,
    pending: Option<&AiProposal>,
    tool: &str,
    args: Value,
) -> Result<DomainResult, DomainError> {
    let args = if args.is_null() { json!({}) } else { args };
    let selected = |id: &str| -> Result<&AiProposal, DomainError> {
        let proposal = pending
            .ok_or_else(|| DomainError::new("stale_proposal", "No proposal is awaiting review"))?;
        if args["proposal_id"].as_str() != Some(id) {
            return Err(DomainError::new(
                "stale_proposal",
                "Proposal ID does not match the staged proposal",
            ));
        }
        Ok(proposal)
    };
    match tool {
        "get_design_context" => {
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("invalid_arguments", e.to_string()))?;
            let context = ai::get_design_context(state, request)
                .map_err(|e| DomainError::new("inspection_failed", format!("{e:?}")))?;
            result(json!({"context": ai::tools::component_design_context(state, context)}))
        }
        "get_objects" => {
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("invalid_arguments", e.to_string()))?;
            let objects = ai::get_objects(state, request)
                .map_err(|e| DomainError::new("inspection_failed", format!("{e:?}")))?;
            result(
                json!({"sources": objects.iter().filter_map(|o| ai::tools::source_context(&state.components, o.object.id)).collect::<Vec<_>>(), "objects": ai::wire::objects(objects)}),
            )
        }
        "get_object_schema" => {
            let names: Vec<&str> = match args.get("object_types").and_then(Value::as_array) {
                Some(types) => types
                    .iter()
                    .map(|v| {
                        v.as_str().ok_or_else(|| {
                            DomainError::new(
                                "invalid_arguments",
                                "object_types must contain strings",
                            )
                        })
                    })
                    .collect::<Result<_, _>>()?,
                None => vec![args["object_type"].as_str().ok_or_else(|| {
                    DomainError::new("invalid_arguments", "Provide object_type or object_types")
                })?],
            };
            if names.is_empty() || names.len() > 10 {
                return Err(DomainError::new(
                    "invalid_arguments",
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
                            "invalid_arguments",
                            format!("Unknown object type: {name}"),
                        )
                    })?;
                let schema = ai::get_object_schema(state, ty)
                    .map_err(|e| DomainError::new("inspection_failed", format!("{e:?}")))?;
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
            .map_err(|e| DomainError::new("invalid_arguments", e.to_string()))?;
            let component = state
                .components
                .definitions
                .get(&id)
                .ok_or_else(|| DomainError::new("invalid_arguments", "Component not found"))?;
            let mut metadata = ai::tools::component_catalog(state)
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == json!(id))
                .cloned()
                .ok_or_else(|| DomainError::new("invalid_arguments", "Component not found"))?;
            if args["include_nodes"] == true {
                metadata["nodes"] = json!(component.nodes);
            }
            result(json!({"component":metadata}))
        }
        "list_graphics" => result(json!({"graphics":crate::graphics::integration::catalog(
            pending.map(|p| &p.canonical_pool).unwrap_or_else(||state.canonical_pool.as_ref().unwrap_or(&state.pool)),
            pending.map(|p| &p.graphics).unwrap_or(&state.graphics),
            pending.map(|p| &p.prepared.object_info).unwrap_or(&state.object_info),
            args["query"].as_str().unwrap_or(""), args["status"].as_str().unwrap_or("all"),
            args["limit"].as_u64().unwrap_or(20).clamp(1,50) as usize)})),
        "resolve_colours" => {
            let queries = args["queries"]
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 16)
                .ok_or_else(|| DomainError::new("invalid_arguments", "Provide 1..16 queries"))?;
            let pool = pending.map(|p| &p.prepared.pool).unwrap_or(&state.pool);
            result(
                json!({"results":queries.iter().map(|q| match ai::visual_intent::resolve(pool,q) {
                Ok(c) => json!({"query":q,"resolved":c}), Err(e) => json!({"query":q,"diagnostic":e})
            }).collect::<Vec<_>>(),"source":crate::effective_colour::source(pool)}),
            )
        }
        "inspect_design_region" => {
            let request = serde_json::from_value(args)
                .map_err(|e| DomainError::new("invalid_arguments", e.to_string()))?;
            let region = ai::design_inspection::inspect_design_region(state, request)
                .map_err(|e| DomainError::new("inspection_failed", format!("{e:?}")))?;
            result(json!({"region":region}))
        }
        "analyze_design" => {
            let proposal = match args["scope"].as_str() {
                Some("proposal") => Some(pending.ok_or_else(|| {
                    DomainError::new("stale_proposal", "No proposal is awaiting review")
                })?),
                Some("active_mask") => None,
                None => pending,
                Some(_) => {
                    return Err(DomainError::new(
                        "invalid_arguments",
                        "scope must be active_mask or proposal",
                    ))
                }
            };
            let scope = if proposal.is_some() {
                ai::DesignAnalysisScope::Proposal
            } else {
                ai::DesignAnalysisScope::ActiveMask
            };
            let analysis = ai::analyze_design(state, proposal, scope)
                .map_err(|e| DomainError::new("inspection_failed", format!("{e:?}")))?;
            result(json!({"analysis":analysis}))
        }
        "propose_transaction" => {
            let base = match pending {
                Some(p) => Some(selected(&p.proposal_id)?),
                None if args.get("proposal_id").is_some() => {
                    return Err(DomainError::new(
                        "stale_proposal",
                        "No proposal matches this ID",
                    ))
                }
                None => None,
            };
            let mut tx_value = args.get("transaction").cloned().unwrap_or(args);
            if !tx_value.is_object() {
                return Err(DomainError::new(
                    "invalid_arguments",
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
                    .map_err(|e| DomainError::new("validation_failed", format!("{e:?}")))?;
            let tx = serde_json::from_value(normalized)
                .map_err(|e| DomainError::new("invalid_arguments", e.to_string()))?;
            let proposal = match base {
                Some(p) => ai::propose_refinement(state, p, tx),
                None => ai::propose_transaction(state, tx),
            }
            .map_err(|e| {
                DomainError::new(
                    "validation_failed",
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
                        "stale_proposal",
                        "No proposal matches this ID",
                    ))
                }
                None => None,
            };
            if base.is_some_and(|p| !p.transaction.operations.is_empty()) {
                return Err(DomainError::new(
                    "validation_failed",
                    "Apply the primitive proposal before staging component actions",
                ));
            }
            let source = base.map(|p| &p.components).unwrap_or(&state.components);
            let mut intents = base
                .map(|p| p.component_intents.clone())
                .unwrap_or_default();
            intents.extend(
                ai::tools::parse_component_actions(source, args["actions"].clone())
                    .map_err(|e| DomainError::new("invalid_arguments", e))?,
            );
            let proposal =
                ai::proposal::propose_component_intents(state, intents).map_err(|e| {
                    DomainError::new(
                        "validation_failed",
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
                        DomainError::new("stale_proposal", "No proposal is awaiting review")
                    })?
                    .proposal_id
                    .as_str(),
            )?;
            let preview = ai::render_proposal_preview(state, proposal)
                .map_err(|e| DomainError::new("inspection_failed", format!("{e:?}")))?;
            result(
                json!({"proposal_id":preview.proposal_id,"width":preview.width,"height":preview.height,"mask_id":preview.mask_id,"diagnostics":preview.diagnostics,"colour_summary":preview.colour_summary,"png_base64":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,&preview.image.bytes)}),
            )
        }
        "proposal_status" => result(
            json!({"proposal":pending.map(|p|json!({"proposal_id":p.proposal_id,"base_revision":p.base_revision,"summary":{"operation_count":p.summary.operation_count,"affected_objects":p.summary.affected_objects},"refinement_round":p.refinement_round}))}),
        ),
        _ => Err(DomainError::new(
            "unsupported_tool",
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
}
