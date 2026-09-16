use crate::ai::{AiContent, AiProvider, AiProviderError, AiProviderFuture, AiProviderResponse, AiRole, AiTurnRequest};
use serde_json::{json, Value};
use std::time::Duration;

pub struct OpenAiProvider { api_key: String, client: reqwest::Client, timeout: Duration }
impl OpenAiProvider {
    pub fn from_environment() -> Result<Self, AiProviderError> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| AiProviderError::MissingApiKey)?;
        if api_key.trim().is_empty() { return Err(AiProviderError::MissingApiKey); }
        Ok(Self { api_key, client: reqwest::Client::new(), timeout: Duration::from_secs(90) })
    }
}
impl AiProvider for OpenAiProvider {
 fn run_turn(&self, request: AiTurnRequest) -> AiProviderFuture<Result<AiProviderResponse, AiProviderError>> {
  let client = self.client.clone(); let key = self.api_key.clone(); let timeout = self.timeout;
  Box::pin(async move {
   let mut body = json!({"model":request.settings.model,"instructions":request.instructions,"input":to_openai_input(&request.conversation),"tools":request.tools,"store":false,"parallel_tool_calls":true});
   if let Some(effort) = request.settings.reasoning_effort { body["reasoning"] = json!({"effort":effort}); }
   let response = client.post("https://api.openai.com/v1/responses").bearer_auth(key).json(&body).timeout(timeout).send().await.map_err(|e| if e.is_timeout() { AiProviderError::Timeout } else { AiProviderError::Http(e.to_string()) })?;
   let status = response.status(); let value: Value = response.json().await.map_err(|e| AiProviderError::InvalidResponse(e.to_string()))?;
   if !status.is_success() { return Err(parse_api_error(status.as_u16(), &value)); }
   parse_response(value)
  })
 }
}
fn to_openai_input(conversation: &crate::ai::Conversation) -> Vec<Value> {
 conversation.messages.iter().flat_map(|message| message.content.iter().map(move |content| match content {
  AiContent::Text(text) => json!({"role":match message.role { AiRole::User => "user", AiRole::Assistant => "assistant", AiRole::Tool => "user" },"content":[{"type":"input_text","text":text}]}),
  AiContent::ToolCall(call) => json!({"type":"function_call","call_id":call.call_id,"name":call.name,"arguments":call.arguments.to_string()}),
  AiContent::ToolResult(result) => json!({"type":"function_call_output","call_id":result.call_id,"output":result.result.to_string()}),
  AiContent::Image(image) => json!({"role":match message.role { AiRole::User => "user", AiRole::Assistant => "assistant", AiRole::Tool => "user" },"content":[{"type":"input_image","image_url":format!("data:{};base64,{}", image.mime_type, base64(&image.bytes))}]}),
 })).collect()
}
fn base64(bytes: &[u8]) -> String {
 const TABLE: &[u8;64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
 let mut out=String::with_capacity((bytes.len()+2)/3*4);
 for chunk in bytes.chunks(3) { let a=chunk[0]; let b=*chunk.get(1).unwrap_or(&0); let c=*chunk.get(2).unwrap_or(&0); out.push(TABLE[(a>>2) as usize] as char); out.push(TABLE[((a&3)<<4 | b>>4) as usize] as char); out.push(if chunk.len()>1 {TABLE[((b&15)<<2 | c>>6) as usize] as char} else {'='}); out.push(if chunk.len()>2 {TABLE[(c&63) as usize] as char} else {'='}); }
 out
}
fn parse_response(value: Value) -> Result<AiProviderResponse, AiProviderError> {
 let output = value.get("output").and_then(Value::as_array).ok_or_else(|| AiProviderError::InvalidResponse("missing output array".into()))?;
 let mut content = Vec::new();
 for item in output {
  match item.get("type").and_then(Value::as_str) {
   Some("function_call") => { let id=item.get("call_id").and_then(Value::as_str).ok_or_else(||AiProviderError::InvalidResponse("function call missing call_id".into()))?; let name=item.get("name").and_then(Value::as_str).ok_or_else(||AiProviderError::InvalidResponse("function call missing name".into()))?; let raw=item.get("arguments").and_then(Value::as_str).ok_or_else(||AiProviderError::InvalidResponse("function call missing arguments".into()))?; let args=serde_json::from_str(raw).map_err(|e|AiProviderError::InvalidResponse(format!("invalid arguments for {name}: {e}")))?; content.push(AiContent::ToolCall(crate::ai::AiToolCall { call_id:id.into(), name:name.into(), arguments:args })); }
   Some("message") => if let Some(parts)=item.get("content").and_then(Value::as_array) { for part in parts { if part.get("type").and_then(Value::as_str)==Some("output_text") { if let Some(text)=part.get("text").and_then(Value::as_str) { content.push(AiContent::Text(text.into())); } } } },
   _ => {}
  }
 }
 if content.is_empty() { return Err(AiProviderError::InvalidResponse("response contained no text or function calls".into())); }
 Ok(AiProviderResponse { content })
}

fn parse_api_error(status: u16, value: &Value) -> AiProviderError {
    let error = &value["error"];
    let message = error["message"].as_str().unwrap_or("No error details returned by OpenAI").to_owned();
    match status {
        401 => AiProviderError::Unauthorized,
        429 if ["code", "type"].iter().any(|field| error[*field].as_str() == Some("insufficient_quota")) => {
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
            assert!(error.to_string().contains("You exceeded your current quota."));
        }
    }

    #[test]
    fn rate_limits_preserve_server_details() {
        let error = parse_api_error(429, &json!({"error": {
            "code": "rate_limit_exceeded",
            "message": "Tokens per minute exceeded. Please try again in 20s."
        }}));
        assert!(matches!(error, AiProviderError::RateLimited(_)));
        assert!(error.to_string().contains("Please try again in 20s."));
    }

    #[test]
    fn missing_error_details_do_not_panic() {
        assert!(matches!(parse_api_error(429, &json!({})), AiProviderError::RateLimited(_)));
        assert!(matches!(parse_api_error(401, &json!({})), AiProviderError::Unauthorized));
    }
}
