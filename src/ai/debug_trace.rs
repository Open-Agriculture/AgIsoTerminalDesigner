//! Bounded, in-memory developer diagnostics. Never persists or records HTTP headers.
use serde_json::Value;
use std::collections::VecDeque;

pub const MAX_TRACE_ENTRIES: usize = 100;
pub const MAX_RAW_TRACE_BYTES_PER_ENTRY: usize = 16 * 1024;
pub const AI_TOKEN_WARNING_THRESHOLD: u64 = 50_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AiTokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}
impl AiTokenUsage {
    pub fn from_response(value: &Value) -> Self {
        let v = &value["usage"];
        Self {
            input_tokens: v["input_tokens"].as_u64().unwrap_or(0),
            cached_input_tokens: v["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0),
            output_tokens: v["output_tokens"].as_u64().unwrap_or(0),
            reasoning_tokens: v["output_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap_or(0),
            total_tokens: v["total_tokens"].as_u64().unwrap_or(0),
        }
    }
    fn add(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.total_tokens += other.total_tokens;
    }
}
#[derive(Debug, Clone, Default)]
pub struct AiUsageStats {
    pub request_count: u64,
    pub tokens: AiTokenUsage,
}
#[derive(Debug, Clone)]
pub struct AiTraceEntry {
    pub round: usize,
    pub label: String,
    pub byte_size: usize,
    pub payload: String,
    pub truncated: bool,
}
#[derive(Debug, Clone, Default)]
pub struct AiDebugState {
    pub last_api: AiTokenUsage,
    pub user_request: AiUsageStats,
    pub conversation: AiUsageStats,
    pub tool_calls: usize,
    pub provider_rounds: usize,
    pub entries: VecDeque<AiTraceEntry>,
}
impl AiDebugState {
    pub fn begin_request(&mut self) {
        self.last_api = AiTokenUsage::default();
        self.user_request = AiUsageStats::default();
        self.tool_calls = 0;
        self.provider_rounds = 0;
        self.trace(
            "User request started",
            &serde_json::json!({"previous_api_calls":self.conversation.request_count}),
        );
    }
    pub fn record_usage(&mut self, usage: AiTokenUsage) {
        self.user_request.request_count += 1;
        self.conversation.request_count += 1;
        self.user_request.tokens.add(&usage);
        self.conversation.tokens.add(&usage);
        self.last_api = usage;
    }
    pub fn trace(&mut self, label: impl Into<String>, value: &Value) {
        // Calculate size before redaction, but retain only sanitized, bounded text.
        let byte_size = value.to_string().len();
        let mut value = value.clone();
        sanitize(&mut value);
        let mut payload = value.to_string();
        let truncated = payload.len() > MAX_RAW_TRACE_BYTES_PER_ENTRY;
        if truncated {
            let mut end = MAX_RAW_TRACE_BYTES_PER_ENTRY;
            while !payload.is_char_boundary(end) {
                end -= 1;
            }
            payload.truncate(end);
        }
        let mut label = Value::String(label.into());
        sanitize(&mut label);
        let label: String = label
            .as_str()
            .unwrap_or("Trace")
            .chars()
            .take(512)
            .collect();
        self.entries.push_back(AiTraceEntry {
            round: self.provider_rounds,
            label,
            byte_size,
            payload,
            truncated,
        });
        while self.entries.len() > MAX_TRACE_ENTRIES {
            self.entries.pop_front();
        }
    }
}
pub fn safe_text(text: String) -> String {
    let mut value = Value::String(text);
    sanitize(&mut value);
    value.as_str().unwrap_or("[redacted]").to_owned()
}
fn sanitize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = key.to_ascii_lowercase();
                if [
                    "authorization",
                    "api_key",
                    "apikey",
                    "access_token",
                    "secret",
                    "password",
                ]
                .iter()
                .any(|s| key.contains(s))
                {
                    *value = Value::String("[redacted]".into());
                } else {
                    sanitize(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(sanitize),
        Value::String(text) => {
            if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                if !key.is_empty() && text.contains(&key) {
                    *text = text.replace(&key, "[redacted]");
                }
            }
            if text.starts_with("data:image/") {
                *text = "[image payload omitted]".into();
            }
            // Also covers credentials embedded inside serialized tool arguments or text.
            else if text.contains("sk-")
                || text.to_ascii_lowercase().contains("bearer ")
                || text.to_ascii_lowercase().contains("authorization")
                || text.to_ascii_lowercase().contains("api_key")
                || text.to_ascii_lowercase().contains("password")
                || text.to_ascii_lowercase().contains("secret")
            {
                *text = "[credential-like text redacted]".into();
            }
        }
        _ => {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn debug_ui(ui: &mut egui::Ui, debug: &AiDebugState, settings: &mut super::AiSettings) {
    ui.collapsing("AI Debug", |ui| {
        ui.horizontal(|ui| {
            ui.label("Max output tokens");
            ui.add(egui::DragValue::new(&mut settings.max_output_tokens).range(1024..=32768));
        });
        for (label, usage) in [
            ("Last API request", &debug.last_api),
            ("User request total", &debug.user_request.tokens),
            ("Conversation total", &debug.conversation.tokens),
        ] {
            ui.label(format!(
                "{label}: input {} (cached {}), output {} (reasoning {}), total {}",
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.output_tokens,
                usage.reasoning_tokens,
                usage.total_tokens
            ));
        }
        ui.label(format!(
            "Tool calls: {} · Provider rounds: {} · Conversation API calls: {}",
            debug.tool_calls, debug.provider_rounds, debug.conversation.request_count
        ));
        if debug.user_request.tokens.total_tokens > AI_TOKEN_WARNING_THRESHOLD {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!(
                    "High context usage: {} tokens this request",
                    debug.user_request.tokens.total_tokens
                ),
            );
        }
        ui.collapsing("Traffic (serialized bytes, not tokens)", |ui| {
            egui::ScrollArea::vertical()
                .max_height(260.)
                .id_salt("ai_traffic")
                .show(ui, |ui| {
                    for (index, entry) in debug.entries.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            ui.collapsing(
                                format!(
                                    "Round {} · {} · {} bytes",
                                    entry.round, entry.label, entry.byte_size
                                ),
                                |ui| {
                                    if entry.truncated {
                                        ui.label("Raw payload truncated");
                                    }
                                    ui.monospace(&entry.payload);
                                },
                            );
                        });
                    }
                });
        });
    });
}
