//! Bounded, in-memory developer diagnostics. Never persists or records HTTP headers.
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;

pub const MAX_TRACE_ENTRIES: usize = 100;
pub const MAX_RAW_TRACE_BYTES_PER_ENTRY: usize = 16 * 1024;
pub const AI_TOKEN_WARNING_THRESHOLD: u64 = 50_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AiTokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}
impl AiTokenUsage {
    pub fn uncached_input_tokens(&self) -> u64 {
        self.input_tokens.saturating_sub(self.cached_input_tokens)
    }
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
#[derive(Debug, Clone, Default, Serialize)]
pub struct AiUsageStats {
    pub request_count: u64,
    pub tokens: AiTokenUsage,
}
#[derive(Debug, Clone, Serialize)]
pub struct AiTraceEntry {
    pub round: usize,
    pub label: String,
    pub byte_size: usize,
    pub payload: String,
    pub truncated: bool,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct AiProviderRoundStats {
    pub reason: String,
    pub prompt_cache_key: String,
    pub prompt_cache_diagnostics: Option<Value>,
    pub model: String,
    pub phase: String,
    pub round: usize,
    #[serde(flatten)]
    pub tokens: AiTokenUsage,
    pub usage_reported: bool,
    pub uncached_input_tokens: u64,
    pub continuation: String,
    pub new_input_bytes: usize,
    pub historical_replay_bytes: usize,
    pub request_bytes: usize,
    pub instructions_bytes: usize,
    pub messages_bytes: usize,
    pub tool_definitions_bytes: usize,
    pub propose_transaction_bytes: usize,
    pub tool_results_bytes: usize,
    pub images_bytes: usize,
    pub instructions_changed: bool,
    pub tools_changed: bool,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct AiToolStats {
    pub calls: usize,
    pub cache_hits: usize,
    pub result_bytes: usize,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct AiDebugState {
    pub rounds: Vec<AiProviderRoundStats>,
    pub current_model: String,
    pub latency_ms: u64,
    pub failure_class: Option<super::task_state::AiFailureClass>,
    pub model_usage: std::collections::BTreeMap<String, AiUsageStats>,
    pub task: Option<super::task_state::AiTaskState>,
    pub largest_tool_results: Vec<(String, usize)>,
    pub tools: std::collections::BTreeMap<String, AiToolStats>,
    #[serde(skip)]
    previous_prefix: Option<(String, String)>,
    pub last_api: AiTokenUsage,
    pub user_request: AiUsageStats,
    pub conversation: AiUsageStats,
    pub tool_calls: usize,
    pub provider_rounds: usize,
    pub entries: VecDeque<AiTraceEntry>,
}
impl AiDebugState {
    pub fn begin_request(&mut self) {
        self.rounds.clear();
        self.model_usage.clear();
        self.latency_ms = 0;
        self.failure_class = None;
        self.task = None;
        self.largest_tool_results.clear();
        self.tools.clear();
        self.previous_prefix = None;
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
        let model = self
            .model_usage
            .entry(self.current_model.clone())
            .or_default();
        model.request_count += 1;
        model.tokens.add(&usage);
        self.user_request.request_count += 1;
        self.conversation.request_count += 1;
        self.user_request.tokens.add(&usage);
        self.conversation.tokens.add(&usage);
        if let Some(round) = self
            .rounds
            .last_mut()
            .filter(|r| r.round == self.provider_rounds)
        {
            round.tokens = usage.clone();
            round.uncached_input_tokens = usage.uncached_input_tokens();
            round.usage_reported = true;
        }
        self.last_api = usage;
    }
    pub fn record_request(&mut self, body: &Value) {
        if let Some(model) = body["model"].as_str() {
            self.current_model = model.into();
        }
        let instructions = body["instructions"].to_string();
        let tools = body["tools"].to_string();
        let mut stats = AiProviderRoundStats {
            reason: self
                .task
                .as_ref()
                .map(|t| t.round_reason.clone())
                .unwrap_or_default(),
            prompt_cache_key: body["prompt_cache_key"].as_str().unwrap_or_default().into(),
            model: self.current_model.clone(),
            phase: self
                .task
                .as_ref()
                .map(|t| format!("{:?}", t.phase))
                .unwrap_or_default(),
            round: self.provider_rounds,
            continuation: body["previous_response_id"]
                .as_str()
                .map(|id| format!("previous_response_id: {id}"))
                .unwrap_or_else(|| "local replay (store=false configuration)".into()),
            request_bytes: body.to_string().len(),
            instructions_bytes: instructions.len(),
            tool_definitions_bytes: tools.len(),
            propose_transaction_bytes: body["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["name"] == "propose_transaction")
                .map(|t| t.to_string().len())
                .sum(),
            ..Default::default()
        };
        if let Some((old_instructions, old_tools)) = &self.previous_prefix {
            stats.instructions_changed = old_instructions != &instructions;
            stats.tools_changed = old_tools != &tools;
        }
        self.previous_prefix = Some((instructions, tools));
        for item in body["input"].as_array().into_iter().flatten() {
            let bytes = item.to_string().len();
            if item["type"] == "function_call_output" {
                stats.tool_results_bytes += bytes;
            } else if item["content"]
                .as_array()
                .is_some_and(|c| c.iter().any(|p| p["type"] == "input_image"))
            {
                stats.images_bytes += bytes;
            } else {
                stats.messages_bytes += bytes;
            }
        }
        self.rounds.push(stats);
    }
    pub fn record_tool(&mut self, name: &str, cache_hit: bool, result: &Value) {
        self.largest_tool_results
            .push((name.to_owned(), result.to_string().len()));
        self.largest_tool_results
            .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.largest_tool_results.truncate(5);
        let stats = self.tools.entry(name.to_owned()).or_default();
        stats.calls += 1;
        stats.cache_hits += usize::from(cache_hit);
        stats.result_bytes += result.to_string().len();
    }
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for name in ["get_objects", "get_object_schema"] {
            if let Some(stats) = self.tools.get(name).filter(|s| s.calls > 1) {
                warnings.push(format!(
                    "{} {name} calls; independent reads could potentially be batched.",
                    stats.calls
                ));
            }
        }
        if self
            .rounds
            .iter()
            .any(|r| r.instructions_changed || r.tools_changed)
        {
            warnings.push("Instructions or tool definitions changed between rounds; reusable prefix may be shorter.".into());
        }
        if self.user_request.tokens.input_tokens > 0
            && self.user_request.tokens.cached_input_tokens == 0
        {
            warnings.push("Provider reported zero cached input tokens. This alone does not establish a cache-miss reason.".into());
        }
        warnings
    }
    /// Export exactly the bounded, sanitized diagnostics; never headers or image bytes.
    pub fn export(&self) -> String {
        let mut value = serde_json::to_value(self).unwrap();
        value["user_request"]["tokens"]["uncached_input_tokens"] =
            serde_json::json!(self.user_request.tokens.uncached_input_tokens());
        value["conversation"]["tokens"]["uncached_input_tokens"] =
            serde_json::json!(self.conversation.tokens.uncached_input_tokens());
        value["last_api"]["uncached_input_tokens"] =
            serde_json::json!(self.last_api.uncached_input_tokens());
        for (name, usage) in &self.model_usage {
            value["model_usage"][name]["tokens"]["uncached_input_tokens"] =
                serde_json::json!(usage.tokens.uncached_input_tokens());
        }
        value["warnings"] = serde_json::json!(self.warnings());
        sanitize(&mut value);
        serde_json::to_string_pretty(&value).unwrap()
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
        if ui.button("Copy debug output").clicked() {
            ui.ctx().copy_text(debug.export());
        }
        ui.horizontal(|ui| {
            ui.label("AI mode");
            for (mode, label) in [(super::task_state::AiModelMode::Automatic,"Automatic"),(super::task_state::AiModelMode::Fast,"Luna"),(super::task_state::AiModelMode::Strong,"Sol")] {
                ui.selectable_value(&mut settings.mode, mode, label);
            }
        });
        ui.label(format!("Current model: {} · provider latency {} ms",debug.current_model,debug.latency_ms));
        if let Some(task) = &debug.task {
            ui.label(format!("Task: {}", task.goal));
            ui.label(format!("Status: {} · target {:?} · resume available {} · checkpoint {}", if task.is_complete { "complete" } else if task.checkpoint { "paused" } else { "running" }, task.target_root, !task.is_complete, task.checkpoint));
            ui.label(format!("Schemas: {} · harness normalizations {}", task.loaded_schemas.iter().cloned().collect::<Vec<_>>().join(", "), task.normalization_count));
            ui.label(format!("Phase {:?} · inspection rounds {} · proposals {} · validation failures {} · escalation {:?}",task.phase,task.inspection_rounds,task.proposal_attempts,task.validation_failures,task.escalation));
        }
        for (name, usage) in &debug.model_usage {
            ui.label(format!("{name}: {} calls · input {} · cached {} · uncached {} · output {} · total {}",usage.request_count,usage.tokens.input_tokens,usage.tokens.cached_input_tokens,usage.tokens.uncached_input_tokens(),usage.tokens.output_tokens,usage.tokens.total_tokens));
        }
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
                "{label}: input {} (cached {}, uncached {}), output {} (reasoning {}), total {}",
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.uncached_input_tokens(),
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
        ui.collapsing("Provider rounds (exact tokens; serialized bytes)", |ui| {
            for round in &debug.rounds {
                ui.label(format!("Why another round: {}", round.reason));
                ui.collapsing(format!("Round {} · input {} · cached {} · uncached {} · output {} · {} bytes", round.round, round.tokens.input_tokens, round.tokens.cached_input_tokens, round.tokens.uncached_input_tokens(), round.tokens.output_tokens, round.request_bytes), |ui| {
                    ui.label(format!("Continuation: {} · New input bytes: {} · Historical replay bytes: {}", round.continuation, round.new_input_bytes, round.historical_replay_bytes));
                    ui.label(format!("Reasoning {} · total {} · usage reported {}", round.tokens.reasoning_tokens, round.tokens.total_tokens, round.usage_reported));
                    ui.label(format!("Instructions {} · messages {} · tools {} (proposal {}) · results {} · images {} bytes", round.instructions_bytes, round.messages_bytes, round.tool_definitions_bytes, round.propose_transaction_bytes, round.tool_results_bytes, round.images_bytes));
                    ui.label(format!("Instructions changed: {} · tools changed: {}", round.instructions_changed, round.tools_changed));
                });
            }
        });
        ui.collapsing("Largest tool results", |ui| {
            for (name, bytes) in &debug.largest_tool_results { ui.label(format!("{name}: {bytes} bytes")); }
        });
        for (name, stats) in &debug.tools {
            ui.label(format!("{name}: {} calls · {} cache hits · {} result bytes", stats.calls, stats.cache_hits, stats.result_bytes));
        }
        for warning in debug.warnings() { ui.label(warning); }
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
