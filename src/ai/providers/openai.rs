use crate::ai::{
    AiContent, AiProvider, AiProviderError, AiProviderFuture, AiProviderResponse, AiRole,
    AiTurnRequest,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

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
}
impl TransportState {
    fn for_request(&mut self, request: &AiTurnRequest) -> Continuation {
        if request.conversation.debug.lock().unwrap().provider_rounds <= 1
            || !Weak::ptr_eq(&self.request, &Arc::downgrade(&request.conversation.debug))
        {
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
        // Preserve opaque reasoning and assistant phase in the adapter's replay;
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
                (state.for_request(&request), !state.unsupported)
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
fn request_body_with_input(request: &AiTurnRequest, input: Vec<Value>) -> Value {
    let mut body = json!({"model":request.settings.model,"max_output_tokens":request.settings.max_output_tokens,"instructions":request.instructions,"input":input,"tools":request.tools,"store":false,"include":["reasoning.encrypted_content"],"prompt_cache_key":format!("AgIsoTerminalDesigner:prompt-v2:tools-v2:{}", request.settings.model),"parallel_tool_calls":true});
    if let Some(effort) = &request.settings.reasoning_effort {
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
                Some("get_design_context" | "get_objects" | "get_object_schema")
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

#[cfg(test)]
mod efficiency_tests {
    use super::*;
    use crate::ai::*;
    #[test]
    fn parses_actual_usage_and_missing_fields() {
        let response = parse_response(json!({"output":[{"type":"message","content":[{"type":"output_text","text":"Done"}]}],"usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":600},"output_tokens":200,"output_tokens_details":{"reasoning_tokens":80},"total_tokens":1200}})).unwrap();
        assert_eq!(
            response.usage,
            AiTokenUsage {
                input_tokens: 1000,
                cached_input_tokens: 600,
                output_tokens: 200,
                reasoning_tokens: 80,
                total_tokens: 1200
            }
        );
        assert_eq!(
            AiTokenUsage::from_response(&json!({})),
            AiTokenUsage::default()
        );
        assert_eq!(
            AiTokenUsage::from_response(&json!({"usage":{"input_tokens":10}})).cached_input_tokens,
            0
        );
    }
    #[test]
    fn initial_request_is_independent_of_pool_properties() {
        let state = crate::ai::efficiency_tests::pool(387, 2000);
        let mut request = AiTurnRequest {
            instructions: format!(
                "{}\nProject: {}",
                crate::ai::backend::SYSTEM_INSTRUCTIONS,
                serde_json::to_string(&project_summary(&state)).unwrap()
            ),
            conversation: Conversation::default(),
            tools: crate::ai::tools::tool_definitions(),
            settings: AiSettings::default(),
        };
        request.conversation.push(AiMessage::text(
            AiRole::User,
            "Make the selected label wider",
        ));
        let compact = request_body(&request).to_string().len();
        let smaller = crate::ai::efficiency_tests::pool(2, 1);
        let small_summary = serde_json::to_string(&project_summary(&smaller))
            .unwrap()
            .len();
        assert!(
            serde_json::to_string(&project_summary(&state))
                .unwrap()
                .len()
                <= small_summary + 3
        );
        // Legacy semantic retrieval: 200 full objects plus the active layout.
        let (ids, _) = crate::ai::snapshot::selected_ids(
            &state.pool,
            crate::ai::snapshot::active_mask(&state.pool),
            SnapshotScope::ActiveMask,
            16,
        );
        let old = json!({"objects":ids.into_iter().take(200).filter_map(|id| crate::ai::snapshot::object_snapshot(&state.pool, &state.object_info, id)).collect::<Vec<_>>(), "layout":crate::ai::snapshot::layout_instances(&state.pool, &state.object_info, crate::ai::snapshot::active_mask(&state.pool), 16)});
        request
            .conversation
            .push(AiMessage::text(AiRole::User, old.to_string()));
        let legacy = request_body(&request).to_string().len();
        println!("387-label synthetic pool: compact initial body {compact} bytes; body with legacy semantic context {legacy} bytes; reduction {:.1}%", 100. * (1. - compact as f64 / legacy as f64));
        assert!(legacy > compact * 20);
        assert_eq!(request_body(&request)["max_output_tokens"], 8192);
        let body = request_body(&request).to_string();
        assert!(!body.contains("Authorization"));
        assert!(!body.contains("api_key"));
    }
}

#[cfg(test)]
mod image_history_tests {
    use super::*;
    #[test]
    fn image_is_sent_during_turn_but_not_after_compaction() {
        let mut conversation = crate::ai::Conversation::default();
        conversation.push(crate::ai::AiMessage {
            role: AiRole::User,
            content: vec![AiContent::Image(crate::ai::AiImage {
                mime_type: "image/png".into(),
                bytes: vec![1, 2, 3],
            })],
        });
        assert!(serde_json::to_string(&to_openai_input(&conversation))
            .unwrap()
            .contains("data:image/png;base64,AQID"));
        conversation.compact();
        let after = serde_json::to_string(&to_openai_input(&conversation)).unwrap();
        assert!(!after.contains("base64"));
        assert!(after.contains("Visual review"));
    }
}

#[cfg(test)]
mod phase52_tests {
    use super::*;
    use crate::ai::*;
    #[test]
    fn stable_prefix_and_replay_dedup_keep_logical_history() {
        let mut request = AiTurnRequest {
            instructions: crate::ai::backend::SYSTEM_INSTRUCTIONS.into(),
            conversation: Conversation::default(),
            tools: crate::ai::tools::tool_definitions(),
            settings: AiSettings::default(),
        };
        let initial = request_body(&request);
        for id in ["one", "two"] {
            request.conversation.push(AiMessage {
                role: AiRole::Assistant,
                content: vec![AiContent::ToolCall(AiToolCall {
                    call_id: id.into(),
                    name: "get_objects".into(),
                    arguments: json!({"ids":[10]}),
                })],
            });
            request.conversation.push(AiMessage {role:AiRole::Tool,content:vec![AiContent::ToolResult(AiToolResult {call_id:id.into(),result:json!({"success":true,"objects":[{"id":10,"properties":{"value":"x".repeat(1000)}}]})})]});
        }
        let next = request_body(&request);
        for field in ["instructions", "tools", "prompt_cache_key"] {
            assert_eq!(initial[field], next[field]);
        }
        assert_eq!(next["store"], false);
        assert!(next.get("previous_response_id").is_none());
        assert_eq!(next["input"].as_array().unwrap().len(), 4);
        let output: Value =
            serde_json::from_str(next["input"][3]["output"].as_str().unwrap()).unwrap();
        assert_eq!(output["same_result_as_call_id"], "one");
        assert!(
            matches!(&request.conversation.messages[3].content[0], AiContent::ToolResult(r) if r.result["objects"].is_array())
        );
    }
}

#[cfg(test)]
mod cache_comparison_tests {
    use super::*;
    #[test]
    fn diagnostic_baseline_never_crosses_user_requests() {
        let mut request = AiTurnRequest {
            instructions: String::new(),
            conversation: Default::default(),
            tools: vec![],
            settings: Default::default(),
        };
        let comparison = CacheComparison {
            request: Arc::downgrade(&request.conversation.debug),
            response_id: Some("r1".into()),
        };
        request.conversation.debug.lock().unwrap().provider_rounds = 1;
        assert!(comparison.baseline(&request).is_none());
        request.conversation.debug.lock().unwrap().provider_rounds = 2;
        assert_eq!(comparison.baseline(&request).as_deref(), Some("r1"));
        request.conversation = Default::default();
        request.conversation.debug.lock().unwrap().provider_rounds = 2;
        assert!(comparison.baseline(&request).is_none());
    }
}

#[cfg(test)]
mod phase53_transport_tests {
    use super::*;
    use crate::ai::*;
    use std::io::{Read, Write};

    fn response(id: &str, names: &[&str]) -> Value {
        let output: Vec<_> = if names.is_empty() {
            vec![
                json!({"type":"message","id":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Staged"}]}),
            ]
        } else {
            names.iter().enumerate().map(|(i, name)| json!({"type":"function_call","id":format!("fc_{id}_{i}"),"call_id":format!("{id}_{i}"),"name":name,"arguments":"{}"})).collect()
        };
        json!({"id":id,"output":output,"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":20},"output_tokens":10,"total_tokens":110}})
    }
    fn server(responses: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, response) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut bytes = Vec::new();
                let split = loop {
                    let mut buf = [0u8; 4096];
                    let read = socket.read(&mut buf).unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&buf[..read]);
                    if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..split]);
                let len: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < split + len {
                    let mut buf = [0u8; 4096];
                    let read = socket.read(&mut buf).unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&buf[..read]);
                }
                requests.push(serde_json::from_slice(&bytes[split..split + len]).unwrap());
                let body = response.to_string();
                write!(socket,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (endpoint, thread)
    }
    fn request() -> AiTurnRequest {
        let mut conversation = Conversation::default();
        conversation.push(AiMessage::text(
            AiRole::User,
            "Add a number with a Meters label",
        ));
        AiTurnRequest {
            instructions: backend::SYSTEM_INSTRUCTIONS.into(),
            conversation,
            tools: tools::tool_definitions(),
            settings: AiSettings::default(),
        }
    }
    fn advance(provider: &OpenAiProvider, request: &mut AiTurnRequest, round: usize) {
        request.conversation.debug.lock().unwrap().provider_rounds = round;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime
            .block_on(provider.run_turn(request.clone()))
            .unwrap();
        let calls: Vec<_> = response
            .content
            .iter()
            .filter_map(|c| {
                if let AiContent::ToolCall(c) = c {
                    Some(c.clone())
                } else {
                    None
                }
            })
            .collect();
        request
            .conversation
            .push(backend::assistant_message(response.content));
        for call in calls {
            request.conversation.push(AiMessage {
                role: AiRole::Tool,
                content: vec![AiContent::ToolResult(AiToolResult {
                    call_id: call.call_id,
                    result: json!({"success":true,"name":call.name}),
                })],
            });
        }
    }
    fn provider(endpoint: String) -> OpenAiProvider {
        OpenAiProvider {
            api_key: "test".into(),
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(10),
            cache_comparison: Default::default(),
            transport: Default::default(),
            endpoint,
        }
    }
    #[test]
    fn native_continuation_transmits_only_new_outputs_and_resets_between_requests() {
        let (endpoint, server) = server(vec![
            (200, response("R1", &["get_design_context"])),
            (200, response("R2", &["get_objects", "get_object_schema"])),
            (200, response("R3", &["propose_transaction"])),
            (200, response("R4", &[])),
            (200, response("R5", &[])),
        ]);
        let provider = provider(endpoint);
        let mut request = request();
        for round in 1..=4 {
            advance(&provider, &mut request, round);
        }
        assert!(request
            .conversation
            .messages
            .iter()
            .any(|m| m.role == AiRole::User));
        assert_eq!(request.conversation.debug.lock().unwrap().rounds.len(), 4);
        for round in &request.conversation.debug.lock().unwrap().rounds[1..] {
            assert_eq!(round.historical_replay_bytes, 0);
            assert!(round.new_input_bytes > 0);
            assert!(round.continuation.starts_with("previous_response_id"));
        }
        request.conversation.compact();
        advance(&provider, &mut request, 1);
        let bodies = server.join().unwrap();
        assert!(bodies[0].get("previous_response_id").is_none());
        for (round, (id, count)) in [("R1", 1), ("R2", 2), ("R3", 1)].iter().enumerate() {
            let body = &bodies[round + 1];
            assert_eq!(body["previous_response_id"], *id);
            let input = body["input"].as_array().unwrap();
            assert_eq!(input.len(), *count);
            assert!(input.iter().all(|v| v["type"] == "function_call_output"
                && v["call_id"].as_str().unwrap().starts_with(id)));
        }
        assert!(bodies[4].get("previous_response_id").is_none());
        for body in &bodies {
            assert_eq!(body["store"], false);
            for key in ["instructions", "tools", "prompt_cache_key"] {
                assert_eq!(body[key], bodies[0][key]);
            }
        }
    }
    #[test]
    fn rejected_continuation_retries_once_with_full_context_and_no_storage_change() {
        let mut first = response("R1", &["get_design_context"]);
        first["output"].as_array_mut().unwrap().insert(
            0,
            json!({"id":"reasoning","type":"reasoning","summary":[],"encrypted_content":"opaque"}),
        );
        let (endpoint, server) = server(vec![
            (200, first),
            (
                400,
                json!({"error":{"param":"previous_response_id","message":"Previous response not found"}}),
            ),
            (200, response("R2", &["get_objects"])),
            (200, response("R3", &[])),
        ]);
        let provider = provider(endpoint);
        let mut request = request();
        for round in 1..=3 {
            advance(&provider, &mut request, round);
        }
        let bodies = server.join().unwrap();
        assert_eq!(bodies[1]["previous_response_id"], "R1");
        for body in &bodies[2..] {
            assert!(body.get("previous_response_id").is_none());
            assert_eq!(body["input"][0]["role"], "user");
            assert_eq!(body["store"], false);
            let items = body["input"].as_array().unwrap();
            assert!(items.iter().any(|v| v["encrypted_content"] == "opaque"));
            assert!(items
                .iter()
                .any(|v| v["type"] == "function_call" && v["call_id"] == "R1_0"));
            assert!(items
                .iter()
                .any(|v| v["type"] == "function_call_output" && v["call_id"] == "R1_0"));
        }
        let debug = request.conversation.debug.lock().unwrap();
        assert_eq!(debug.rounds.last().unwrap().continuation, "local replay");
        assert!(debug.rounds.last().unwrap().historical_replay_bytes > 0);
        assert_eq!(debug.user_request.tokens.input_tokens, 300);
        assert!(!continuation_rejected(
            429,
            &json!({"error":{"param":"previous_response_id"}})
        ));
        assert!(!continuation_rejected(
            400,
            &json!({"error":{"param":"tools"}})
        ));
    }
    #[test]
    #[ignore = "Live API probe: requires OPENAI_API_KEY and incurs a small API charge"]
    fn live_store_false_continuation_probe() {
        let provider = OpenAiProvider::from_environment().expect("OPENAI_API_KEY is required");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let initial = provider.client.post(&provider.endpoint).bearer_auth(&provider.api_key)
                .json(&json!({"model":AiSettings::default().model,"store":false,"input":"Reply OK.","max_output_tokens":64}))
                .send().await.unwrap();
            assert!(initial.status().is_success(),"Initial API request failed: {}",initial.status());
            let first: Value = initial.json().await.unwrap();
            let response = provider.client.post(&provider.endpoint).bearer_auth(&provider.api_key)
                .json(&json!({"model":AiSettings::default().model,"store":false,"previous_response_id":first["id"],"instructions":"Reply briefly.","input":"Reply OK again.","max_output_tokens":64}))
                .send().await.unwrap();
            let status = response.status();
            let value: Value = response.json().await.unwrap();
            println!("store:false continuation HTTP {status}; native supported: {}",status.is_success());
            assert!(status.is_success() || continuation_rejected(status.as_u16(),&value),"Unexpected API error: {}",crate::ai::debug_trace::safe_text(value.to_string()));
        });
    }
}
