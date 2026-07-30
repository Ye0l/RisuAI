//! Provider abstraction.
//!
//! M0 ships one implementation (OpenAI-compatible). The trait exists so that M2-3
//! (Anthropic) and M2-4 (Gemini) slot in without touching the assembly pipeline —
//! upstream's equivalent split is `src/ts/process/request/`.

pub mod openai;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// `OpenAIChat` — `src/ts/process/index.svelte.ts:36`, minus multimodal fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Upstream's `memo`: an assembly-time tag (`"NewChat"`, `"supaMemory"`, …) used to
    /// decide what may be trimmed. Never sent to the provider.
    #[serde(skip)]
    pub memo: Option<String>,
    /// Upstream's `removable`: set on real chat history, which the context trimmer is
    /// allowed to drop.
    #[serde(skip)]
    pub removable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

impl ChatMessage {
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        ChatMessage {
            role,
            content: content.into(),
            name: None,
            memo: None,
            removable: false,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(ChatRole::System, content)
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_memo(mut self, memo: impl Into<String>) -> Self {
        self.memo = Some(memo.into());
        self
    }
}

/// Why the model stopped. The distinction matters: a reply cut off at the token cap
/// looks exactly like a short reply unless the provider's `finish_reason` is surfaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    /// Ran to a natural stop or a stop sequence.
    Stop,
    /// Hit `max_tokens`. The text is truncated mid-thought.
    Length,
    /// Provider-side filtering.
    ContentFilter,
    /// Anything else, kept verbatim so unusual gateways stay debuggable.
    Other(String),
    /// The provider did not report one.
    Unknown,
}

impl FinishReason {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            None => FinishReason::Unknown,
            // OpenAI says "stop"; some gateways say "end_turn" / "eos".
            Some("stop" | "end_turn" | "eos" | "STOP") => FinishReason::Stop,
            Some("length" | "max_tokens" | "MAX_TOKENS") => FinishReason::Length,
            Some("content_filter" | "SAFETY") => FinishReason::ContentFilter,
            Some(other) => FinishReason::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub content: String,
    pub finish_reason: FinishReason,
    pub usage: Usage,
}

/// Everything a provider needs for one completion.
pub struct ChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub model: &'a str,
    /// Already converted from upstream's 0-200 percent scale.
    pub temperature: f64,
    pub top_p: f64,
    pub frequency_penalty: f64,
    pub presence_penalty: f64,
    pub max_tokens: usize,
}

// Only ever used through a concrete type, so the lack of dyn-compatibility is fine.
#[allow(async_fn_in_trait)]
pub trait Provider {
    async fn send(&self, request: ChatRequest<'_>) -> Result<Completion>;
}
