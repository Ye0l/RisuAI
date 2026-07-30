//! OpenAI-compatible chat completions (M0-5).
//!
//! Derived from `src/ts/process/request/openAI/requests.ts`, reduced to the
//! non-streaming path. Any gateway speaking this dialect works by changing `baseURL`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ChatRequest, Completion, FinishReason, Provider, Usage};
use crate::config::ApiConfig;

pub struct OpenAiProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl OpenAiProvider {
    pub fn new(config: &ApiConfig) -> Self {
        OpenAiProvider {
            client: reqwest::Client::new(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
        }
    }
}

#[derive(Deserialize)]
struct CompletionResponse {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<UsageBody>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    message: Option<ResponseMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct UsageBody {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

impl Provider for OpenAiProvider {
    async fn send(&self, request: ChatRequest<'_>) -> Result<Completion> {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|m| {
                // `name` is dropped: the OpenAI API restricts it to
                // `^[a-zA-Z0-9_-]+$`, and character names routinely violate that.
                // Upstream gates it behind `promptSettings.sendName` for the same
                // reason. Sending names is M2 work.
                json!({
                    "role": match m.role {
                        super::ChatRole::System => "system",
                        super::ChatRole::User => "user",
                        super::ChatRole::Assistant => "assistant",
                    },
                    "content": m.content,
                })
            })
            .collect();

        let body = json!({
            "model": request.model,
            "messages": messages,
            "temperature": request.temperature,
            "top_p": request.top_p,
            "frequency_penalty": request.frequency_penalty,
            "presence_penalty": request.presence_penalty,
            "max_tokens": request.max_tokens,
        });

        let url = format!("{}/chat/completions", self.base_url);
        let mut builder = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            builder = builder.bearer_auth(&self.api_key);
        }

        let response = builder
            .send()
            .await
            .with_context(|| format!("request to {url} failed"))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .context("could not read response body")?;

        if !status.is_success() {
            bail!("{status} from {url}: {}", truncate(&text, 2000));
        }

        let parsed: CompletionResponse = serde_json::from_str(&text)
            .with_context(|| format!("unexpected response shape: {}", truncate(&text, 500)))?;

        let usage = Usage {
            prompt_tokens: parsed.usage.as_ref().and_then(|u| u.prompt_tokens),
            completion_tokens: parsed.usage.as_ref().and_then(|u| u.completion_tokens),
        };

        let choice = parsed.choices.into_iter().next();
        let finish_reason =
            FinishReason::parse(choice.as_ref().and_then(|c| c.finish_reason.as_deref()));
        let content = choice
            .and_then(|c| c.message)
            .and_then(|m| m.content)
            .unwrap_or_default();

        if content.trim().is_empty() {
            // Distinguish the failure modes: an empty body with `length` means the
            // whole budget went somewhere other than visible text — typically a
            // reasoning model spending it on hidden tokens.
            match finish_reason {
                FinishReason::Length => bail!(
                    "model returned no text and stopped at max_tokens ({}) — if this is \
                     a reasoning model, the budget was consumed before it wrote anything; \
                     raise maxResponse",
                    request.max_tokens
                ),
                FinishReason::ContentFilter => {
                    bail!("model returned no text: the provider's content filter blocked it")
                }
                _ => bail!("model returned an empty response"),
            }
        }

        Ok(Completion {
            content,
            finish_reason,
            usage,
        })
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("한글이다", 2), "한글…");
    }

    #[test]
    fn parses_the_finish_reasons_that_matter() {
        assert_eq!(FinishReason::parse(Some("stop")), FinishReason::Stop);
        assert_eq!(FinishReason::parse(Some("length")), FinishReason::Length);
        assert_eq!(
            FinishReason::parse(Some("content_filter")),
            FinishReason::ContentFilter
        );
        assert_eq!(FinishReason::parse(None), FinishReason::Unknown);
        assert_eq!(
            FinishReason::parse(Some("tool_calls")),
            FinishReason::Other("tool_calls".to_string())
        );
    }

    #[test]
    fn accepts_the_aliases_other_gateways_use() {
        // Anthropic-style and Gemini-style gateways proxied behind an OpenAI shape.
        assert_eq!(FinishReason::parse(Some("end_turn")), FinishReason::Stop);
        assert_eq!(FinishReason::parse(Some("MAX_TOKENS")), FinishReason::Length);
        assert_eq!(
            FinishReason::parse(Some("SAFETY")),
            FinishReason::ContentFilter
        );
    }

    #[test]
    fn deserializes_a_truncated_response() {
        let body = r#"{
            "choices": [{
                "message": {"role":"assistant","content":"She turns, and"},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 412, "completion_tokens": 500}
        }"#;
        let parsed: CompletionResponse = serde_json::from_str(body).unwrap();
        assert_eq!(
            FinishReason::parse(parsed.choices[0].finish_reason.as_deref()),
            FinishReason::Length
        );
        assert_eq!(parsed.usage.unwrap().completion_tokens, Some(500));
    }

    #[test]
    fn tolerates_a_response_with_no_usage_or_reason() {
        let body = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        let parsed: CompletionResponse = serde_json::from_str(body).unwrap();
        assert!(parsed.usage.is_none());
        assert_eq!(
            FinishReason::parse(parsed.choices[0].finish_reason.as_deref()),
            FinishReason::Unknown
        );
    }
}
