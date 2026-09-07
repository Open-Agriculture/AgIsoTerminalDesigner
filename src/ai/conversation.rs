use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Conversation { pub messages: Vec<AiMessage> }
impl Conversation { pub fn push(&mut self, message: AiMessage) { self.messages.push(message); } }

#[derive(Debug, Clone)]
pub struct AiMessage { pub role: AiRole, pub content: Vec<AiContent> }
impl AiMessage {
    pub fn text(role: AiRole, text: impl Into<String>) -> Self { Self { role, content: vec![AiContent::Text(text.into())] } }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum AiRole { User, Assistant, Tool }
#[derive(Debug, Clone)] pub enum AiContent { Text(String), ToolCall(AiToolCall), ToolResult(AiToolResult), Image(AiImage) }
#[derive(Debug, Clone)] pub struct AiImage { pub mime_type: String, pub bytes: Vec<u8> }
#[derive(Debug, Clone)] pub struct AiToolCall { pub call_id: String, pub name: String, pub arguments: Value }
#[derive(Debug, Clone)] pub struct AiToolResult { pub call_id: String, pub result: Value }
