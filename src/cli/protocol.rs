//! Closed set of direct automation actions used by the CLI domain dispatcher.

use serde::{Deserialize, Serialize};

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
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    }
}

automation_actions! {
    ExportObjectBytes => "export_object_bytes",
    GetComponent => "get_component",
    GetComponentInstances => "get_component_instances",
    GetDesignContext => "get_design_context",
    GetGraphic => "get_graphic",
    GetObjectSchema => "get_object_schema",
    GetObjectTypes => "get_object_types",
    GetObjects => "get_objects",
    InspectDesignRegion => "inspect_design_region",
    ListComponents => "list_components",
    ListGraphics => "list_graphics",
    ListObjects => "list_objects",
    PreviewDesign => "preview_design",
    ProjectSettings => "project_settings",
    ResolveColours => "resolve_colours",
    ValidateDesign => "validate_design",
}
