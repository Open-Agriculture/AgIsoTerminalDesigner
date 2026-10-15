//! Typed automation commands shared by transport and domain dispatch.

use super::ProjectRevision;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Closed command surface shared by native clients and the bridge.
macro_rules! automation_actions {
    ($($variant:ident => $name:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum AutomationAction {
            $(#[serde(rename = $name)] $variant),*
        }
        impl AutomationAction {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $name),* }
            }
        }
        impl std::str::FromStr for AutomationAction {
            type Err = String;
            fn from_str(name: &str) -> Result<Self, Self::Err> {
                match name {
                    $($name => Ok(Self::$variant),)*
                    _ => Err(format!("Unsupported automation action: {name}")),
                }
            }
        }
        impl std::fmt::Display for AutomationAction {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    }
}
automation_actions! {
    EditProject => "edit_project",
    ExportArtifacts => "export_artifacts",
    ExportObjectBytes => "export_object_bytes",
    GetComponent => "get_component",
    GetComponentInstances => "get_component_instances",
    GetDesignContext => "get_design_context",
    GetGraphic => "get_graphic",
    GetObjectSchema => "get_object_schema",
    GetObjectTypes => "get_object_types",
    GetObjects => "get_objects",
    GetProjectStatus => "get_project_status",
    InspectDesignRegion => "inspect_design_region",
    ListComponents => "list_components",
    ListGraphics => "list_graphics",
    ListObjects => "list_objects",
    PreviewDesign => "preview_design",
    ProjectSettings => "project_settings",
    ResolveColours => "resolve_colours",
    ValidateDesign => "validate_design",
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<ProjectRevision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<ProjectRevision>,
}

/// One provider-neutral action sent through the authenticated editor bridge.
/// Status requests may omit project identity/revision; project commands require both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRequest {
    #[serde(default)]
    pub project_instance_id: Option<String>,
    #[serde(default)]
    pub revision: Option<ProjectRevision>,
    pub action: AutomationAction,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationResponse {
    pub success: bool,
    pub editor_instance_id: String,
    pub project_instance_id: String,
    pub revision: Option<ProjectRevision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

#[derive(Debug, Clone)]
pub struct AutomationClientError {
    pub code: String,
    pub message: String,
    pub expected_revision: Option<ProjectRevision>,
    pub current_revision: Option<ProjectRevision>,
}

impl std::fmt::Display for AutomationClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AutomationClientError {}

impl From<WireError> for AutomationClientError {
    fn from(error: WireError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            expected_revision: error.expected_revision,
            current_revision: error.current_revision,
        }
    }
}
