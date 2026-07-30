//! Interactive chat loop (M0-7).
//!
//! The CLI counterpart of `sendChat` (`src/ts/process/index.svelte.ts:67`), reduced to:
//! append the user turn, assemble, request, append the reply, persist.

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::cbs::{self, CbsContext};
use crate::config::Config;
use crate::model::{Character, Message, Role};
use crate::prompt;
use crate::provider::openai::OpenAiProvider;
use crate::provider::{ChatRequest, Provider};
use crate::store::Store;

const HELP: &str = "\
  /help          show this
  /prompt        print the prompt that would be sent next
  /retry         regenerate the last reply
  /undo          drop the last exchange
  /new           start a fresh chat with this character
  /quit          exit (progress is saved after every turn)";

pub async fn run(
    store: &Store,
    mut character: Character,
    config: &Config,
    model_override: Option<String>,
) -> Result<()> {
    let model = model_override.unwrap_or_else(|| config.api.model.clone());
    let provider = OpenAiProvider::new(&config.api);

    seed_first_message(&mut character, config);
    store.save_character(&character)?;

    println!("{} — {}", character.name, model);
    println!("{}", "-".repeat(40));
    for message in &character.current_chat().message {
        print_turn(&character, config, message);
    }
    println!("(/help for commands)");

    let mut editor = DefaultEditor::new()?;

    loop {
        let line = match editor.readline("> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(error) => return Err(error.into()),
        };
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        editor.add_history_entry(input)?;

        if let Some(command) = input.strip_prefix('/') {
            match command {
                "quit" | "exit" | "q" => break,
                "help" | "?" => println!("{HELP}"),
                "prompt" => {
                    let result = prompt::assemble(&character, config);
                    println!("{}", crate::render_prompt(&result));
                }
                "new" => {
                    let index = character.chats.len();
                    character
                        .chats
                        .push(crate::model::Chat::new(format!("Chat {}", index + 1)));
                    character.chat_page = index;
                    seed_first_message(&mut character, config);
                    store.save_character(&character)?;
                    println!("started {}", character.current_chat().name);
                    for message in &character.current_chat().message {
                        print_turn(&character, config, message);
                    }
                }
                "undo" => {
                    let chat = character.current_chat_mut();
                    // Drop the reply, then the user turn that prompted it.
                    if chat.message.last().is_some_and(|m| m.role == Role::Char) {
                        chat.message.pop();
                    }
                    if chat.message.last().is_some_and(|m| m.role == Role::User) {
                        chat.message.pop();
                    }
                    store.save_character(&character)?;
                    println!("(undone)");
                }
                "retry" => {
                    let had_reply = character
                        .current_chat()
                        .message
                        .last()
                        .is_some_and(|m| m.role == Role::Char);
                    if !had_reply {
                        println!("(nothing to retry)");
                        continue;
                    }
                    character.current_chat_mut().message.pop();
                    generate(&provider, &mut character, config, &model, store).await?;
                }
                other => println!("unknown command: /{other} (/help)"),
            }
            continue;
        }

        character
            .current_chat_mut()
            .message
            .push(Message::new(Role::User, input));
        generate(&provider, &mut character, config, &model, store).await?;
    }

    store.save_character(&character)?;
    Ok(())
}

async fn generate(
    provider: &OpenAiProvider,
    character: &mut Character,
    config: &Config,
    model: &str,
    store: &Store,
) -> Result<()> {
    let assembled = prompt::assemble(character, config);
    if assembled.trimmed > 0 {
        eprintln!(
            "(trimmed {} old message(s) to fit maxContext)",
            assembled.trimmed
        );
    }

    let preset = &config.preset;
    let request = ChatRequest {
        messages: &assembled.messages,
        model,
        // Upstream stores these on a 0-200 percent scale.
        temperature: preset.temperature / 100.0,
        top_p: preset.top_p,
        frequency_penalty: preset.frequency_penalty / 100.0,
        presence_penalty: preset.presence_penalty / 100.0,
        max_tokens: preset.max_response,
    };

    match provider.send(request).await {
        Ok(reply) => {
            let message = Message::new(Role::Char, reply);
            print_turn(character, config, &message);
            character.current_chat_mut().message.push(message);
            store.save_character(character)?;
        }
        Err(error) => {
            // Leave the user's turn in place so /retry works after fixing the cause.
            eprintln!("request failed: {error:#}");
        }
    }
    Ok(())
}

/// A new chat opens with the character's greeting, as the UI does.
fn seed_first_message(character: &mut Character, config: &Config) {
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

fn print_turn(character: &Character, config: &Config, message: &Message) {
    let speaker = match message.role {
        Role::User => config.username.as_str(),
        Role::Char => character.name.as_str(),
    };
    println!("\n{speaker}: {}\n", message.data);
}
