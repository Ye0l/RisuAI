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
    /// Returns the assistant's reply text.
    async fn send(&self, request: ChatRequest<'_>) -> Result<String>;
}
