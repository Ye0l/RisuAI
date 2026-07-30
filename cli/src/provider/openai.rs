//! OpenAI-compatible chat completions (M0-5).
//!
//! Derived from `src/ts/process/request/openAI/requests.ts`, reduced to the
//! non-streaming path. Any gateway speaking this dialect works by changing `baseURL`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ChatRequest, Provider};
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
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    message: Option<ResponseMessage>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
}

impl Provider for OpenAiProvider {
    async fn send(&self, request: ChatRequest<'_>) -> Result<String> {
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

        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .and_then(|m| m.content)
            .unwrap_or_default();

        if content.trim().is_empty() {
            bail!("model returned an empty response");
        }
        Ok(content)
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
}
