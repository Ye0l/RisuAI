//! Prompt assembly — the legacy `formatingOrder` path (M0-4).
//!
//! Ports `src/ts/process/index.svelte.ts`: the `unformated` bucket struct at line 317
//! is filled in at 376-600, chat history is built at 799-1140, and the buckets are
//! concatenated in `formatingOrder` sequence at 1432.
//!
//! Not ported in M0: the `promptTemplate` path (M2-1), lorebooks (M1-2), memory,
//! modules, triggers, depth prompts, group chat, multimodal inlays.

use crate::cbs::{self, CbsContext};
use crate::config::{Config, FormatingOrderItem};
use crate::model::{Character, Chat, Message, Role};
use crate::provider::{ChatMessage, ChatRole};
use crate::token;

/// The named buckets of `let unformated = {...}` (`index.svelte.ts:317`).
#[derive(Default)]
struct Buckets {
    main: Vec<ChatMessage>,
    jailbreak: Vec<ChatMessage>,
    chats: Vec<ChatMessage>,
    lorebook: Vec<ChatMessage>,
    global_note: Vec<ChatMessage>,
    author_note: Vec<ChatMessage>,
    last_chat: Vec<ChatMessage>,
    description: Vec<ChatMessage>,
    post_everything: Vec<ChatMessage>,
    persona_prompt: Vec<ChatMessage>,
}

impl Buckets {
    fn get(&self, item: FormatingOrderItem) -> &[ChatMessage] {
        use FormatingOrderItem::*;
        match item {
            Main => &self.main,
            Jailbreak => &self.jailbreak,
            Chats => &self.chats,
            Lorebook => &self.lorebook,
            GlobalNote => &self.global_note,
            AuthorNote => &self.author_note,
            LastChat => &self.last_chat,
            Description => &self.description,
            PostEverything => &self.post_everything,
            PersonaPrompt => &self.persona_prompt,
        }
    }
}

pub struct AssembleResult {
    pub messages: Vec<ChatMessage>,
    /// Number of history messages dropped to fit `maxContext`.
    pub trimmed: usize,
    /// Tokens by which the prompt still exceeds `maxContext` after trimming. Non-zero
    /// means `maxResponse` plus the static blocks leave no room for the conversation.
    pub over_budget: usize,
    /// `formatingOrder` omits `chats`, so no history is emitted at all.
    pub drops_history: bool,
}

/// A new chat opens with the character's greeting, as the UI does.
///
/// Shared with `prompt`/`--wire` rather than living in the REPL: a preview of a
/// not-yet-started chat would otherwise omit the greeting and misrepresent what a real
/// turn sends.
pub fn seed_first_message(character: &mut Character, config: &Config) {
    if !character.current_chat().message.is_empty() || character.first_message.is_empty() {
        return;
    }
    let ctx = CbsContext::from_character(character, &config.username, &config.persona_prompt);
    let greeting = cbs::parse(&character.first_message, &ctx);
    character
        .current_chat_mut()
        .message
        .push(Message::new(Role::Char, greeting));
}

/// Build the full prompt for the next assistant turn.
pub fn assemble(character: &Character, config: &Config) -> AssembleResult {
    let chat = character.current_chat();
    let preset = &config.preset;
    let ctx = CbsContext::from_character(character, &config.username, &config.persona_prompt);
    let parse = |text: &str| cbs::parse(text, &ctx);

    let mut buckets = Buckets::default();

    // --- main / jailbreak / globalNote (index.svelte.ts:378-408) ---------------
    // Upstream skips all three for utility bots and when a promptTemplate is active.
    if !character.utility_bot {
        let main = if character.system_prompt.is_empty() {
            preset.main_prompt.clone()
        } else {
            character
                .system_prompt
                .replace("{{original}}", &preset.main_prompt)
        };
        let main = if config.prompt_preprocess && !config.additional_prompt.is_empty() {
            format!("{main}\n{}", config.additional_prompt)
        } else {
            main
        };
        buckets.main = format_prompt(&parse(&main));

        if config.jailbreak_toggle {
            buckets.jailbreak = format_prompt(&parse(&preset.jailbreak));
        }

        let global_note = if character.replace_global_note.is_empty() {
            preset.global_note.clone()
        } else {
            character
                .replace_global_note
                .replace("{{original}}", &preset.global_note)
        };
        buckets.global_note = format_prompt(&parse(&global_note));
    }

    // --- author's note (index.svelte.ts:413-424) -------------------------------
    // The chat's own note wins; otherwise fall back to the configured default.
    let author_note = if !chat.note.is_empty() {
        chat.note.as_str()
    } else {
        config.author_note_default_text.as_str()
    };
    if !author_note.is_empty() {
        buckets.author_note.push(ChatMessage::system(parse(author_note)));
    }

    // --- description (index.svelte.ts:440-456) ---------------------------------
    {
        let prefix = if config.prompt_preprocess {
            config.description_prefix.as_str()
        } else {
            ""
        };
        let mut description = parse(&format!("{prefix}{}", character.desc));
        if !character.personality.is_empty() {
            description.push_str(&parse(&format!(
                "\n\nDescription of {{{{char}}}}: {}",
                character.personality
            )));
        }
        if !character.scenario.is_empty() {
            description.push_str(&parse(&format!(
                "\n\nCircumstances and context of the dialogue: {}",
                character.scenario
            )));
        }
        buckets.description.push(ChatMessage::system(description));
    }

    // --- persona (index.svelte.ts:528) -----------------------------------------
    if !config.persona_prompt.is_empty() {
        buckets
            .persona_prompt
            .push(ChatMessage::system(parse(&config.persona_prompt)));
    }

    // --- chat history (index.svelte.ts:799-1140) -------------------------------
    let mut chats = example_messages(character, config);
    chats.push(ChatMessage::system("[Start a new chat]").with_memo("NewChat"));
    chats.extend(history_messages(character, chat, config));

    // Context trimming (index.svelte.ts:1110-1120). Upstream reserves maxResponse
    // plus a 50-token cushion, then drops from the front until it fits.
    let budget = preset.max_context;
    let fixed = preset.max_response + 50 + count_buckets(&buckets);
    let mut chat_tokens = token::approx_messages(&chats);
    let mut trimmed = 0;
    // The newest history turn is the message being replied to; trimming it away sends
    // a prompt that silently omits what the user just typed. Keep at least one.
    let mut removable = chats.iter().filter(|m| m.removable).count();
    while fixed + chat_tokens > budget && removable > 1 {
        let Some(index) = chats.iter().position(|m| m.removable) else {
            break;
        };
        chat_tokens -= token::approx_message(&chats[index]);
        chats.remove(index);
        removable -= 1;
        trimmed += 1;
    }
    // What could not be trimmed away. Non-zero means the fixed overhead alone
    // (maxResponse plus the static prompt blocks) does not leave room for the
    // conversation, which is a configuration problem, not something to paper over.
    let over_budget = (fixed + chat_tokens).saturating_sub(budget);

    // --- concatenate in formatingOrder, then postEverything (1190-1193, 1432) ---
    let mut order = preset.formating_order.clone();
    if !order.contains(&FormatingOrderItem::PostEverything) {
        order.push(FormatingOrderItem::PostEverything);
    }

    // `lastChat` is split off the tail so `formatingOrder` can place trailing
    // instructions after the final user turn (index.svelte.ts:1132). Only split it off
    // if the order actually emits that bucket — otherwise the newest message, normally
    // the one just typed, would be dropped on the floor.
    if order.contains(&FormatingOrderItem::LastChat) {
        if let Some(last) = chats.pop() {
            buckets.last_chat.push(last);
        }
    }
    buckets.chats = chats
        .into_iter()
        .filter(|m| !m.content.trim().is_empty())
        .collect();

    let mut messages: Vec<ChatMessage> = Vec::new();
    for item in &order {
        push_prompts(&mut messages, buckets.get(*item));
    }

    for message in &mut messages {
        message.content = message.content.trim().to_string();
    }

    AssembleResult {
        messages,
        trimmed,
        over_budget,
        drops_history: !order.contains(&FormatingOrderItem::Chats),
    }
}

fn count_buckets(buckets: &Buckets) -> usize {
    use FormatingOrderItem::*;
    [
        Main,
        Jailbreak,
        Lorebook,
        GlobalNote,
        AuthorNote,
        Description,
        PostEverything,
        PersonaPrompt,
    ]
    .iter()
    .map(|item| token::approx_messages(buckets.get(*item)))
    .sum()
}

/// `pushPrompts` (`index.svelte.ts:1203`): drop blanks, and coalesce adjacent system
/// messages that share a `memo` and `name`.
fn push_prompts(out: &mut Vec<ChatMessage>, incoming: &[ChatMessage]) {
    for message in incoming {
        if message.content.trim().is_empty() {
            continue;
        }
        if message.role == ChatRole::System {
            if let Some(last) = out.last_mut() {
                if last.role == ChatRole::System
                    && last.memo == message.memo
                    && last.name == message.name
                {
                    last.content.push_str("\n\n");
                    last.content.push_str(&message.content);
                    continue;
                }
            }
        }
        out.push(message.clone());
    }
}

/// `formatPrompt` (`index.svelte.ts:381`): split a prompt string on `@@role` /
/// `@@@role` markers. Text with no leading marker is treated as system.
fn format_prompt(text: &str) -> Vec<ChatMessage> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let owned;
    let text = if text.starts_with("@@") {
        text
    } else {
        owned = format!("@@system\n{text}");
        &owned
    };

    // Equivalent to upstream's /@@@?(user|assistant|system)\n/ split: odd indices are
    // the captured role, even indices the content between markers.
    let re = regex::Regex::new(r"@@@?(user|assistant|system)\n").expect("static regex");
    let mut out = Vec::new();
    let mut last_end = 0;
    let mut pending_role: Option<ChatRole> = None;

    for capture in re.captures_iter(text) {
        let whole = capture.get(0).expect("group 0 always matches");
        if let Some(role) = pending_role.take() {
            let content = text[last_end..whole.start()].trim();
            if !content.is_empty() {
                out.push(ChatMessage::new(role, content));
            }
        }
        pending_role = Some(match &capture[1] {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => ChatRole::System,
        });
        last_end = whole.end();
    }

    if let Some(role) = pending_role {
        let content = text[last_end..].trim();
        if !content.is_empty() {
            out.push(ChatMessage::new(role, content));
        }
    }

    out
}

/// `exampleMessage` — `src/ts/process/exampleMessages.ts`.
fn example_messages(character: &Character, config: &Config) -> Vec<ChatMessage> {
    if character.example_message.is_empty() {
        return Vec::new();
    }

    let ctx = CbsContext::from_character(character, &config.username, &config.persona_prompt);
    let name_prefix = format!("{}:", character.name.to_lowercase());
    let mut out: Vec<ChatMessage> = Vec::new();
    let mut current: Option<ChatMessage> = None;

    for line in character.example_message.lines() {
        let trimmed = line.trim();
        let lowered = trimmed.to_lowercase();

        if lowered == "<start>" {
            out.extend(current.take());
            out.push(
                ChatMessage::system("[Start a new chat]").with_memo("NewChatExample"),
            );
        } else if lowered.starts_with("{{char}}:")
            || lowered.starts_with("<bot>:")
            || (!name_prefix.is_empty() && lowered.starts_with(&name_prefix))
        {
            out.extend(current.take());
            current = Some(
                ChatMessage::new(ChatRole::Assistant, after_colon(trimmed))
                    .with_name("example_assistant"),
            );
        } else if lowered.starts_with("{{user}}:") || lowered.starts_with("<user>:") {
            out.extend(current.take());
            current = Some(
                ChatMessage::new(ChatRole::User, after_colon(trimmed)).with_name("example_user"),
            );
        } else if let Some(message) = current.as_mut() {
            message.content.push('\n');
            message.content.push_str(trimmed);
        }
    }
    out.extend(current);

    for message in &mut out {
        message.content = cbs::parse(&message.content, &ctx);
    }
    out
}

/// Upstream uses `split(':', 2)[1]` — everything after the *first* colon.
fn after_colon(line: &str) -> String {
    match line.split_once(':') {
        Some((_, rest)) => rest.trim_start().to_string(),
        None => line.to_string(),
    }
}

/// `makeMs` + the per-message loop (`index.svelte.ts:869`), reduced to text handling.
fn history_messages(character: &Character, chat: &Chat, config: &Config) -> Vec<ChatMessage> {
    let ctx = CbsContext::from_character(character, &config.username, &config.persona_prompt);

    // Walk backwards so that `allBefore` truncates everything earlier, then restore
    // chronological order.
    let mut selected: Vec<&Message> = Vec::new();
    for message in chat.message.iter().rev() {
        if message.is_all_before() {
            break;
        }
        if message.is_disabled() {
            continue;
        }
        selected.push(message);
    }
    selected.reverse();

    selected
        .into_iter()
        .map(|message| {
            let role = match message.role {
                Role::User => ChatRole::User,
                Role::Char => ChatRole::Assistant,
            };
            let name = match message.role {
                Role::User => config.username.clone(),
                Role::Char => character.name.clone(),
            };
            let mut out = ChatMessage::new(role, cbs::parse(&message.data, &ctx));
            out.name = Some(name);
            // Only real history may be trimmed away; framing must survive.
            out.removable = true;
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Preset;

    fn character() -> Character {
        let mut c = Character::new("Seras");
        c.desc = "{{char}} is a knight.".to_string();
        c.personality = "brave".to_string();
        c.scenario = "a castle".to_string();
        c.first_message = "Hello, {{user}}.".to_string();
        c
    }

    fn config() -> Config {
        Config {
            username: "Alucard".to_string(),
            preset: Preset {
                main_prompt: "MAIN".to_string(),
                global_note: "NOTE".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn contents(messages: &[ChatMessage]) -> Vec<&str> {
        messages.iter().map(|m| m.content.as_str()).collect()
    }

    #[test]
    fn format_prompt_defaults_to_system() {
        let out = format_prompt("hello");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ChatRole::System);
        assert_eq!(out[0].content, "hello");
    }

    #[test]
    fn format_prompt_splits_on_role_markers() {
        let out = format_prompt("@@system\nsys\n@@user\nusr\n@@@assistant\nasst");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, ChatRole::System);
        assert_eq!(out[1].role, ChatRole::User);
        assert_eq!(out[1].content, "usr");
        assert_eq!(out[2].role, ChatRole::Assistant);
        assert_eq!(out[2].content, "asst");
    }

    #[test]
    fn format_prompt_ignores_blank_sections() {
        let out = format_prompt("@@system\n\n@@user\nhi");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ChatRole::User);
    }

    #[test]
    fn description_bundles_personality_and_scenario_with_cbs_applied() {
        let mut character = character();
        character.system_prompt = String::new();
        let result = assemble(&character, &config());
        let joined = contents(&result.messages).join("\n---\n");
        assert!(joined.contains("Seras is a knight."));
        assert!(joined.contains("Description of Seras: brave"));
        assert!(joined.contains("Circumstances and context of the dialogue: a castle"));
    }

    #[test]
    fn respects_formating_order() {
        use FormatingOrderItem::*;
        let mut config = config();
        // Isolate the two buckets under test: adjacent system messages coalesce, so
        // ordering is only observable when they are not merged into one blob.
        config.preset.formating_order = vec![Description, Main];
        let result = assemble(&character(), &config);
        let joined = contents(&result.messages).join("\n");
        let desc = joined.find("Seras is a knight.").unwrap();
        let main = joined.find("MAIN").unwrap();
        assert!(desc < main, "description should precede main: {joined}");

        config.preset.formating_order = vec![Main, Description];
        let result = assemble(&character(), &config);
        let joined = contents(&result.messages).join("\n");
        assert!(joined.find("MAIN").unwrap() < joined.find("Seras is a knight.").unwrap());
    }

    #[test]
    fn system_prompt_overrides_main_and_expands_original() {
        let mut character = character();
        character.system_prompt = "before {{original}} after".to_string();
        let result = assemble(&character, &config());
        assert!(contents(&result.messages)
            .join("\n")
            .contains("before MAIN after"));
    }

    #[test]
    fn jailbreak_is_gated_on_the_toggle() {
        let mut config = config();
        config.preset.jailbreak = "JB-TEXT".to_string();

        let without = assemble(&character(), &config);
        assert!(!contents(&without.messages).join("\n").contains("JB-TEXT"));

        config.jailbreak_toggle = true;
        let with = assemble(&character(), &config);
        assert!(contents(&with.messages).join("\n").contains("JB-TEXT"));
    }

    #[test]
    fn utility_bot_skips_main_jailbreak_and_global_note() {
        let mut character = character();
        character.utility_bot = true;
        let result = assemble(&character, &config());
        let joined = contents(&result.messages).join("\n");
        assert!(!joined.contains("MAIN"));
        assert!(!joined.contains("NOTE"));
    }

    #[test]
    fn history_maps_roles_and_drops_disabled_messages() {
        let mut character = character();
        {
            let chat = character.current_chat_mut();
            chat.message.push(Message::new(Role::Char, "greeting"));
            let mut hidden = Message::new(Role::User, "hidden");
            hidden.disabled = Some(serde_json::Value::Bool(true));
            chat.message.push(hidden);
            chat.message.push(Message::new(Role::User, "visible"));
        }

        let result = assemble(&character, &config());
        let joined = contents(&result.messages).join("\n");
        assert!(joined.contains("greeting"));
        assert!(joined.contains("visible"));
        assert!(!joined.contains("hidden"));

        let greeting = result
            .messages
            .iter()
            .find(|m| m.content == "greeting")
            .unwrap();
        assert_eq!(greeting.role, ChatRole::Assistant);
    }

    #[test]
    fn all_before_truncates_earlier_history() {
        let mut character = character();
        {
            let chat = character.current_chat_mut();
            chat.message.push(Message::new(Role::Char, "ancient"));
            let mut cut = Message::new(Role::User, "cut-point");
            cut.disabled = Some(serde_json::Value::String("allBefore".to_string()));
            chat.message.push(cut);
            chat.message.push(Message::new(Role::Char, "recent"));
        }

        let result = assemble(&character, &config());
        let joined = contents(&result.messages).join("\n");
        assert!(!joined.contains("ancient"));
        assert!(!joined.contains("cut-point"));
        assert!(joined.contains("recent"));
    }

    #[test]
    fn parses_example_messages() {
        let mut character = character();
        character.example_message =
            "<START>\n{{user}}: hi there\n{{char}}: hey\nsecond line".to_string();
        let messages = example_messages(&character, &config());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].memo.as_deref(), Some("NewChatExample"));
        assert_eq!(messages[1].role, ChatRole::User);
        assert_eq!(messages[1].content, "hi there");
        assert_eq!(messages[1].name.as_deref(), Some("example_user"));
        assert_eq!(messages[2].role, ChatRole::Assistant);
        assert_eq!(messages[2].content, "hey\nsecond line");
    }

    #[test]
    fn example_messages_accept_the_character_name_as_a_speaker() {
        let mut character = character();
        character.example_message = "Seras: by name".to_string();
        let messages = example_messages(&character, &config());
        assert_eq!(messages[0].role, ChatRole::Assistant);
        assert_eq!(messages[0].content, "by name");
    }

    #[test]
    fn adjacent_system_messages_are_coalesced() {
        let mut out = Vec::new();
        push_prompts(
            &mut out,
            &[
                ChatMessage::system("one"),
                ChatMessage::system("two"),
                ChatMessage::new(ChatRole::User, "u"),
                ChatMessage::system("three"),
            ],
        );
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].content, "one\n\ntwo");
        assert_eq!(out[2].content, "three");
    }

    #[test]
    fn differing_memos_prevent_coalescing() {
        let mut out = Vec::new();
        push_prompts(
            &mut out,
            &[
                ChatMessage::system("one").with_memo("a"),
                ChatMessage::system("two").with_memo("b"),
            ],
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn trims_oldest_history_when_over_budget() {
        let mut character = character();
        {
            let chat = character.current_chat_mut();
            for i in 0..40 {
                chat.message
                    .push(Message::new(Role::User, format!("message number {i} ").repeat(20)));
            }
        }

        let mut config = config();
        config.preset.max_context = 1200;
        config.preset.max_response = 100;

        let result = assemble(&character, &config);
        assert!(result.trimmed > 0, "expected trimming to occur");

        let joined = contents(&result.messages).join("\n");
        assert!(!joined.contains("message number 0 "));
        // The newest turn always survives; it is what the model is replying to.
        assert!(joined.contains("message number 39 "));
        // Framing is preserved even under pressure.
        assert!(joined.contains("[Start a new chat]"));
    }

    /// The newest turn is what the model is being asked to reply to. Trimming it away
    /// produces a prompt that silently omits what the user just typed.
    #[test]
    fn the_newest_turn_survives_an_impossible_budget() {
        let mut character = character();
        character.desc = "long description. ".repeat(400);
        character.current_chat_mut().message.push(Message::new(
            Role::User,
            "MY_TYPED_INPUT",
        ));

        let mut config = config();
        // maxResponse alone exceeds maxContext: no trimming can ever satisfy this.
        config.preset.max_context = 4000;
        config.preset.max_response = 6000;

        let result = assemble(&character, &config);
        let joined = contents(&result.messages).join("\n");
        assert!(
            joined.contains("MY_TYPED_INPUT"),
            "the just-typed message must survive: {joined}"
        );
        assert!(
            result.over_budget > 0,
            "an unsatisfiable budget must be reported, not hidden"
        );
    }

    #[test]
    fn a_satisfiable_budget_reports_no_overrun() {
        let mut character = character();
        for i in 0..20 {
            character
                .current_chat_mut()
                .message
                .push(Message::new(Role::User, format!("message {i} ").repeat(20)));
        }
        let mut config = config();
        config.preset.max_context = 4000;
        config.preset.max_response = 300;

        let result = assemble(&character, &config);
        assert_eq!(result.over_budget, 0);
        assert!(!result.drops_history);
    }

    /// `lastChat` receives the newest message. If the order never emits that bucket the
    /// message has to stay in `chats` instead of vanishing.
    #[test]
    fn the_newest_turn_survives_an_order_without_last_chat() {
        use FormatingOrderItem::*;
        let mut character = character();
        character
            .current_chat_mut()
            .message
            .push(Message::new(Role::User, "MY_TYPED_INPUT"));

        let mut config = config();
        config.preset.formating_order = vec![Main, Description, Chats, GlobalNote];

        let result = assemble(&character, &config);
        assert!(
            contents(&result.messages).join("\n").contains("MY_TYPED_INPUT"),
            "{:?}",
            contents(&result.messages)
        );
    }

    #[test]
    fn an_order_without_chats_is_reported() {
        use FormatingOrderItem::*;
        let mut config = config();
        config.preset.formating_order = vec![Main, Description, LastChat];
        let result = assemble(&character(), &config);
        assert!(result.drops_history);
    }

    #[test]
    fn author_note_falls_back_to_the_configured_default() {
        let mut config = config();
        config.author_note_default_text = "DEFAULT-NOTE".to_string();
        let result = assemble(&character(), &config);
        assert!(contents(&result.messages)
            .join("\n")
            .contains("DEFAULT-NOTE"));

        let mut character = character();
        character.current_chat_mut().note = "CHAT-NOTE".to_string();
        let result = assemble(&character, &config);
        let joined = contents(&result.messages).join("\n");
        assert!(joined.contains("CHAT-NOTE"));
        assert!(!joined.contains("DEFAULT-NOTE"));
    }

    #[test]
    fn character_global_note_replaces_the_preset_one() {
        let mut character = character();
        character.replace_global_note = "[{{original}}]".to_string();
        let result = assemble(&character, &config());
        assert!(contents(&result.messages).join("\n").contains("[NOTE]"));
    }
}
