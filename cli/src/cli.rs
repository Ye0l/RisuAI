use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "risu-cli",
    about = "Headless CLI reimplementation of RisuAI's chat engine",
    version
)]
pub struct Cli {
    /// Data directory (default: $RISU_CLI_HOME, else the platform data dir).
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List imported characters.
    Ls,

    /// Work with character cards.
    Card {
        #[command(subcommand)]
        command: CardCommand,
    },

    /// Print the assembled prompt without sending it anywhere.
    Prompt {
        /// Character name, name prefix, or chaId.
        #[arg(short, long)]
        character: String,

        /// Append a pending user message before assembling.
        #[arg(short, long)]
        message: Option<String>,

        /// Emit the raw JSON message array instead of the readable rendering.
        #[arg(long)]
        json: bool,
    },

    /// Start an interactive chat.
    Chat {
        /// Character name, name prefix, or chaId.
        #[arg(short, long)]
        character: String,

        /// Override the configured model for this session.
        #[arg(long)]
        model: Option<String>,
    },

    /// Print the resolved data directory and config path.
    Config,
}

#[derive(Subcommand)]
pub enum CardCommand {
    /// Show what a card contains without importing it.
    Info {
        /// Path to a .json character card.
        path: PathBuf,
    },

    /// Import a card into the data directory.
    Import {
        /// Path to a .json character card.
        path: PathBuf,
    },
}
