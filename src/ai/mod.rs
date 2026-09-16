//! Provider-neutral AI domain layer. It only reads snapshots and proposes transactions.
pub mod backend;
pub mod debug_trace;
pub use debug_trace::{AiTokenUsage, AiUsageStats};
pub mod conversation;
pub mod proposal;
pub mod preview;
pub mod design_analysis;
pub mod providers;
pub mod protocol;
pub mod schema;
pub mod snapshot;
pub mod tools;
#[cfg(not(target_arch = "wasm32"))]
pub mod ui;

use crate::ObjectInfo;
use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectId, ObjectPool};
use std::collections::HashMap;
pub type ProjectRevision = u64;
#[derive(Debug, Clone)]
pub struct AiDesignState {
    pub revision: ProjectRevision,
    pub pool: ObjectPool,
    pub object_info: HashMap<ObjectId, ObjectInfo>,
    pub vt_version: VtVersion,
    pub mask_size: u16,
    pub selected: Option<ObjectId>,
}
pub use proposal::{propose_transaction, propose_refinement, AiProposal, AiProposalSummary};
pub use backend::{AiProvider, AiProviderError, AiProviderFuture, AiProviderResponse, AiSettings, AiTurnRequest};
pub use conversation::{AiContent, AiImage, AiMessage, AiRole, AiToolCall, AiToolResult, Conversation};
pub use schema::{get_object_schema, ObjectSchemaSnapshot, PropertySchemaSnapshot};
pub use snapshot::*;
pub use tools::{get_design_context, get_objects, AiDomainTool, AiToolError, GetObjectsRequest};
pub use preview::{render_proposal_preview, render_preview_ui, AiPreview, PreviewError, RenderedPreview};
pub use design_analysis::{analyze_design, DesignAnalysis, DesignAnalysisError, DesignAnalysisScope, DesignDiagnostic, DesignDiagnosticSeverity, DiagnosticRegion};
#[cfg(test)]
mod efficiency_tests;

mod wire;
