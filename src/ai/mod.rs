//! Provider-neutral AI domain layer. It only reads snapshots and proposes transactions.
pub mod proposal;
pub mod protocol;
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
    pub pool: ObjectPool,
    pub object_info: HashMap<ObjectId, ObjectInfo>,
    pub vt_version: VtVersion,
    pub mask_size: u16,
    pub selected: Option<ObjectId>,
}
pub use proposal::{propose_transaction, AiProposal, AiProposalSummary};
pub use schema::{get_object_schema, ObjectSchemaSnapshot, PropertySchemaSnapshot};
pub use snapshot::*;
pub use tools::{get_design_context, get_objects, AiDomainTool, AiToolError, GetObjectsRequest};
