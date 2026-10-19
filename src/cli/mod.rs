//! Standalone CLI editing, inspection, exports, and session persistence.
pub mod automation_service;
pub mod design_inspection;
pub mod editing;
pub mod export;
pub mod preview;
pub mod protocol;
pub mod schema;
#[cfg(not(target_arch = "wasm32"))]
pub mod session;
pub mod snapshot;
pub mod tools;

use crate::ObjectInfo;
use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectId, ObjectPool};
use std::collections::HashMap;
pub type ProjectRevision = u64;
#[derive(Debug, Clone)]
pub struct AiDesignState {
    pub revision: ProjectRevision,
    pub components: crate::components::ComponentSource,
    pub graphics: crate::graphics::GraphicLibrary,
    pub object_id_ranges: crate::ObjectIdRanges,
    pub pool: ObjectPool,
    pub object_info: HashMap<ObjectId, ObjectInfo>,
    pub vt_version: VtVersion,
    pub mask_size: u16,
    /// Saved layout dimensions used by inspection and schematic rendering.
    pub soft_key_size: (u16, u16),
    pub soft_key_layout: crate::SoftKeyLayout,
    pub selected: Option<ObjectId>,
}
pub use preview::{AiImage, PreviewError, RenderedPreview};
pub use schema::get_object_schema;
pub use snapshot::*;
pub use tools::{get_design_context, get_objects, AiToolError, GetObjectsRequest};

pub mod visual_intent;
