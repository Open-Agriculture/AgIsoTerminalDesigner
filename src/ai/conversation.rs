use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Conversation {
    pub messages: Vec<AiMessage>,
    pub replay_generation: u64,
    pub task_state: Option<super::task_state::AiTaskState>,
    pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub debug: std::sync::Arc<std::sync::Mutex<super::debug_trace::AiDebugState>>,
}
impl Conversation {
    pub fn push(&mut self, message: AiMessage) {
        self.messages.push(message);
    }
}

#[derive(Debug, Clone)]
pub struct AiMessage {
    pub role: AiRole,
    pub content: Vec<AiContent>,
}
impl AiMessage {
    pub fn text(role: AiRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![AiContent::Text(text.into())],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRole {
    User,
    Assistant,
    Tool,
}
#[derive(Debug, Clone)]
pub enum AiContent {
    Text(String),
    ToolCall(AiToolCall),
    ToolResult(AiToolResult),
    Image(AiImage),
}
#[derive(Debug, Clone)]
pub struct AiImage {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct AiToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Debug, Clone)]
pub struct AiToolResult {
    pub call_id: String,
    pub result: Value,
}

impl Conversation {
    /// Only call between user requests: never remove an in-flight tool chain.
    pub fn compact(&mut self) {
        for message in &mut self.messages {
            message.content = message
                .content
                .drain(..)
                .filter_map(|content| match content {
                    AiContent::Text(text) => Some(AiContent::Text(text)),
                    AiContent::Image(_) => Some(AiContent::Text(
                        "Visual review of staged proposal completed.".into(),
                    )),
                    AiContent::ToolResult(result) if result.result.get("proposal_id").is_some() => {
                        Some(AiContent::Text(format!(
                            "Staged proposal {}: {} operations; not applied.",
                            result.result["proposal_id"], result.result["operation_count"]
                        )))
                    }
                    _ => None,
                })
                .collect();
            if message.role == AiRole::Tool {
                message.role = AiRole::Assistant;
            }
        }
        self.messages.retain(|m| !m.content.is_empty());
        const KEEP_TEXT_MESSAGES: usize = 12;
        let excess = self.messages.len().saturating_sub(KEEP_TEXT_MESSAGES);
        self.messages.drain(..excess);
    }
}
