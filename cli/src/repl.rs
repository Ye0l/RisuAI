//! Interactive chat loop (M0-7).
//!
//! The CLI counterpart of `sendChat` (`src/ts/process/index.svelte.ts:67`), reduced to:
//! append the user turn, assemble, request, append the reply, persist.

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::config::{Compat, Config};
use crate::debug;
use crate::model::{Character, Message, Role};
use crate::prompt;
use crate::provider::openai::OpenAiProvider;
use crate::provider::reformat::MessageShape;
use crate::provider::{ChatMessage, ChatRequest, Completion, FinishReason, Provider};
use crate::store::Store;

const HELP: &str = "\
  /help          show this
  /prompt        print the prompt that would be sent next
  /wire          print the exact request body that would be POSTed
  /debug         toggle full HTTP request/response logging
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

    prompt::seed_first_message(&mut character, config);
    store.save_character(&character)?;

    let compat = config.api.resolved_compat();
    println!("{} — {} ({:?} message shape)", character.name, model, compat);
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
                "wire" => {
                    let result = prompt::assemble(&character, config);
                    let request = chat_request(&result.messages, config, &model);
                    let body = crate::provider::openai::build_body(&request);
                    println!(
                        "POST {}",
                        debug::redact(&format!("{}/chat/completions", config.api.base_url))
                    );
                    println!("{}", serde_json::to_string_pretty(&body)?);
                }
                "debug" => {
                    let enabled = !debug::is_enabled();
                    debug::set_enabled(enabled);
                    println!("debug logging {}", if enabled { "on" } else { "off" });
                }
                "new" => {
                    let index = character.chats.len();
                    character
                        .chats
                        .push(crate::model::Chat::new(format!("Chat {}", index + 1)));
                    character.chat_page = index;
                    prompt::seed_first_message(&mut character, config);
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

/// Build the provider request from a preset. Shared with `prompt --wire` so that what
/// `--wire` prints is necessarily what a real turn would send.
pub fn chat_request<'a>(
    messages: &'a [ChatMessage],
    config: &'a Config,
    model: &'a str,
) -> ChatRequest<'a> {
    let preset = &config.preset;
    ChatRequest {
        messages,
        model,
        shape: match config.api.resolved_compat() {
            Compat::Strict => MessageShape::strict(),
            // `auto` is already resolved; anything else means no rewriting.
            _ => MessageShape::openai(),
        },
        // Upstream stores these on a 0-200 percent scale.
        temperature: preset.temperature / 100.0,
        top_p: preset.top_p,
        frequency_penalty: preset.frequency_penalty / 100.0,
        presence_penalty: preset.presence_penalty / 100.0,
        max_tokens: preset.max_response,
    }
}

async fn generate(
    provider: &OpenAiProvider,
    character: &mut Character,
    config: &Config,
    model: &str,
    store: &Store,
) -> Result<()> {
    let assembled = prompt::assemble(character, config);
    warn_about_assembly(&assembled, config);

    let preset = &config.preset;
    let request = chat_request(&assembled.messages, config, model);

    match provider.send(request).await {
        Ok(completion) => {
            let message = Message::new(Role::Char, &completion.content);
            print_turn(character, config, &message);
            report_finish(&completion, preset.max_response);
            character.current_chat_mut().message.push(message);
            store.save_character(character)?;
        }
        Err(error) => {
            // Leave the user's turn in place so /retry works after fixing the cause.
            eprintln!("error: {error:#}");
        }
    }
    Ok(())
}

/// Report anything about the assembled prompt that would otherwise silently change what
/// the model sees.
fn warn_about_assembly(assembled: &prompt::AssembleResult, config: &Config) {
    if assembled.trimmed > 0 {
        eprintln!(
            "  · trimmed {} old message(s) to fit maxContext",
            assembled.trimmed
        );
    }
    if assembled.over_budget > 0 {
        let preset = &config.preset;
        eprintln!(
            "  ⚠ prompt is ~{} tokens over maxContext ({}) even after trimming. \
             maxResponse ({}) plus the static prompt blocks leave no room for the \
             conversation — raise maxContext or lower maxResponse.",
            assembled.over_budget, preset.max_context, preset.max_response
        );
    }
    if assembled.drops_history {
        eprintln!(
            "  ⚠ formatingOrder has no `chats` entry, so no conversation history is \
             being sent."
        );
    }
}

/// Say why the model stopped. Without this a reply cut off at `max_tokens` is
/// indistinguishable from a short reply — which is the whole problem being solved here.
fn report_finish(completion: &Completion, max_response: usize) {
    // Real counts from the provider. Worth showing every turn: they are the ground
    // truth against which `token::approx` (the M0 stub used for context trimming) can
    // be sanity-checked, and they make budget pressure visible before it truncates.
    match (
        completion.usage.prompt_tokens,
        completion.usage.completion_tokens,
    ) {
        (Some(prompt), Some(reply)) => {
            eprintln!("  · {prompt} prompt + {reply}/{max_response} reply tokens")
        }
        (Some(prompt), None) => eprintln!("  · {prompt} prompt tokens"),
        (None, Some(reply)) => eprintln!("  · {reply}/{max_response} reply tokens"),
        (None, None) => {}
    }

    match &completion.finish_reason {
        FinishReason::Length => {
            // Only blame our cap when the reply actually reached it. A `length` stop
            // well under budget means something upstream imposed its own limit, and
            // telling the user to raise `maxResponse` would send them the wrong way.
            let reached_cap = completion
                .usage
                .completion_tokens
                .is_none_or(|used| used + used / 10 >= max_response as u64);
            if reached_cap {
                eprintln!(
                    "  ⚠ cut off: hit max_tokens ({max_response}). Raise \
                     `preset.maxResponse` in config.json, or pass `--max-response N`."
                );
            } else {
                let used = completion.usage.completion_tokens.unwrap_or(0);
                eprintln!(
                    "  ⚠ cut off after {used} tokens, well under the {max_response} \
                     requested — the provider or gateway applied its own limit."
                );
            }
        }
        FinishReason::ContentFilter => {
            eprintln!("  ⚠ the provider's content filter stopped this response early.");
        }
        FinishReason::Other(reason) => {
            eprintln!("  ⚠ stopped for an unrecognized reason: {reason:?}");
        }
        FinishReason::Unknown => {
            // Some gateways omit it entirely. Flag it once so a silent truncation is
            // never mistaken for a clean stop.
            eprintln!("  · the provider reported no finish_reason, so truncation cannot be ruled out");
        }
        FinishReason::Stop => {
            if let Some(completion_tokens) = completion.usage.completion_tokens {
                // A "stop" that lands exactly on the cap is suspicious: some gateways
                // mislabel a truncation.
                if completion_tokens >= max_response as u64 {
                    eprintln!(
                        "  ⚠ reported a clean stop but used the whole {max_response}-token \
                         budget — likely truncated anyway."
                    );
                }
            }
        }
    }
}

fn print_turn(character: &Character, config: &Config, message: &Message) {
    let speaker = match message.role {
        Role::User => config.username.as_str(),
        Role::Char => character.name.as_str(),
    };
    println!("\n{speaker}: {}\n", message.data);
}
