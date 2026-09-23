use crate::ai::{
    AiContent, AiProvider, AiProviderError, AiProviderFuture, AiProviderResponse, AiRole,
    AiTurnRequest,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

pub fn model_id(settings: &crate::ai::AiSettings) -> &str {
    match settings.role {
        crate::ai::task_state::AiModelRole::Fast => &settings.model,
        crate::ai::task_state::AiModelRole::Strong => "gpt-5.6-sol",
    }
}

#[derive(Default)]
struct CacheComparison {
    request: Weak<Mutex<crate::ai::debug_trace::AiDebugState>>,
    response_id: Option<String>,
}
impl CacheComparison {
    fn baseline(&self, request: &AiTurnRequest) -> Option<String> {
        let debug = request.conversation.debug.lock().unwrap();
        (debug.provider_rounds > 1
            && Weak::ptr_eq(&self.request, &Arc::downgrade(&request.conversation.debug)))
        .then(|| self.response_id.clone())
        .flatten()
    }
}

/// Request-local transport state; the application's conversation stays neutral.
#[derive(Default, Clone)]
struct Continuation {
    response_id: Option<String>,
    next_message: usize,
    replay_outputs: std::collections::BTreeMap<usize, Vec<Value>>,
}
#[derive(Default)]
struct TransportState {
    request: Weak<Mutex<crate::ai::debug_trace::AiDebugState>>,
    continuation: Continuation,
    // Once the endpoint rejects non-stored continuation, avoid repeated probes.
    unsupported: bool,
    generation: u64,
}
impl TransportState {
    fn for_request(&mut self, request: &AiTurnRequest) -> Continuation {
        if self.generation != request.conversation.replay_generation
            || request.conversation.debug.lock().unwrap().provider_rounds <= 1
            || !Weak::ptr_eq(&self.request, &Arc::downgrade(&request.conversation.debug))
        {
            self.generation = request.conversation.replay_generation;
            self.request = Arc::downgrade(&request.conversation.debug);
            self.continuation = Continuation::default();
        }
        self.continuation.clone()
    }
}
impl Continuation {
    fn body(&self, request: &AiTurnRequest, native: bool) -> (Value, usize) {
        let cursor = self.next_message.min(request.conversation.messages.len());
        let new_input = input_from_messages(&request.conversation.messages[cursor..]);
        if native && self.response_id.is_some() && cursor > 0 {
            let mut body = request_body_with_input(request, new_input);
            body["previous_response_id"] = json!(self.response_id);
            return (body, 0);
        }
        // Preserve assistant phase and tool-call continuity, without reasoning blobs;
        // domain tools never see provider response IDs or provider output items.
        let mut input = Vec::new();
        let mut historical_bytes = 0;
        for (index, message) in request.conversation.messages.iter().enumerate() {
            let items = if let Some(output) = self.replay_outputs.get(&index) {
                output.clone()
            } else {
                input_from_messages(std::slice::from_ref(message))
            };
            if index < cursor {
                historical_bytes += items.iter().map(|v| v.to_string().len()).sum::<usize>();
            }
            input.extend(items);
        }
        deduplicate_reads(&mut input);
        if cursor > 0 {
            historical_bytes = input
                .iter()
                .take(input.len().saturating_sub(new_input.len()))
                .map(|v| v.to_string().len())
                .sum();
        }
        (request_body_with_input(request, input), historical_bytes)
    }
    fn accept(&mut self, request: &AiTurnRequest, value: &Value) {
        self.response_id = value["id"].as_str().map(str::to_owned);
        let mut output = value["output"].as_array().cloned().unwrap_or_default();
        output.retain(|item| item["type"] != "reasoning");
        for item in &mut output {
            // IDs refer to non-stored objects; replay their contents instead.
            if let Some(map) = item.as_object_mut() {
                map.remove("id");
            }
        }
        self.replay_outputs
            .insert(request.conversation.messages.len(), output);
        self.next_message = request.conversation.messages.len() + 1;
    }
}
fn continuation_rejected(status: u16, value: &Value) -> bool {
    if !matches!(status, 400 | 404) {
        return false;
    }
    let error = &value["error"];
    error["param"] == "previous_response_id"
        || error["code"] == "previous_response_not_found"
        || error["message"].as_str().is_some_and(|m| {
            let m = m.to_ascii_lowercase();
            m.contains("previous_response_id") || m.contains("previous response")
        })
}

pub struct OpenAiProvider {
    api_key: String,
    client: reqwest::Client,
    timeout: Duration,
    cache_comparison: Arc<Mutex<CacheComparison>>,
    transport: Arc<Mutex<TransportState>>,
    endpoint: String,
}
impl OpenAiProvider {
    pub fn from_environment() -> Result<Self, AiProviderError> {
        let api_key =
            std::env::var("OPENAI_API_KEY").map_err(|_| AiProviderError::MissingApiKey)?;
        if api_key.trim().is_empty() {
            return Err(AiProviderError::MissingApiKey);
        }
        Ok(Self {
            api_key,
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(90),
            cache_comparison: Default::default(),
            transport: Default::default(),
            endpoint: "https://api.openai.com/v1/responses".into(),
        })
    }
}
impl AiProvider for OpenAiProvider {
    fn run_turn(
        &self,
        request: AiTurnRequest,
    ) -> AiProviderFuture<Result<AiProviderResponse, AiProviderError>> {
        let client = self.client.clone();
        let key = self.api_key.clone();
        let timeout = self.timeout;
        let cache_comparison = self.cache_comparison.clone();
        let transport = self.transport.clone();
        let endpoint = self.endpoint.clone();
        Box::pin(async move {
            let (mut continuation, mut native) = {
                let mut state = transport.lock().unwrap();
                (state.for_request(&request), false)
            };
            loop {
                let (mut body, historical_bytes) = continuation.body(&request, native);
                // This requests diagnostics only, never server-side conversation continuation.
                // Unknown/older models retain their original request shape.
                if cfg!(debug_assertions)
                    && (request.settings.model.starts_with("gpt-5.6")
                        || request.settings.model.starts_with("gpt-6"))
                {
                    if let Some(id) = cache_comparison.lock().unwrap().baseline(&request) {
                        body["prompt_cache_options"] = json!({"comparison_response_id":id});
                    }
                }
                {
                    let mut debug = request.conversation.debug.lock().unwrap();
                    debug.record_request(&body);
                    if let Some(round) = debug.rounds.last_mut() {
                        round.historical_replay_bytes = historical_bytes;
                        round.new_input_bytes = body["input"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|v| v.to_string().len())
                            .sum::<usize>()
                            .saturating_sub(historical_bytes);
                    }
                    debug.trace("→ OpenAI request", &body);
                }
                let response = client
                    .post(&endpoint)
                    .bearer_auth(&key)
                    .json(&body)
                    .timeout(timeout)
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            AiProviderError::Timeout
                        } else {
                            AiProviderError::Http(e.to_string())
                        }
                    })?;
                let status = response.status();
                let value: Value = response
                    .json()
                    .await
                    .map_err(|e| AiProviderError::InvalidResponse(e.to_string()))?;
                {
                    let mut debug = request.conversation.debug.lock().unwrap();
                    debug.trace("← OpenAI response", &value);
                    if let Some(diagnostics) = value
                        .get("prompt_cache_diagnostics")
                        .filter(|v| !v.is_null())
                    {
                        debug.trace("OpenAI prompt cache diagnostics", diagnostics);
                    }
                    // Record before parsing content so incomplete/malformed responses still count.
                    if value.get("usage").is_some_and(|v| !v.is_null()) {
                        debug.record_usage(crate::ai::AiTokenUsage::from_response(&value));
                    }
                }
                if !status.is_success() {
                    if body.get("previous_response_id").is_some()
                        && continuation_rejected(status.as_u16(), &value)
                    {
                        native = false;
                        transport.lock().unwrap().unsupported = true;
                        request.conversation.debug.lock().unwrap().trace("Continuation fallback: local replay", &json!({"reason":"Endpoint rejected previous_response_id with store:false"}));
                        continue;
                    }
                    return Err(parse_api_error(status.as_u16(), &value));
                }
                if cfg!(debug_assertions) {
                    *cache_comparison.lock().unwrap() = CacheComparison {
                        request: Arc::downgrade(&request.conversation.debug),
                        response_id: value["id"].as_str().map(str::to_owned),
                    };
                }
                let response = parse_response(value.clone())?;
                if let Some(round) = request.conversation.debug.lock().unwrap().rounds.last_mut() {
                    round.prompt_cache_diagnostics = value
                        .get("prompt_cache_diagnostics")
                        .cloned()
                        .or_else(|| value["usage"].get("prompt_cache_diagnostics").cloned());
                }
                continuation.accept(&request, &value);
                transport.lock().unwrap().continuation = continuation;
                return Ok(response);
            }
        })
    }
}
#[cfg(test)]
pub(crate) fn request_body(request: &AiTurnRequest) -> Value {
    request_body_with_input(request, to_openai_input(&request.conversation))
}
fn request_body_with_input(request: &AiTurnRequest, mut input: Vec<Value>) -> Value {
    if let Some(state) = &request.conversation.task_state {
        input.push(json!({"role":"user","content":[{"type":"input_text","text":format!("Task state: {}",serde_json::to_string(state).unwrap())}]}));
    }
    let mut body = json!({"model":model_id(&request.settings),"max_output_tokens":request.settings.max_output_tokens,"instructions":request.instructions,"input":input,"tools":request.tools,"store":false,"prompt_cache_key":format!("AgIsoTerminalDesigner:{}:v4", model_id(&request.settings)),"parallel_tool_calls":true});
    let effort = if request.settings.role == crate::ai::task_state::AiModelRole::Strong {
        &request.settings.strong_reasoning_effort
    } else {
        &request.settings.reasoning_effort
    };
    if let Some(effort) = effort {
        body["reasoning"] = json!({"effort":effort});
    }
    body
}
#[cfg(test)]
fn to_openai_input(conversation: &crate::ai::Conversation) -> Vec<Value> {
    input_from_messages(&conversation.messages)
}
fn input_from_messages(messages: &[crate::ai::AiMessage]) -> Vec<Value> {
    let mut items: Vec<Value> = messages.iter().flat_map(|message| message.content.iter().map(move |content| match content {
  AiContent::Text(text) => json!({"role":match message.role { AiRole::User => "user", AiRole::Assistant => "assistant", AiRole::Tool => "user" },"content":[{"type":"input_text","text":text}]}),
  AiContent::ToolCall(call) => json!({"type":"function_call","call_id":call.call_id,"name":call.name,"arguments":call.arguments.to_string()}),
  AiContent::ToolResult(result) => json!({"type":"function_call_output","call_id":result.call_id,"output":result.result.to_string()}),
  AiContent::Image(image) => json!({"role":match message.role { AiRole::User => "user", AiRole::Assistant => "assistant", AiRole::Tool => "user" },"content":[{"type":"input_image","image_url":format!("data:{};base64,{}", image.mime_type, base64(&image.bytes))}]}),
 })).collect();
    deduplicate_reads(&mut items);
    items
}
fn deduplicate_reads(items: &mut [Value]) {
    // Keep every call/output pair, but transmit identical successful read payloads once.
    // Earlier full results remain in the replay; local logical history is untouched.
    let mut reads = std::collections::HashSet::new();
    let mut seen = std::collections::HashMap::<String, String>::new();
    for item in items {
        if item["type"] == "function_call"
            && matches!(
                item["name"].as_str(),
                Some(
                    "get_design_context"
                        | "get_objects"
                        | "get_object_schema"
                        | "inspect_design_region"
                )
            )
        {
            reads.insert(item["call_id"].as_str().unwrap_or_default().to_owned());
        }
        if item["type"] == "function_call_output"
            && reads.contains(item["call_id"].as_str().unwrap_or_default())
        {
            let output = item["output"].as_str().unwrap_or_default().to_owned();
            if serde_json::from_str::<Value>(&output)
                .ok()
                .is_some_and(|v| v["success"] == true)
            {
                if let Some(id) = seen.get(&output) {
                    let reference = json!({"success":true,"same_result_as_call_id":id}).to_string();
                    if reference.len() < output.len() {
                        item["output"] = json!(reference);
                    }
                } else {
                    seen.insert(
                        output,
                        item["call_id"].as_str().unwrap_or_default().to_owned(),
                    );
                }
            }
        }
    }
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(a >> 2) as usize] as char);
        out.push(TABLE[((a & 3) << 4 | b >> 4) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((b & 15) << 2 | c >> 6) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
fn parse_response(value: Value) -> Result<AiProviderResponse, AiProviderError> {
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| AiProviderError::InvalidResponse("missing output array".into()))?;
    let mut content = Vec::new();
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                let id = item.get("call_id").and_then(Value::as_str).ok_or_else(|| {
                    AiProviderError::InvalidResponse("function call missing call_id".into())
                })?;
                let name = item.get("name").and_then(Value::as_str).ok_or_else(|| {
                    AiProviderError::InvalidResponse("function call missing name".into())
                })?;
                let raw = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AiProviderError::InvalidResponse("function call missing arguments".into())
                    })?;
                let args = serde_json::from_str(raw).map_err(|e| {
                    AiProviderError::InvalidResponse(format!("invalid arguments for {name}: {e}"))
                })?;
                content.push(AiContent::ToolCall(crate::ai::AiToolCall {
                    call_id: id.into(),
                    name: name.into(),
                    arguments: args,
                }));
            }
            Some("message") => {
                if let Some(parts) = item.get("content").and_then(Value::as_array) {
                    for part in parts {
                        if part.get("type").and_then(Value::as_str) == Some("output_text") {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                content.push(AiContent::Text(text.into()));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if content.is_empty() {
        return Err(AiProviderError::InvalidResponse(
            "response contained no text or function calls".into(),
        ));
    }
    Ok(AiProviderResponse {
        usage: crate::ai::AiTokenUsage::from_response(&value),
        content,
    })
}

fn parse_api_error(status: u16, value: &Value) -> AiProviderError {
    let error = &value["error"];
    let message = error["message"]
        .as_str()
        .unwrap_or("No error details returned by OpenAI")
        .to_owned();
    match status {
        401 => AiProviderError::Unauthorized,
        429 if ["code", "type"]
            .iter()
            .any(|field| error[*field].as_str() == Some("insufficient_quota")) =>
        {
            AiProviderError::QuotaExceeded(message)
        }
        429 => AiProviderError::RateLimited(message),
        _ => AiProviderError::Http(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_errors_explain_billing_instead_of_rate_limits() {
        for field in ["code", "type"] {
            let mut value = json!({"error": {"message": "You exceeded your current quota."}});
            value["error"][field] = json!("insufficient_quota");
            let error = parse_api_error(429, &value);
            assert!(matches!(error, AiProviderError::QuotaExceeded(_)));
            assert!(error.to_string().contains("API billing"));
            assert!(error
                .to_string()
                .contains("You exceeded your current quota."));
        }
    }

    #[test]
    fn rate_limits_preserve_server_details() {
        let error = parse_api_error(
            429,
            &json!({"error": {
                "code": "rate_limit_exceeded",
                "message": "Tokens per minute exceeded. Please try again in 20s."
            }}),
        );
        assert!(matches!(error, AiProviderError::RateLimited(_)));
        assert!(error.to_string().contains("Please try again in 20s."));
    }

    #[test]
    fn missing_error_details_do_not_panic() {
        assert!(matches!(
            parse_api_error(429, &json!({})),
            AiProviderError::RateLimited(_)
        ));
        assert!(matches!(
            parse_api_error(401, &json!({})),
            AiProviderError::Unauthorized
        ));
    }
}
