use super::conversation::{AiContent, AiMessage, AiRole, Conversation};
use std::{future::Future, pin::Pin};

pub type AiProviderFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
#[derive(Debug, Clone)]
pub enum AiProviderKind {
    OpenAI,
}
#[derive(Debug, Clone)]
pub struct AiSettings {
    pub provider: AiProviderKind,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub max_output_tokens: u32,
}
impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::OpenAI,
            model: "gpt-5.6".into(),
            reasoning_effort: Some("medium".into()),
            max_output_tokens: 8192,
        }
    }
}
#[derive(Debug, Clone)]
pub struct AiTurnRequest {
    pub instructions: String,
    pub conversation: Conversation,
    pub tools: Vec<serde_json::Value>,
    pub settings: AiSettings,
}
#[derive(Debug, Clone)]
pub struct AiProviderResponse {
    pub content: Vec<AiContent>,
    pub usage: super::AiTokenUsage,
}
#[derive(Debug, Clone)]
pub enum AiProviderError {
    MissingApiKey,
    Http(String),
    Unauthorized,
    RateLimited(String),
    QuotaExceeded(String),
    Timeout,
    InvalidResponse(String),
    Cancelled,
}
impl std::fmt::Display for AiProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self { Self::MissingApiKey => "OpenAI API key not configured".into(), Self::Http(e) => format!("OpenAI request failed: {e}"), Self::Unauthorized => "OpenAI rejected the API key".into(), Self::RateLimited(e) => format!("OpenAI rate limit reached: {e}"), Self::QuotaExceeded(e) => format!("OpenAI API quota exceeded. Check API billing, credits, and usage limits for the project associated with your API key. {e}"), Self::Timeout => "OpenAI request timed out".into(), Self::InvalidResponse(e) => format!("OpenAI returned an invalid response: {e}"), Self::Cancelled => "Assistant request was cancelled".into() })
    }
}
impl std::error::Error for AiProviderError {}
pub trait AiProvider: Send + Sync {
    fn run_turn(
        &self,
        request: AiTurnRequest,
    ) -> AiProviderFuture<Result<AiProviderResponse, AiProviderError>>;
}
pub const SYSTEM_INSTRUCTIONS: &str = "You assist an ISO 11783 VT object-pool designer. Inspect the active mask and selection first; retrieve only relevant context and follow pagination. Batch known IDs in get_objects and needed types in get_object_schema object_types; request independent inspections together. Aim for context, one batched inspection, then a proposal. Do not repeat successful reads or guess properties. All edits use propose_transaction and remain staged, never applied. Names are symbolic; IDs identify objects. Positions belong to parent-child relationships. Use SetObjectReference for references and preserve variable/macro links unless the task requires changes. Submit related edits atomically. Review staged previews for overlap, clipping, alignment, spacing and readability. Only propose corrections when needed; corrections target the staged candidate. Finish with a concise summary.";

pub fn assistant_message(content: Vec<AiContent>) -> AiMessage {
    AiMessage {
        role: AiRole::Assistant,
        content,
    }
}
