//! Operation-based mutation system
//! Provides a unified interface for all mutations (UI, AI, scripts)
//! with reversibility, validation, and undo/redo support

mod diff;
pub mod executor;
pub mod history;
pub mod operation;
pub mod transaction;

pub use executor::OperationExecutor;
pub use history::OperationHistory;
pub use operation::{ObjectReferenceList, Operation, OperationContext, OperationError};
pub use transaction::{AppliedTransaction, OperationTransaction};

pub(crate) use diff::diff_object;
