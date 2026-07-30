//! Token counting — **M0 stub**.
//!
//! A character-count heuristic so that `maxContext` trimming does something. It is not
//! accurate and must not be used for anything that reports token usage to the user.
//! M1-4 replaces this with `tiktoken-rs` (already a dependency of `src-tauri`).

use crate::provider::ChatMessage;

/// Rough tokens-per-character ratio for English. Wildly wrong for CJK, which is the
/// main reason this is a stub and not a solution.
const CHARS_PER_TOKEN: usize = 4;

pub fn approx(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

/// Upstream's `ChatTokenizer.tokenizeChat` adds a per-message overhead for the role and
/// separator framing; `chatAdditonalTokens` defaults to 1.
const PER_MESSAGE_OVERHEAD: usize = 4;

pub fn approx_message(message: &ChatMessage) -> usize {
    approx(&message.content) + PER_MESSAGE_OVERHEAD
}

pub fn approx_messages(messages: &[ChatMessage]) -> usize {
    messages.iter().map(approx_message).sum()
}
