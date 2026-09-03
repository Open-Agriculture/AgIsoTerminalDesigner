//! AI integration layer
//! Provides JSON protocol and pool snapshots optimized for LLM context

pub mod protocol;
pub mod snapshot;

pub use protocol::{AICommandRequest, AICommandResponse, AIValidationDiagnostic};
pub use snapshot::{ChildPlacement, ObjectSnapshot, PoolSnapshot};
