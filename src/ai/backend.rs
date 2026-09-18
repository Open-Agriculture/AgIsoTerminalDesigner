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
    pub mode: super::task_state::AiModelMode,
    pub role: super::task_state::AiModelRole,
    pub phase: super::task_state::AiTaskPhase,
    pub strong_reasoning_effort: Option<String>,
    pub reasoning_effort: Option<String>,
    pub max_output_tokens: u32,
}
impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::OpenAI,
            model: "gpt-5.6-luna".into(),
            mode: Default::default(),
            role: Default::default(),
            phase: Default::default(),
            strong_reasoning_effort: Some("medium".into()),
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
pub const SYSTEM_INSTRUCTIONS: &str = "You assist an ISO 11783 VT object-pool designer. Follow an explicit user target; otherwise use the active mask. Inspect structure once, then focused local details only if needed. Do not inspect reverse parents for local layout/style work. Reuse existing styles; when none exist use sensible VT defaults. For a known empty screen, stop exploring and plan the design. Call finish_inspection with the minimal required_object_types as soon as the plan is sufficient. Include only types the transaction actually uses, not speculative styles, macros or masks. CreateObject uses domain defaults; set only design-relevant properties. All edits go through propose_transaction, remain staged and require user Apply. IDs identify existing objects; names can identify new objects. Positions belong to parent-child relationships; references use SetObjectReference. Repair reported validation issues without restarting exploration. Request stronger reasoning only for a concrete blocker. Use semantic colour names or RGB; never guess or describe an unresolved VT index as a named colour. Include optional theme in finish_inspection and use the resolved theme exactly. Reuse existing resolved colours for style matching. Use CreateIcon recipes for standard icons, never hand-author pixel bytes. Respect SHARED_STYLE_SIDE_EFFECT warnings: use dedicated attributes for local changes, modify shared attributes only for global intent. Do not create palettes for ordinary restyling. Review staged previews for clipping and readability, then summarize concisely.";

pub fn assistant_message(content: Vec<AiContent>) -> AiMessage {
    AiMessage {
        role: AiRole::Assistant,
        content,
    }
}
