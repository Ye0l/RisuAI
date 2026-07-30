//! risu-cli — a headless reimplementation of RisuAI's chat engine.
//!
//! See `ROADMAP.md` for what is and is not implemented. M0 is the minimal vertical
//! slice: JSON card in, assembled prompt out, OpenAI-compatible request, JSON on disk.

mod cbs;
mod card;
mod cli;
mod config;
mod debug;
mod model;
mod prompt;
mod provider;
mod repl;
mod store;
mod token;

use anyhow::{Context, Result};
use clap::Parser;

use crate::card::CardSource;
use crate::cli::{CardCommand, Cli, Command};
use crate::prompt::AssembleResult;
use crate::provider::ChatRole;
use crate::store::Store;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` gives the whole context chain on one line. A backtrace is noise
            // for a bad input file, which is most of what can go wrong here.
            eprintln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    debug::set_enabled(cli.debug || debug_from_env());
    let store = Store::open(cli.data_dir.clone())?;

    match cli.command {
        Command::Ls => cmd_ls(&store),
        Command::Card { command } => cmd_card(&store, command),
        Command::Prompt {
            character,
            message,
            json,
            wire,
        } => cmd_prompt(&store, &character, message, json, wire),
        Command::Chat {
            character,
            model,
            max_response,
        } => cmd_chat(&store, &character, model, max_response).await,
        Command::Config => cmd_config(&store),
    }
}

fn cmd_ls(store: &Store) -> Result<()> {
    let characters = store.list_characters()?;
    if characters.is_empty() {
        println!("no characters yet — `risu-cli card import <file.json>`");
        return Ok(());
    }
    for character in characters {
        let chat = character.current_chat();
        println!(
            "{:<24} {:>4} msg  {}",
            character.name,
            chat.message.len(),
            character.cha_id
        );
    }
    Ok(())
}

fn cmd_card(store: &Store, command: CardCommand) -> Result<()> {
    match command {
        CardCommand::Info { path } => {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let loaded = card::load(&bytes)?;
            let character = loaded.character;
            println!("format         {}", describe_source(&loaded.source));
            println!("name           {}", character.name);
            if !character.creator.is_empty() {
                println!("creator        {}", character.creator);
            }
            if !character.character_version.is_empty() {
                println!("version        {}", character.character_version);
            }
            if !character.tags.is_empty() {
                println!("tags           {}", character.tags.join(", "));
            }
            println!("description    {} chars", character.desc.len());
            println!("personality    {} chars", character.personality.len());
            println!("scenario       {} chars", character.scenario.len());
            println!("first message  {} chars", character.first_message.len());
            println!("examples       {} chars", character.example_message.len());
            println!("greetings      {}", character.alternate_greetings.len());
            println!("lorebook       {} entries", character.global_lore.len());
            println!("scripts        {}", character.customscript.len());
            Ok(())
        }
        CardCommand::Import { path } => {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let loaded = card::load(&bytes)?;
            let saved = store.save_character(&loaded.character)?;
            println!(
                "imported {} ({}) -> {}",
                loaded.character.name,
                describe_source(&loaded.source),
                saved.display()
            );
            warn_about_dropped_data(&loaded.source);
            Ok(())
        }
    }
}

fn cmd_prompt(
    store: &Store,
    query: &str,
    pending: Option<String>,
    as_json: bool,
    as_wire: bool,
) -> Result<()> {
    let config = store.load_config()?;
    let mut character = store.find_character(query)?;
    // In memory only — previewing a prompt must not start the chat on disk.
    prompt::seed_first_message(&mut character, &config);

    if let Some(text) = pending {
        character
            .current_chat_mut()
            .message
            .push(model::Message::new(model::Role::User, text));
    }

    let result = prompt::assemble(&character, &config);
    if as_wire {
        // Same builder the provider uses, so this cannot drift from what is sent.
        let request = repl::chat_request(&result.messages, &config, &config.api.model);
        let body = provider::openai::build_body(&request);
        println!(
            "POST {}",
            debug::redact(&format!("{}/chat/completions", config.api.base_url))
        );
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else if as_json {
        println!("{}", serde_json::to_string_pretty(&result.messages)?);
    } else {
        println!("{}", render_prompt(&result));
    }
    Ok(())
}

async fn cmd_chat(
    store: &Store,
    query: &str,
    model: Option<String>,
    max_response: Option<usize>,
) -> Result<()> {
    let mut config = store.load_config()?;
    if let Some(max_response) = max_response {
        config.preset.max_response = max_response;
    }
    let character = store.find_character(query)?;
    repl::run(store, character, &config, model).await
}

fn cmd_config(store: &Store) -> Result<()> {
    // Side effect: writes the defaults if this is a first run.
    let config = store.load_config()?;
    println!("data dir  {}", store.root().display());
    println!("config    {}", store.root().join("config.json").display());
    println!("model     {}", config.api.model);
    println!("endpoint  {}", debug::redact(&config.api.base_url));
    println!(
        "compat    {:?} (resolved: {:?})",
        config.api.compat,
        config.api.resolved_compat()
    );
    println!(
        "api key   {}",
        if config.api.api_key.is_empty() {
            "(unset)"
        } else {
            "(set)"
        }
    );
    Ok(())
}

/// `RISU_DEBUG`, parsed the way people actually write it. Clap's own `env` support for
/// a bool flag insists on the literal `true`/`false` and errors on `RISU_DEBUG=1`.
fn debug_from_env() -> bool {
    match std::env::var("RISU_DEBUG") {
        Ok(value) => !matches!(
            value.trim().to_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        ),
        Err(_) => false,
    }
}

fn describe_source(source: &CardSource) -> String {
    match source {
        CardSource::Json => "JSON".to_string(),
        CardSource::Png { asset_chunks: 0 } => "PNG".to_string(),
        CardSource::Png { asset_chunks } => format!("PNG, {asset_chunks} asset chunk(s)"),
        CardSource::Charx { assets, has_module } => {
            let module = if *has_module { ", module.risum" } else { "" };
            format!("CHARX, {assets} asset(s){module}")
        }
    }
}

/// Assets and embedded modules are recognised but not yet stored. Say so at import
/// time rather than letting the character quietly come up missing pieces later.
fn warn_about_dropped_data(source: &CardSource) {
    let (assets, has_module) = match source {
        CardSource::Json => (0, false),
        CardSource::Png { asset_chunks } => (*asset_chunks, false),
        CardSource::Charx { assets, has_module } => (*assets, *has_module),
    };
    if assets > 0 {
        eprintln!("note: {assets} embedded asset(s) skipped — no asset store yet");
    }
    if has_module {
        eprintln!(
            "note: module.risum skipped — its lorebook/regex/trigger overrides need \
             the msgpack reader (M3-3)"
        );
    }
}

/// Human-readable rendering of an assembled prompt, for `prompt` and `/prompt`.
pub fn render_prompt(result: &AssembleResult) -> String {
    let mut out = String::new();
    for message in &result.messages {
        let role = match message.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };
        out.push_str(&format!("--- {role} ---\n{}\n\n", message.content));
    }
    out.push_str(&format!(
        "({} messages, ~{} tokens, {} trimmed)",
        result.messages.len(),
        token::approx_messages(&result.messages),
        result.trimmed
    ));
    if result.over_budget > 0 {
        out.push_str(&format!(
            "\n⚠ ~{} tokens over maxContext even after trimming",
            result.over_budget
        ));
    }
    if result.drops_history {
        out.push_str("\n⚠ formatingOrder has no `chats` entry: no history is sent");
    }
    out
}
