//! Design inspection and staged proposal domain layer.
#[cfg(not(target_arch = "wasm32"))]
pub mod automation_bridge;
pub mod automation_service;
#[cfg(not(target_arch = "wasm32"))]
pub mod automation_trace;
pub mod design_analysis;
pub mod design_inspection;
pub mod preview;
pub mod proposal;
pub mod schema;
pub mod snapshot;
pub mod tools;

use crate::ObjectInfo;
use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectId, ObjectPool};
use std::collections::HashMap;
pub type ProjectRevision = u64;
#[derive(Debug, Clone)]
pub struct AiDesignState {
    pub revision: ProjectRevision,
    pub canonical_pool: Option<ObjectPool>,
    pub components: crate::components::ComponentSource,
    pub graphics: crate::graphics::GraphicLibrary,
    pub pool: ObjectPool,
    pub object_info: HashMap<ObjectId, ObjectInfo>,
    pub vt_version: VtVersion,
    pub mask_size: u16,
    pub selected: Option<ObjectId>,
}
pub use design_analysis::{
    analyze_design, DesignAnalysis, DesignAnalysisError, DesignAnalysisScope, DesignDiagnostic,
    DesignDiagnosticSeverity, DiagnosticRegion,
};
pub use preview::AiImage;
pub use preview::{
    render_preview_ui, render_proposal_preview, AiPreview, PreviewError, RenderedPreview,
};
pub use proposal::{propose_refinement, propose_transaction, AiProposal, AiProposalSummary};
pub use schema::{get_object_schema, ObjectSchemaSnapshot, PropertySchemaSnapshot};
pub use snapshot::*;
pub use tools::{get_design_context, get_objects, AiToolError, GetObjectsRequest};

pub(crate) mod wire;

pub mod icon_recipes;
pub mod visual_diagnostics;
pub mod visual_intent;
