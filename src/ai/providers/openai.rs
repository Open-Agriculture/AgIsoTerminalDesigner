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

pub struct OpenAiProvider {
    api_key: String,
    client: reqwest::Client,
    timeout: Duration,
    cache_comparison: Arc<Mutex<CacheComparison>>,
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
        Box::pin(async move {
            let mut body = request_body(&request);
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
                debug.trace("→ OpenAI request", &body);
            }
            let response = client
                .post("https://api.openai.com/v1/responses")
                .bearer_auth(key)
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
                return Err(parse_api_error(status.as_u16(), &value));
            }
            if cfg!(debug_assertions) {
                *cache_comparison.lock().unwrap() = CacheComparison {
                    request: Arc::downgrade(&request.conversation.debug),
                    response_id: value["id"].as_str().map(str::to_owned),
                };
            }
            parse_response(value)
        })
    }
}
pub(crate) fn request_body(request: &AiTurnRequest) -> Value {
    let mut body = json!({"model":request.settings.model,"max_output_tokens":request.settings.max_output_tokens,"instructions":request.instructions,"input":to_openai_input(&request.conversation),"tools":request.tools,"store":false,"prompt_cache_key":format!("AgIsoTerminalDesigner:prompt-v2:tools-v2:{}", request.settings.model),"parallel_tool_calls":true});
    if let Some(effort) = &request.settings.reasoning_effort {
        body["reasoning"] = json!({"effort":effort});
    }
    body
}
fn to_openai_input(conversation: &crate::ai::Conversation) -> Vec<Value> {
    let mut items: Vec<Value> = conversation.messages.iter().flat_map(|message| message.content.iter().map(move |content| match content {
  AiContent::Text(text) => json!({"role":match message.role { AiRole::User => "user", AiRole::Assistant => "assistant", AiRole::Tool => "user" },"content":[{"type":"input_text","text":text}]}),
  AiContent::ToolCall(call) => json!({"type":"function_call","call_id":call.call_id,"name":call.name,"arguments":call.arguments.to_string()}),
  AiContent::ToolResult(result) => json!({"type":"function_call_output","call_id":result.call_id,"output":result.result.to_string()}),
  AiContent::Image(image) => json!({"role":match message.role { AiRole::User => "user", AiRole::Assistant => "assistant", AiRole::Tool => "user" },"content":[{"type":"input_image","image_url":format!("data:{};base64,{}", image.mime_type, base64(&image.bytes))}]}),
 })).collect();
    // Keep every call/output pair, but transmit identical successful read payloads once.
    // Earlier full results remain in the replay; local logical history is untouched.
    let mut reads = std::collections::HashSet::new();
    let mut seen = std::collections::HashMap::<String, String>::new();
    for item in &mut items {
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
    items
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
