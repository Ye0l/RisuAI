//! Preset and user settings.
//!
//! Upstream keeps these on the single flat `Database` object, mixing per-preset fields
//! (`botPreset`, `database.svelte.ts:1580`) with user settings. This splits them into
//! `Preset` and the surrounding `Config`, which is the only meaningful divergence in
//! naming — the field names themselves match upstream.

use serde::{Deserialize, Serialize};

/// `FormatingOrderItem` — `database.svelte.ts:1813`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatingOrderItem {
    #[serde(rename = "main")]
    Main,
    #[serde(rename = "jailbreak")]
    Jailbreak,
    #[serde(rename = "chats")]
    Chats,
    #[serde(rename = "lorebook")]
    Lorebook,
    #[serde(rename = "globalNote")]
    GlobalNote,
    #[serde(rename = "authorNote")]
    AuthorNote,
    #[serde(rename = "lastChat")]
    LastChat,
    #[serde(rename = "description")]
    Description,
    #[serde(rename = "postEverything")]
    PostEverything,
    #[serde(rename = "personaPrompt")]
    PersonaPrompt,
}

/// Subset of upstream's `botPreset`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "mainPrompt", default)]
    pub main_prompt: String,
    #[serde(default)]
    pub jailbreak: String,
    #[serde(rename = "globalNote", default)]
    pub global_note: String,
    #[serde(rename = "formatingOrder", default = "default_formating_order")]
    pub formating_order: Vec<FormatingOrderItem>,
    #[serde(rename = "maxContext", default = "default_max_context")]
    pub max_context: usize,
    #[serde(rename = "maxResponse", default = "default_max_response")]
    pub max_response: usize,
    /// Percent, as upstream stores it (80 => 0.8).
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(rename = "frequencyPenalty", default)]
    pub frequency_penalty: f64,
    #[serde(rename = "PresensePenalty", default)]
    pub presence_penalty: f64,
    #[serde(rename = "top_p", default = "default_top_p")]
    pub top_p: f64,
}

fn default_formating_order() -> Vec<FormatingOrderItem> {
    use FormatingOrderItem::*;
    // Matches `presetTemplate.formatingOrder`, `database.svelte.ts:1999`.
    vec![
        Main,
        Description,
        PersonaPrompt,
        Chats,
        LastChat,
        Jailbreak,
        Lorebook,
        GlobalNote,
        AuthorNote,
    ]
}

fn default_max_context() -> usize {
    // `database.svelte.ts:56`.
    4000
}
fn default_max_response() -> usize {
    // `database.svelte.ts:59`. Note this is the live default a fresh RisuAI install
    // gets; `presetTemplate.maxResponse` is 300 but the DB-level field wins. Still
    // small for modern models — expect to raise it.
    500
}
fn default_temperature() -> f64 {
    80.0
}
fn default_top_p() -> f64 {
    1.0
}

impl Default for Preset {
    fn default() -> Self {
        Preset {
            name: "Default".to_string(),
            // Upstream's shipped default is `prebuiltPresets.OAI.mainPrompt`. This is a
            // neutral stand-in; point `mainPrompt` at your own text in config.json.
            main_prompt: "Write {{char}}'s next reply in a fictional roleplay between \
                          {{char}} and {{user}}. Stay in character, write in third \
                          person, and never speak or act as {{user}}."
                .to_string(),
            jailbreak: String::new(),
            global_note: String::new(),
            formating_order: default_formating_order(),
            max_context: default_max_context(),
            max_response: default_max_response(),
            temperature: default_temperature(),
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            top_p: default_top_p(),
        }
    }
}

/// Where requests go. M0 speaks only the OpenAI-compatible chat-completions dialect, so
/// any gateway exposing that (OpenAI, OpenRouter, Ollama, llama.cpp, a proxy) works by
/// changing `baseURL`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(rename = "baseURL", default = "default_base_url")]
    pub base_url: String,
    #[serde(rename = "apiKey", default)]
    pub api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_model() -> String {
    "gpt-4o-mini".to_string()
}

impl Default for ApiConfig {
    fn default() -> Self {
        ApiConfig {
            base_url: default_base_url(),
            api_key: String::new(),
            model: default_model(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "username", default = "default_username")]
    pub username: String,
    #[serde(rename = "personaPrompt", default)]
    pub persona_prompt: String,
    #[serde(rename = "jailbreakToggle", default)]
    pub jailbreak_toggle: bool,
    /// Upstream default is `'description of {{char}}: '` (`database.svelte.ts:95`) but
    /// it is only prepended when `promptPreprocess` is on.
    #[serde(rename = "descriptionPrefix", default = "default_description_prefix")]
    pub description_prefix: String,
    #[serde(rename = "promptPreprocess", default)]
    pub prompt_preprocess: bool,
    #[serde(rename = "additionalPrompt", default)]
    pub additional_prompt: String,
    #[serde(rename = "authorNoteDefaultText", default)]
    pub author_note_default_text: String,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub preset: Preset,
}

fn default_username() -> String {
    "User".to_string()
}
fn default_description_prefix() -> String {
    "description of {{char}}: ".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            username: default_username(),
            persona_prompt: String::new(),
            jailbreak_toggle: false,
            description_prefix: default_description_prefix(),
            prompt_preprocess: false,
            additional_prompt: String::new(),
            author_note_default_text: String::new(),
            api: ApiConfig::default(),
            preset: Preset::default(),
        }
    }
}
