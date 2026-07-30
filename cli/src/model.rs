//! Core data model.
//!
//! A deliberate subset of the upstream TypeScript interfaces in
//! `src/ts/storage/database.svelte.ts`. Field names are kept identical to upstream so
//! that RisuAI-exported JSON round-trips, and so M3's `.bin` reader can deserialize
//! into the same structs. Fields belonging to dropped subsystems (TTS, Stable
//! Diffusion, emotion images, assets, 3D) are not modelled; `serde` ignores them on
//! read and they are simply absent on write.

use serde::{Deserialize, Serialize};

fn is_false(b: &bool) -> bool {
    !*b
}

/// `character` — `database.svelte.ts:1342`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub name: String,
    #[serde(rename = "chaId")]
    pub cha_id: String,
    #[serde(rename = "firstMessage", default)]
    pub first_message: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub scenario: String,
    #[serde(rename = "exampleMessage", default)]
    pub example_message: String,
    #[serde(rename = "systemPrompt", default)]
    pub system_prompt: String,
    /// Upstream calls this `replaceGlobalNote`; it overrides the preset's global note
    /// and may embed `{{original}}`.
    #[serde(rename = "replaceGlobalNote", default)]
    pub replace_global_note: String,
    #[serde(rename = "postHistoryInstructions", default)]
    pub post_history_instructions: String,
    #[serde(rename = "creatorNotes", default)]
    pub creator_notes: String,
    #[serde(default)]
    pub creator: String,
    #[serde(rename = "characterVersion", default)]
    pub character_version: String,
    #[serde(rename = "alternateGreetings", default)]
    pub alternate_greetings: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(rename = "utilityBot", default, skip_serializing_if = "is_false")]
    pub utility_bot: bool,

    /// Not consumed until M1-2.
    #[serde(rename = "globalLore", default)]
    pub global_lore: Vec<LoreBook>,
    /// Not consumed until M1-3.
    #[serde(default)]
    pub customscript: Vec<CustomScript>,

    #[serde(default = "default_chats")]
    pub chats: Vec<Chat>,
    #[serde(rename = "chatPage", default)]
    pub chat_page: usize,
}

fn default_chats() -> Vec<Chat> {
    vec![Chat::new("Chat 1")]
}

impl Character {
    pub fn new(name: impl Into<String>) -> Self {
        Character {
            name: name.into(),
            cha_id: uuid::Uuid::new_v4().to_string(),
            first_message: String::new(),
            desc: String::new(),
            personality: String::new(),
            scenario: String::new(),
            example_message: String::new(),
            system_prompt: String::new(),
            replace_global_note: String::new(),
            post_history_instructions: String::new(),
            creator_notes: String::new(),
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: Vec::new(),
            tags: Vec::new(),
            utility_bot: false,
            global_lore: Vec::new(),
            customscript: Vec::new(),
            chats: default_chats(),
            chat_page: 0,
        }
    }

    /// The chat currently selected by `chatPage`, clamped into range.
    pub fn current_chat(&self) -> &Chat {
        let idx = self.chat_page.min(self.chats.len().saturating_sub(1));
        &self.chats[idx]
    }

    pub fn current_chat_mut(&mut self) -> &mut Chat {
        if self.chats.is_empty() {
            self.chats.push(Chat::new("Chat 1"));
        }
        let idx = self.chat_page.min(self.chats.len() - 1);
        &mut self.chats[idx]
    }
}

/// `Chat` — `database.svelte.ts:1815`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub name: String,
    #[serde(default)]
    pub message: Vec<Message>,
    /// Author's note for this chat.
    #[serde(default)]
    pub note: String,
    #[serde(rename = "localLore", default)]
    pub local_lore: Vec<LoreBook>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl Chat {
    pub fn new(name: impl Into<String>) -> Self {
        Chat {
            name: name.into(),
            message: Vec::new(),
            note: String::new(),
            local_lore: Vec::new(),
            id: Some(uuid::Uuid::new_v4().to_string()),
        }
    }
}

/// Upstream stores roles as the literal strings `"user"` and `"char"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "char")]
    Char,
}

/// `Message` — `database.svelte.ts:1846`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub data: String,
    #[serde(rename = "chatId", default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
    /// `false | true | 'allBefore'` upstream. `Some("allBefore")` truncates history at
    /// this point; `Some("true")` hides just this message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<serde_json::Value>,
}

impl Message {
    pub fn new(role: Role, data: impl Into<String>) -> Self {
        Message {
            role,
            data: data.into(),
            chat_id: Some(uuid::Uuid::new_v4().to_string()),
            time: Some(chrono::Utc::now().timestamp_millis()),
            disabled: None,
        }
    }

    /// Mirrors upstream's `d.disabled === true` check.
    pub fn is_disabled(&self) -> bool {
        matches!(&self.disabled, Some(serde_json::Value::Bool(true)))
    }

    /// Mirrors upstream's `d.disabled === 'allBefore'` check, which stops the history
    /// walk entirely.
    pub fn is_all_before(&self) -> bool {
        matches!(&self.disabled, Some(serde_json::Value::String(s)) if s == "allBefore")
    }
}

/// `loreBook` — `database.svelte.ts:1319`. Carried through import/export in M0 but not
/// yet activated; the engine lands in M1-2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoreBook {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub secondkey: String,
    #[serde(default)]
    pub insertorder: i64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub content: String,
    #[serde(default = "default_lore_mode")]
    pub mode: String,
    #[serde(rename = "alwaysActive", default)]
    pub always_active: bool,
    #[serde(default)]
    pub selective: bool,
    #[serde(rename = "useRegex", default)]
    pub use_regex: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

fn default_lore_mode() -> String {
    "normal".to_string()
}

/// `customscript` — `database.svelte.ts:1307`. Applied in M1-3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomScript {
    #[serde(default)]
    pub comment: String,
    #[serde(rename = "in", default)]
    pub input: String,
    #[serde(rename = "out", default)]
    pub output: String,
    /// `editinput` | `editoutput` | `editprocess` | `editdisplay`
    #[serde(rename = "type", default)]
    pub script_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
}
