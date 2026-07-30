//! Message-shape normalization for providers stricter than OpenAI.
//!
//! Port of `reformater()` (`src/ts/process/request/request.ts:345`). OpenAI accepts
//! system messages anywhere and consecutive same-role turns; most other APIs do not.
//! GLM/z.ai rejects the whole request with "messages parameter is illegal" if a system
//! message appears mid-conversation, if two same-role turns are adjacent, or if the
//! conversation does not start with a user turn.
//!
//! RisuAI's assembled prompt violates all three by design: `formatingOrder` interleaves
//! system blocks with chat history, and `globalNote` lands *after* `lastChat`, so the
//! final message is usually a system one.

use crate::provider::{ChatMessage, ChatRole};

/// What a provider will accept. Mirrors the relevant `LLMFlags`
/// (`src/ts/model/types.ts:3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageShape {
    /// `hasFullSystemPrompt`: system messages allowed anywhere. OpenAI.
    pub full_system_prompt: bool,
    /// `hasFirstSystemPrompt`: a single leading system message is allowed; later ones
    /// must be rewritten.
    pub first_system_prompt: bool,
    /// `requiresAlternateRole`: user and assistant must strictly alternate.
    pub alternate_roles: bool,
    /// `mustStartWithUserInput`: the first non-system message must be from the user.
    pub start_with_user: bool,
    /// How a demoted system message's content is wrapped. `{{slot}}` is the content.
    pub system_content_replacement: String,
    /// What role a demoted system message takes.
    pub system_role_replacement: ChatRole,
}

impl MessageShape {
    /// OpenAI: no restrictions, nothing to rewrite.
    pub fn openai() -> Self {
        MessageShape {
            full_system_prompt: true,
            first_system_prompt: false,
            alternate_roles: false,
            start_with_user: false,
            system_content_replacement: default_system_content_replacement(),
            system_role_replacement: ChatRole::User,
        }
    }

    /// GLM/z.ai, DeepSeek, Mistral, Cohere and most Anthropic-shaped gateways.
    pub fn strict() -> Self {
        MessageShape {
            full_system_prompt: false,
            first_system_prompt: true,
            alternate_roles: true,
            start_with_user: true,
            system_content_replacement: default_system_content_replacement(),
            system_role_replacement: ChatRole::User,
        }
    }

}

/// `database.svelte.ts:569`.
pub fn default_system_content_replacement() -> String {
    "system: {{slot}}".to_string()
}

/// Rewrite an assembled prompt into something the provider will accept.
pub fn reformat(messages: Vec<ChatMessage>, shape: &MessageShape) -> Vec<ChatMessage> {
    let mut messages = messages;
    let mut hoisted_system: Option<ChatMessage> = None;

    if !shape.full_system_prompt {
        if shape.first_system_prompt {
            // Pull the leading run of system messages off into one, to be restored at
            // the front after everything else is rewritten.
            let mut split = 0;
            while split < messages.len() && messages[split].role == ChatRole::System {
                split += 1;
            }
            for message in messages.drain(..split) {
                match &mut hoisted_system {
                    Some(system) => {
                        system.content.push_str("\n\n");
                        system.content.push_str(&message.content);
                    }
                    None => hoisted_system = Some(message),
                }
            }
        }

        // Any system message left is mid-conversation: demote it to a real turn.
        for message in &mut messages {
            if message.role == ChatRole::System {
                message.content = shape
                    .system_content_replacement
                    .replace("{{slot}}", &message.content);
                message.role = shape.system_role_replacement;
            }
        }
    }

    if shape.alternate_roles {
        let mut merged: Vec<ChatMessage> = Vec::with_capacity(messages.len());
        for message in messages {
            match merged.last_mut() {
                Some(last) if last.role == message.role => {
                    last.content.push('\n');
                    last.content.push_str(&message.content);
                }
                _ => merged.push(message),
            }
        }
        messages = merged;
    }

    if shape.start_with_user && messages.first().map(|m| m.role) != Some(ChatRole::User) {
        // Upstream inserts a single space rather than an empty string: providers that
        // demand a leading user turn generally also reject empty content.
        messages.insert(0, ChatMessage::new(ChatRole::User, " "));
    }

    if let Some(system) = hoisted_system {
        messages.insert(0, system);
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: ChatRole, content: &str) -> ChatMessage {
        ChatMessage::new(role, content)
    }

    /// The exact shape RisuAI's formatingOrder produces, which z.ai rejects.
    fn risu_shaped_prompt() -> Vec<ChatMessage> {
        vec![
            msg(ChatRole::System, "main+description+persona"),
            msg(ChatRole::System, "[Start a new chat]"),
            msg(ChatRole::User, "Who are you?"),
            msg(ChatRole::Assistant, "Seras."),
            msg(ChatRole::System, "[Start a new chat]"),
            msg(ChatRole::Assistant, "The door groans open."),
            msg(ChatRole::User, "Any word from the south?"),
            msg(ChatRole::System, "Never speak for User."),
        ]
    }

    fn roles(messages: &[ChatMessage]) -> Vec<ChatRole> {
        messages.iter().map(|m| m.role).collect()
    }

    #[test]
    fn openai_shape_changes_nothing() {
        let input = risu_shaped_prompt();
        let output = reformat(input.clone(), &MessageShape::openai());
        assert_eq!(output, input);
    }

    #[test]
    fn strict_shape_produces_a_valid_glm_conversation() {
        let output = reformat(risu_shaped_prompt(), &MessageShape::strict());

        use ChatRole::*;
        assert_eq!(roles(&output), vec![System, User, Assistant, User, Assistant, User]);

        // Exactly one system message, and it is first.
        assert_eq!(output.iter().filter(|m| m.role == System).count(), 1);
        // The conversation ends on a user turn.
        assert_eq!(output.last().unwrap().role, User);
        // No two adjacent turns share a role.
        assert!(output.windows(2).all(|w| w[0].role != w[1].role));
    }

    #[test]
    fn leading_system_messages_are_merged_not_dropped() {
        let output = reformat(risu_shaped_prompt(), &MessageShape::strict());
        let system = &output[0];
        assert!(system.content.contains("main+description+persona"));
        assert!(system.content.contains("[Start a new chat]"));
    }

    #[test]
    fn the_trailing_system_note_survives_as_a_user_turn() {
        // This is the message that made the whole request illegal; it must still reach
        // the model, not be discarded.
        let output = reformat(risu_shaped_prompt(), &MessageShape::strict());
        let last = output.last().unwrap();
        assert_eq!(last.role, ChatRole::User);
        assert!(last.content.contains("Any word from the south?"));
        assert!(last.content.contains("system: Never speak for User."));
    }

    #[test]
    fn a_conversation_starting_with_assistant_gets_a_user_turn_prepended() {
        let input = vec![
            msg(ChatRole::System, "sys"),
            msg(ChatRole::Assistant, "greeting"),
            msg(ChatRole::User, "hi"),
        ];
        let output = reformat(input, &MessageShape::strict());
        use ChatRole::*;
        assert_eq!(roles(&output), vec![System, User, Assistant, User]);
        assert_eq!(output[1].content, " ");
    }

    #[test]
    fn no_system_messages_at_all_is_handled() {
        let input = vec![msg(ChatRole::User, "hi")];
        let output = reformat(input, &MessageShape::strict());
        assert_eq!(roles(&output), vec![ChatRole::User]);
    }

    #[test]
    fn an_all_system_prompt_collapses_to_one_message() {
        let input = vec![msg(ChatRole::System, "a"), msg(ChatRole::System, "b")];
        let output = reformat(input, &MessageShape::strict());
        // Hoisted into one system, then a user turn is required to follow it.
        use ChatRole::*;
        assert_eq!(roles(&output), vec![System, User]);
        assert_eq!(output[0].content, "a\n\nb");
    }

    #[test]
    fn an_empty_prompt_does_not_panic() {
        assert!(reformat(Vec::new(), &MessageShape::strict()).len() <= 1);
    }

    #[test]
    fn custom_replacement_is_applied() {
        let mut shape = MessageShape::strict();
        shape.system_content_replacement = "<note>{{slot}}</note>".to_string();
        let input = vec![
            msg(ChatRole::User, "hi"),
            msg(ChatRole::System, "be brief"),
        ];
        let output = reformat(input, &shape);
        assert!(output[0].content.contains("<note>be brief</note>"), "{output:?}");
    }

    #[test]
    fn demoting_to_assistant_is_supported() {
        let mut shape = MessageShape::strict();
        shape.system_role_replacement = ChatRole::Assistant;
        let input = vec![
            msg(ChatRole::User, "hi"),
            msg(ChatRole::System, "note"),
            msg(ChatRole::User, "again"),
        ];
        let output = reformat(input, &shape);
        use ChatRole::*;
        assert_eq!(roles(&output), vec![User, Assistant, User]);
    }
}
