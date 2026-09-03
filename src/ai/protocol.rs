//! AI protocol for JSON serialization
//! Wraps operations for easy AI integration

use crate::operations::Operation;
use serde::{Deserialize, Serialize};

/// Command from AI → application
/// Uses operations internally; serialization layer for AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AICommandRequest {
    pub operations: Vec<Operation>,
    pub description: Option<String>,
    pub validate_only: bool,
}

/// Response from application → AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AICommandResponse {
    pub success: bool,
    pub transaction_id: String,
    pub affected_objects: Vec<u16>,
    pub diagnostics: Vec<AIValidationDiagnostic>,
    pub snapshot: Option<super::snapshot::PoolSnapshot>,
}

/// AI-friendly validation diagnostic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIValidationDiagnostic {
    pub severity: String, // "error", "warning", "info"
    pub message: String,
    pub code: String,
}
