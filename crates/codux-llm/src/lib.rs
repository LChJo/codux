//! Shared LLM provider completion core.
//!
//! Both the desktop runtime and the headless agent need to call an AI provider
//! (the agent runs memory extraction against a provider config the controller
//! forwards). This crate owns the genai integration so neither side duplicates
//! it. The desktop keeps its richer `AISettings`-aware provider *selection*; it
//! converts the chosen provider into [`LlmProvider`] before calling [`complete`].

use std::sync::OnceLock;
use std::time::Duration;

use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatResponseFormat, JsonSpec};
use genai::resolver::{AuthData, Endpoint};
use genai::{Client, Headers, ModelIden, ModelSpec, ServiceTarget, WebConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single AI provider, reduced to the fields the completion core needs. This
/// is the wire shape the controller forwards to a host for memory extraction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProvider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone)]
pub struct LlmJsonSchema {
    pub name: String,
    pub description: Option<String>,
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct LlmCompletionOptions {
    pub max_tokens: u32,
    pub temperature: f32,
    pub preserve_formatting: bool,
    pub json_response: bool,
    pub json_schema: Option<LlmJsonSchema>,
    pub timeout_seconds: u64,
}

impl Default for LlmCompletionOptions {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            temperature: 0.4,
            preserve_formatting: false,
            json_response: false,
            json_schema: None,
            timeout_seconds: 15,
        }
    }
}

/// Optional tracing hook. The desktop installs `runtime_trace`; the agent its
/// own logger. Installed once at startup; defaults to a no-op.
static TRACE_HOOK: OnceLock<fn(&str, &str)> = OnceLock::new();

/// Install a process-wide trace hook for provider calls (scope, message). The
/// first installation wins; later calls are ignored.
pub fn set_trace_hook(hook: fn(&str, &str)) {
    let _ = TRACE_HOOK.set(hook);
}

fn trace(scope: &str, message: &str) {
    if let Some(hook) = TRACE_HOOK.get() {
        hook(scope, message);
    }
}

/// Whether a provider kind can be driven by the completion core.
pub fn supports_completion(kind: &str) -> bool {
    provider_adapter_kind(kind).is_some()
}

/// Run a single chat completion against the provider. Returns the assistant
/// text, or an error string suitable for surfacing to the user.
pub async fn complete(
    provider: &LlmProvider,
    prompt: &str,
    system_prompt: Option<&str>,
    options: LlmCompletionOptions,
) -> Result<String, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Prompt cannot be empty.".to_string());
    }
    let adapter_kind = provider_adapter_kind(&provider.kind)
        .ok_or_else(|| "Unsupported AI provider kind.".to_string())?;
    let model = fallback_model(provider, default_model_for_provider_kind(&provider.kind));
    let service_target = genai_service_target(provider, adapter_kind, &model)?;
    trace(
        "ai-llm",
        &format!(
            "request start kind={} provider_id={} model={} base_url={} prompt_chars={} system_chars={} max_tokens={} temperature={:.2} json_response={} json_schema={} json_schema_supported={}",
            provider.kind,
            provider.id,
            model,
            provider.base_url,
            prompt.chars().count(),
            system_prompt
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.chars().count())
                .unwrap_or(0),
            options.max_tokens,
            options.temperature,
            options.json_response,
            options
                .json_schema
                .as_ref()
                .map(|schema| schema.name.as_str())
                .unwrap_or(""),
            options
                .json_schema
                .as_ref()
                .is_some_and(|_| provider_supports_json_schema_response_format(provider))
        ),
    );
    let mut request = ChatRequest::default();
    if let Some(system) = system_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.with_system(system);
    }
    request = request.append_message(ChatMessage::user(prompt));
    let client = Client::builder()
        .with_web_config(
            WebConfig::default().with_timeout(Duration::from_secs(options.timeout_seconds.max(1))),
        )
        .build();
    let chat_options = ChatOptions::default()
        .with_max_tokens(options.max_tokens)
        .with_temperature(f64::from(options.temperature))
        .with_capture_raw_body(true);
    let chat_options = if let Some(json_schema) = options
        .json_schema
        .as_ref()
        .filter(|_| provider_supports_json_schema_response_format(provider))
    {
        let mut spec = JsonSpec::new(json_schema.name.clone(), json_schema.schema.clone());
        if let Some(description) = json_schema.description.as_deref() {
            spec = spec.with_description(description);
        }
        chat_options.with_response_format(ChatResponseFormat::JsonSpec(spec))
    } else if options.json_response {
        chat_options.with_response_format(ChatResponseFormat::JsonMode)
    } else {
        chat_options
    };
    let response = client
        .exec_chat(
            ModelSpec::from_target(service_target),
            request,
            Some(&chat_options),
        )
        .await
        .map_err(|error| provider_call_error(provider, &model, error))?;
    let text = response.content.into_joined_texts().unwrap_or_default();
    let text = sanitize_provider_response(&text, options.preserve_formatting);
    if text.is_empty() {
        trace(
            "ai-llm",
            &format!("request empty provider_id={} model={}", provider.id, model),
        );
        Err("The AI provider returned an empty response.".to_string())
    } else {
        trace(
            "ai-llm",
            &format!(
                "request ok kind={} provider_id={} model={} text_chars={}",
                provider.kind,
                provider.id,
                model,
                text.chars().count()
            ),
        );
        Ok(text)
    }
}

fn required_api_key(provider: &LlmProvider) -> Result<&str, String> {
    let api_key = provider.api_key.trim();
    if api_key.is_empty() && !provider_kind_allows_empty_api_key(&provider.kind) {
        Err("The selected AI provider is missing an API key.".to_string())
    } else {
        Ok(api_key)
    }
}

fn fallback_model(provider: &LlmProvider, fallback: &str) -> String {
    let model = provider.model.trim();
    if model.is_empty() {
        fallback.to_string()
    } else {
        model.to_string()
    }
}

fn provider_adapter_kind(kind: &str) -> Option<AdapterKind> {
    match kind {
        "openai" | "openAICompatible" => Some(AdapterKind::OpenAI),
        "anthropic" | "kimi" | "kimiCode" => Some(AdapterKind::Anthropic),
        "deepseek" => Some(AdapterKind::DeepSeek),
        "gemini" => Some(AdapterKind::Gemini),
        "groq" => Some(AdapterKind::Groq),
        "openrouter" => Some(AdapterKind::OpenRouter),
        "ollama" | "localLlama" => Some(AdapterKind::Ollama),
        _ => None,
    }
}

fn provider_kind_allows_empty_api_key(kind: &str) -> bool {
    matches!(kind, "ollama" | "localLlama")
}

fn provider_supports_json_schema_response_format(provider: &LlmProvider) -> bool {
    matches!(provider.kind.as_str(), "openai" | "anthropic" | "gemini")
}

fn default_model_for_provider_kind(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "claude-3-5-haiku-latest",
        "kimi" => "kimi-k3",
        "kimiCode" => "kimi-for-coding",
        "deepseek" => "deepseek-chat",
        "gemini" => "gemini-2.5-flash",
        "groq" => "llama-3.3-70b-versatile",
        "openrouter" => "openai/gpt-4.1-mini",
        "ollama" | "localLlama" => "llama3.2",
        _ => "gpt-4.1-mini",
    }
}

fn default_endpoint_for_provider_kind(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "https://api.anthropic.com/v1/",
        "kimi" => "https://api.moonshot.cn/anthropic/",
        "kimiCode" => "https://api.kimi.com/coding/",
        "deepseek" => "https://api.deepseek.com/",
        "gemini" => "https://generativelanguage.googleapis.com/v1beta/",
        "groq" => "https://api.groq.com/openai/v1/",
        "openrouter" => "https://openrouter.ai/api/v1/",
        "ollama" | "localLlama" => "http://localhost:11434",
        _ => "https://api.openai.com/v1/",
    }
}

fn genai_service_target(
    provider: &LlmProvider,
    adapter_kind: AdapterKind,
    model: &str,
) -> Result<ServiceTarget, String> {
    let endpoint = provider_endpoint(provider)?;
    let auth = if provider.kind == "kimi" {
        let api_key = required_api_key(provider)?;
        AuthData::RequestOverride {
            url: anthropic_compatible_messages_url(&endpoint),
            headers: Headers::from(vec![
                ("Authorization", format!("Bearer {api_key}")),
                ("anthropic-version", "2023-06-01".to_string()),
            ]),
        }
    } else if provider.kind == "kimiCode" {
        let api_key = required_api_key(provider)?;
        AuthData::RequestOverride {
            url: anthropic_compatible_messages_url(&endpoint),
            headers: Headers::from(vec![
                ("x-api-key", api_key.to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ]),
        }
    } else if provider_kind_allows_empty_api_key(&provider.kind) {
        AuthData::None
    } else {
        AuthData::from_single(required_api_key(provider)?.to_string())
    };
    Ok(ServiceTarget {
        endpoint: Endpoint::from_owned(endpoint),
        auth,
        model: ModelIden::new(adapter_kind, model),
    })
}

fn anthropic_compatible_messages_url(endpoint: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with("/v1") {
        format!("{endpoint}/messages")
    } else {
        format!("{endpoint}/v1/messages")
    }
}

fn provider_endpoint(provider: &LlmProvider) -> Result<String, String> {
    let endpoint = normalized_provider_base_url(
        &provider.base_url,
        default_endpoint_for_provider_kind(&provider.kind),
    );
    if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
        return Err("The selected AI provider has an invalid base URL.".to_string());
    }
    Ok(endpoint)
}

fn normalized_provider_base_url(base_url: &str, fallback: &str) -> String {
    let trimmed = base_url.trim();
    let value = if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    };
    if value.ends_with('/') {
        value.to_string()
    } else {
        format!("{value}/")
    }
}

fn provider_call_error(provider: &LlmProvider, model: &str, error: genai::Error) -> String {
    let error = sanitize_response_line(&error.to_string());
    trace(
        "ai-llm",
        &format!(
            "request failed kind={} provider_id={} model={} error={}",
            provider.kind, provider.id, model, error
        ),
    );
    let mut message = format!(
        "{} request failed for model {}: {}",
        provider.display_name, model, error
    );
    if let Some(hint) = provider_authentication_hint(&provider.kind, &error) {
        message.push_str(" Hint: ");
        message.push_str(hint);
    }
    message
}

fn provider_authentication_hint(kind: &str, error: &str) -> Option<&'static str> {
    if !error.contains("401") && !error.contains("Invalid Authentication") {
        return None;
    }
    match kind {
        "kimi" => Some(
            "This endpoint requires a Kimi Open Platform key from platform.kimi.com; Kimi Code membership keys must use the Kimi Code provider.",
        ),
        "kimiCode" => Some(
            "This endpoint requires a membership key from the Kimi Code Console; Kimi Open Platform keys must use the Kimi API provider.",
        ),
        _ => None,
    }
}

fn sanitize_provider_response(text: &str, preserve_formatting: bool) -> String {
    if preserve_formatting {
        text.trim()
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '“' | '”' | '‘' | '’'))
            .to_string()
    } else {
        sanitize_response_line(text)
    }
}

/// Collapse a provider response (or error) to a single trimmed line, capped.
pub fn sanitize_response_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '“' | '”' | '‘' | '’'))
        .chars()
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_provider_uses_anthropic_messages_with_bearer_auth() {
        let provider = LlmProvider {
            id: "kimi-test".to_string(),
            kind: "kimi".to_string(),
            display_name: "Kimi API".to_string(),
            model: String::new(),
            base_url: "https://api.moonshot.cn/anthropic".to_string(),
            api_key: "moonshot-key".to_string(),
        };

        assert!(supports_completion("kimi"));
        assert!(matches!(
            provider_adapter_kind("kimi"),
            Some(AdapterKind::Anthropic)
        ));
        assert_eq!(default_model_for_provider_kind("kimi"), "kimi-k3");
        assert_eq!(
            default_endpoint_for_provider_kind("kimi"),
            "https://api.moonshot.cn/anthropic/"
        );

        let target = genai_service_target(&provider, AdapterKind::Anthropic, "kimi-k3")
            .expect("kimi service target");
        let AuthData::RequestOverride { url, headers } = target.auth else {
            panic!("kimi must override Anthropic x-api-key auth");
        };
        assert_eq!(url, "https://api.moonshot.cn/anthropic/v1/messages");
        let headers = headers.iter().collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            headers
                .get(&"Authorization".to_string())
                .map(|value| value.as_str()),
            Some("Bearer moonshot-key")
        );
        assert_eq!(
            headers
                .get(&"anthropic-version".to_string())
                .map(|value| value.as_str()),
            Some("2023-06-01")
        );
        assert!(!headers.contains_key(&"x-api-key".to_string()));
    }

    #[test]
    fn kimi_code_provider_uses_anthropic_messages_with_x_api_key() {
        let provider = LlmProvider {
            id: "kimi-code-test".to_string(),
            kind: "kimiCode".to_string(),
            display_name: "Kimi Code".to_string(),
            model: String::new(),
            base_url: "https://api.kimi.com/coding/".to_string(),
            api_key: "kimi-code-key".to_string(),
        };

        assert!(supports_completion("kimiCode"));
        assert!(matches!(
            provider_adapter_kind("kimiCode"),
            Some(AdapterKind::Anthropic)
        ));
        assert_eq!(
            default_model_for_provider_kind("kimiCode"),
            "kimi-for-coding"
        );
        assert_eq!(
            default_endpoint_for_provider_kind("kimiCode"),
            "https://api.kimi.com/coding/"
        );

        let target = genai_service_target(&provider, AdapterKind::Anthropic, "kimi-for-coding")
            .expect("kimi code service target");
        let AuthData::RequestOverride { url, headers } = target.auth else {
            panic!("kimi code must use its Anthropic-compatible endpoint");
        };
        assert_eq!(url, "https://api.kimi.com/coding/v1/messages");
        let headers = headers.iter().collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            headers
                .get(&"x-api-key".to_string())
                .map(|value| value.as_str()),
            Some("kimi-code-key")
        );
        assert_eq!(
            headers
                .get(&"anthropic-version".to_string())
                .map(|value| value.as_str()),
            Some("2023-06-01")
        );
        assert!(!headers.contains_key(&"Authorization".to_string()));
    }

    #[test]
    fn kimi_authentication_hint_distinguishes_key_products() {
        let error = "Request failed with status code '401 Unauthorized'. Invalid Authentication";
        assert_eq!(
            provider_authentication_hint("kimi", error),
            Some("This endpoint requires a Kimi Open Platform key from platform.kimi.com; Kimi Code membership keys must use the Kimi Code provider.")
        );
        assert_eq!(
            provider_authentication_hint("kimiCode", error),
            Some("This endpoint requires a membership key from the Kimi Code Console; Kimi Open Platform keys must use the Kimi API provider.")
        );
        assert_eq!(
            provider_authentication_hint("kimi", "429 Too Many Requests"),
            None
        );
    }
}
