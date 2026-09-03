//! Transaction bundling and application results

use super::operation::Operation;
use serde::{Deserialize, Serialize};

/// A transaction bundles multiple operations into a single undo/redo action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationTransaction {
    pub schema_version: u32,
    pub description: Option<String>,
    pub operations: Vec<Operation>,
}

impl OperationTransaction {
    /// Create a new empty transaction
    pub fn new(description: Option<String>) -> Self {
        Self {
            schema_version: 1,
            description,
            operations: Vec::new(),
        }
    }

    /// Add an operation to the transaction
    pub fn add_operation(&mut self, op: Operation) {
        self.operations.push(op);
    }

    /// Add multiple operations
    pub fn add_operations(&mut self, ops: Vec<Operation>) {
        self.operations.extend(ops);
    }
}

/// Result of executing a transaction
#[derive(Debug, Clone)]
pub struct AppliedTransaction {
    pub transaction_id: String,
    pub description: Option<String>,
    pub forward_operations: Vec<Operation>,
    pub inverse_operations: Vec<Operation>,
    pub affected_objects: Vec<u16>,
    pub execution_time_ms: u128,
}

impl AppliedTransaction {
    /// Generate the reverse transaction (for undo)
    /// Reverses the inverse operations so they execute in reverse order
    pub fn create_inverse(&self) -> OperationTransaction {
        OperationTransaction {
            schema_version: 1,
            description: self.description.as_ref().map(|d| format!("Undo: {}", d)),
            // Reverse the inverse_operations: they were collected in execution order,
            // so reversing puts them in correct undo order
            operations: self.inverse_operations.iter().rev().cloned().collect(),
        }
    }
}
