//! Plain-JSON data directory (M0-6).
//!
//! Replaces upstream's msgpack `.bin` save format (`src/ts/storage/risuSave.ts`) with
//! one readable file per character. Reading real `.bin` saves is M3-3; the model
//! structs already use upstream's field names so that reader can deserialize into them.
//!
//! ```text
//! $RISU_CLI_HOME/
//!   config.json
//!   characters/<chaId>.json
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::model::Character;

pub struct Store {
    root: PathBuf,
}

impl Store {
    /// `$RISU_CLI_HOME`, else `$XDG_DATA_HOME/risu-cli`, else `~/.risu-cli`.
    pub fn open(explicit: Option<PathBuf>) -> Result<Self> {
        let root = match explicit {
            Some(path) => path,
            None => match std::env::var_os("RISU_CLI_HOME") {
                Some(path) => PathBuf::from(path),
                None => match dirs::data_dir() {
                    Some(dir) => dir.join("risu-cli"),
                    None => dirs::home_dir()
                        .context("no home directory; set RISU_CLI_HOME")?
                        .join(".risu-cli"),
                },
            },
        };

        fs::create_dir_all(root.join("characters"))
            .with_context(|| format!("could not create data directory {}", root.display()))?;
        Ok(Store { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    fn characters_dir(&self) -> PathBuf {
        self.root.join("characters")
    }

    /// Loads `config.json`, writing out the defaults on first run so the file is there
    /// to edit.
    pub fn load_config(&self) -> Result<Config> {
        let path = self.config_path();
        if !path.exists() {
            let config = Config::default();
            self.save_config(&config)?;
            return Ok(config);
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        let path = self.config_path();
        let text = serde_json::to_string_pretty(config)?;
        write_atomic(&path, text.as_bytes())
    }

    pub fn list_characters(&self) -> Result<Vec<Character>> {
        let dir = self.characters_dir();
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("could not read {}", dir.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = fs::read_to_string(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            match serde_json::from_str::<Character>(&text) {
                Ok(character) => out.push(character),
                Err(error) => {
                    eprintln!("warning: skipping {}: {error}", path.display());
                }
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    }

    /// Resolve by exact `chaId`, then by exact name, then by unique case-insensitive
    /// prefix.
    pub fn find_character(&self, query: &str) -> Result<Character> {
        let characters = self.list_characters()?;
        if characters.is_empty() {
            bail!("no characters imported yet — try `risu-cli card import <file>`");
        }

        if let Some(found) = characters.iter().find(|c| c.cha_id == query) {
            return Ok(found.clone());
        }
        if let Some(found) = characters.iter().find(|c| c.name == query) {
            return Ok(found.clone());
        }

        let needle = query.to_lowercase();
        let matches: Vec<&Character> = characters
            .iter()
            .filter(|c| c.name.to_lowercase().starts_with(&needle))
            .collect();

        match matches.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!("no character matching {query:?}"),
            many => {
                let names: Vec<&str> = many.iter().map(|c| c.name.as_str()).collect();
                bail!("{query:?} is ambiguous: {}", names.join(", "))
            }
        }
    }

    pub fn save_character(&self, character: &Character) -> Result<PathBuf> {
        let path = self.characters_dir().join(format!("{}.json", character.cha_id));
        let text = serde_json::to_string_pretty(character)?;
        write_atomic(&path, text.as_bytes())?;
        Ok(path)
    }
}

/// Write via a temp file and rename, so an interrupted save cannot truncate a chat log.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes)
        .with_context(|| format!("could not write {}", temp.display()))?;
    fs::rename(&temp, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}
