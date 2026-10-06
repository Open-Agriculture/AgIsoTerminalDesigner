#[cfg(not(target_arch = "wasm32"))]
use crate::ai::mcp_bridge::{invoke_runtime_tool, select_runtime};
#[cfg(not(target_arch = "wasm32"))]
use rmcp::transport::stdio;
#[cfg(not(target_arch = "wasm32"))]
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    tool, tool_router, ServiceExt,
};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::{json, Value};
#[cfg(not(target_arch = "wasm32"))]
use std::env;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct DesignerMcp {
    instance_id: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl DesignerMcp {
    fn tool_schema(name: &str) -> rmcp::model::JsonObject {
        if matches!(name, "get_project_status" | "proposal_status") {
            return rmcp::model::object(json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }));
        }
        if name == "preview_proposal" {
            return rmcp::model::object(json!({
                "type":"object",
                "properties":{"proposal_id":{"type":"string"}},
                "required":["proposal_id"],
                "additionalProperties":false
            }));
        }
        if name == "inspect_design_region" {
            return rmcp::model::object(json!({
                "type":"object",
                "properties":{
                    "root_id":{"type":"integer","minimum":0,"maximum":65534},
                    "region":{"type":"string","enum":["top","bottom","left","right","center","top_left","top_right","bottom_left","bottom_right","all"]},
                    "types":{"type":"array","items":{"type":"string"}},
                    "purpose":{"type":"string"},
                    "max_results":{"type":"integer","minimum":1,"maximum":30}
                },
                "required":["root_id"],
                "additionalProperties":false
            }));
        }
        let definition = crate::ai::tools::tool_definitions()
            .into_iter()
            .find(|definition| definition["name"].as_str() == Some(name));
        let mut schema = definition
            .map(|definition| definition["parameters"].clone())
            .unwrap_or_else(|| json!({"type":"object"}));
        // The previous model provider used strict function schemas, encoding
        // optional fields as required nullable fields. MCP callers can omit them.
        let optional: Vec<String> = schema["properties"]
            .as_object()
            .into_iter()
            .flat_map(|properties| properties.iter())
            .filter_map(|(name, property)| {
                property["anyOf"]
                    .as_array()
                    .filter(|variants| {
                        variants.len() == 2 && variants.iter().any(|v| v["type"] == "null")
                    })
                    .map(|_| name.clone())
            })
            .collect();
        for name in &optional {
            let variants = schema["properties"][name]["anyOf"].as_array().unwrap();
            let actual = variants
                .iter()
                .find(|v| v["type"] != "null")
                .unwrap()
                .clone();
            schema["properties"][name] = actual;
        }
        if let Some(required) = schema["required"].as_array_mut() {
            required.retain(|name| {
                name.as_str()
                    .is_none_or(|name| !optional.iter().any(|optional| optional == name))
            });
        }
        if name == "get_object_schema" {
            schema["anyOf"] = json!([{"required":["object_type"]},{"required":["object_types"]}]);
        }
        if matches!(name, "propose_transaction" | "propose_component_actions") {
            schema["properties"]["proposal_id"] = json!({"type":"string","description":"Required when refining the currently staged proposal; omit only when proposing the first change."});
        }
        rmcp::model::object(schema)
    }

    fn invoke(&self, name: &str, args: Value) -> Json<Value> {
        let response = invoke_runtime_tool(Some(&self.instance_id), name, args)
            .unwrap_or_else(|error| json!({"success":false,"error":error}));
        Json(response)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tool_router(server_handler)]
impl DesignerMcp {
    #[tool(
        input_schema = Self::tool_schema("get_project_status"),
        description = "Return editor connection state, whether a project is open, its revision, and staged proposal status."
    )]
    async fn get_project_status(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("get_project_status", args)
    }

    #[tool(
        input_schema = Self::tool_schema("get_design_context"),
        description = "Read the currently open project's status and design context. This reads live unsaved editor state."
    )]
    async fn get_design_context(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("get_design_context", args)
    }

    #[tool(
        input_schema = Self::tool_schema("get_objects"),
        description = "Inspect selected project objects and requested properties, relationships, children, or parents."
    )]
    async fn get_objects(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("get_objects", args)
    }

    #[tool(input_schema = Self::tool_schema("get_object_schema"), description = "Read writable properties and relationship rules for object types.")]
    async fn get_object_schema(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("get_object_schema", args)
    }

    #[tool(input_schema = Self::tool_schema("list_components"), description = "List reusable components in the open project.")]
    async fn list_components(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("list_components", args)
    }

    #[tool(input_schema = Self::tool_schema("get_component"), description = "Read a reusable component and its parameters.")]
    async fn get_component(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("get_component", args)
    }

    #[tool(input_schema = Self::tool_schema("list_graphics"), description = "List reusable graphics and available graphic assets.")]
    async fn list_graphics(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("list_graphics", args)
    }

    #[tool(
        input_schema = Self::tool_schema("resolve_colours"),
        description = "Resolve requested semantic colours against the effective project palette."
    )]
    async fn resolve_colours(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("resolve_colours", args)
    }

    #[tool(input_schema = Self::tool_schema("inspect_design_region"), description = "Inspect a region of the design for nearby structure and style.")]
    async fn inspect_design_region(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("inspect_design_region", args)
    }

    #[tool(
        input_schema = Self::tool_schema("analyze_design"),
        description = "Analyze design quality and diagnostics for the current project or staged proposal."
    )]
    async fn analyze_design(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("analyze_design", args)
    }

    #[tool(
        input_schema = Self::tool_schema("propose_transaction"),
        description = "Stage a validated transaction for user review. This tool never applies or saves changes. Include proposal_id when refining an existing proposal."
    )]
    async fn propose_transaction(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("propose_transaction", args)
    }

    #[tool(
        input_schema = Self::tool_schema("propose_component_actions"),
        description = "Stage validated component actions for user review. This tool never applies or saves changes. Include proposal_id when refining an existing proposal."
    )]
    async fn propose_component_actions(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("propose_component_actions", args)
    }

    #[tool(input_schema = Self::tool_schema("preview_proposal"), description = "Render a preview of the staged proposal. Include its proposal_id.")]
    async fn preview_proposal(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("preview_proposal", args)
    }

    #[tool(input_schema = Self::tool_schema("proposal_status"), description = "Read staged proposal status. This tool never applies or saves changes.")]
    async fn proposal_status(&self, Parameters(args): Parameters<Value>) -> Json<Value> {
        self.invoke("proposal_status", args)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut explicit_instance = env::var("AG_ISO_EDITOR_INSTANCE_ID").ok();
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--instance" => {
                explicit_instance = Some(
                    arguments
                        .next()
                        .ok_or("--instance requires an editor instance ID")?,
                )
            }
            "--help" | "-h" => {
                println!("Usage: ag-iso-terminal-designer-mcp [--instance <editor-instance-id>]");
                return Ok(());
            }
            other => return Err(format!("Unknown argument: {other}").into()),
        }
    }
    let (_, runtime) = select_runtime(explicit_instance.as_deref())?;
    let service = DesignerMcp {
        instance_id: runtime.instance_id,
    }
    .serve(stdio())
    .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn discovery_schemas_preserve_optional_inputs() {
        let context = DesignerMcp::tool_schema("get_design_context");
        assert!(context
            .get("properties")
            .and_then(|p| p.get("scope"))
            .is_some());
        assert!(!context
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.contains(&json!("scope"))));
        let objects = DesignerMcp::tool_schema("get_objects");
        assert!(objects
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| required.contains(&json!("ids"))));
    }
}
