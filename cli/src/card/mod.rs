//! Character card loading.
//!
//! Ports the field mapping of `importCharacterCardSpec` (`src/ts/characterCards.ts:720`)
//! for `chara_card_v2` / `chara_card_v3`, plus the off-spec fallback at
//! `characterCards.ts:68`.
//!
//! Every container format ends up in the same place: extract the embedded card JSON,
//! then run it through [`from_value`]. `.risum` / `.risup` (RisuAI's own msgpack
//! formats) still need M3-3's reader.

pub mod charx;
pub mod png;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::model::{Character, LoreBook};

/// Which container a card arrived in, for reporting back to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardSource {
    Json,
    /// PNG `tEXt` chunks, with the number of embedded asset chunks.
    Png { asset_chunks: usize },
    /// CHARX zip — or a zip appended to a JPEG.
    Charx { assets: usize, has_module: bool },
}

pub struct LoadedCard {
    pub character: Character,
    pub source: CardSource,
}

/// Detect the container by content, not by file extension: `.jpg` cards are zips,
/// `.charx` files are zips, and a mislabelled `.png` should still load.
pub fn load(bytes: &[u8]) -> Result<LoadedCard> {
    if png::is_png(bytes) {
        let extracted = png::extract(bytes)?;
        return Ok(LoadedCard {
            character: from_json_slice(&extracted.json)?,
            source: CardSource::Png {
                asset_chunks: extracted.asset_chunks,
            },
        });
    }

    if charx::looks_like_zip(bytes) || charx::is_jpeg(bytes) {
        let extracted = charx::extract(bytes)?;
        return Ok(LoadedCard {
            character: from_json_slice(&extracted.json)?,
            source: CardSource::Charx {
                assets: extracted.assets,
                has_module: extracted.has_module,
            },
        });
    }

    Ok(LoadedCard {
        character: from_json_slice(bytes)?,
        source: CardSource::Json,
    })
}

/// Parse a character card from raw JSON bytes.
pub fn from_json_slice(bytes: &[u8]) -> Result<Character> {
    let value: Value =
        serde_json::from_slice(bytes).context("card is not valid JSON")?;
    from_value(&value)
}

pub fn from_value(value: &Value) -> Result<Character> {
    match value.get("spec").and_then(Value::as_str) {
        Some("chara_card_v2") | Some("chara_card_v3") => from_spec_card(value),
        // A character exported by risu-cli itself, or lifted out of a RisuAI save.
        _ if value.get("chaId").is_some() => serde_json::from_value(value.clone())
            .context("looks like a risu character but failed to deserialize"),
        _ => from_off_spec_card(value),
    }
}

/// `chara_card_v2` / `chara_card_v3`.
fn from_spec_card(card: &Value) -> Result<Character> {
    let data = card
        .get("data")
        .context("spec card has no `data` object")?;

    let mut character = Character::new(str_field(data, "name"));
    character.first_message = str_field(data, "first_mes");
    character.desc = str_field(data, "description");
    character.personality = str_field(data, "personality");
    character.scenario = str_field(data, "scenario");
    character.example_message = str_field(data, "mes_example");
    character.creator_notes = str_field(data, "creator_notes");
    character.system_prompt = str_field(data, "system_prompt");
    character.creator = str_field(data, "creator");
    character.character_version = str_field(data, "character_version");
    // Upstream maps `post_history_instructions` onto `replaceGlobalNote`, not onto
    // `postHistoryInstructions` (characterCards.ts:991).
    character.replace_global_note = str_field(data, "post_history_instructions");
    character.alternate_greetings = str_vec_field(data, "alternate_greetings");
    character.tags = str_vec_field(data, "tags");

    if let Some(charbook) = data.get("character_book") {
        character.global_lore = convert_charbook(charbook);
    }

    // `extensions.risuai` carries RisuAI-specific fields on cards it exported itself.
    if let Some(risuext) = data.pointer("/extensions/risuai") {
        if let Some(scripts) = risuext.get("customScripts") {
            character.customscript =
                serde_json::from_value(scripts.clone()).unwrap_or_default();
        }
        if let Some(b) = risuext.get("utilityBot").and_then(Value::as_bool) {
            character.utility_bot = b;
        }
    }

    if character.name.is_empty() {
        bail!("card has no name");
    }
    Ok(character)
}

/// Pre-spec "TavernAI" cards: `char_name` / `char_persona` / `char_greeting`.
/// Mirrors the guard at `characterCards.ts:68`.
fn from_off_spec_card(card: &Value) -> Result<Character> {
    let name = first_str(card, &["char_name", "name"]);
    let desc = first_str(card, &["char_persona", "description"]);
    let greeting = first_str(card, &["char_greeting", "first_mes"]);

    if name.is_empty() || desc.is_empty() || greeting.is_empty() {
        bail!("unrecognized card format: not a v2/v3 spec card and missing off-spec fields");
    }

    let mut character = Character::new(name);
    character.desc = desc;
    character.first_message = greeting;
    character.example_message = first_str(card, &["example_dialogue", "mes_example"]);
    character.scenario = first_str(card, &["world_scenario", "scenario"]);
    character.personality = str_field(card, "personality");
    Ok(character)
}

/// `convertCharbook` — `characterCards.ts:1036`, reduced to the fields M1-2 will read.
/// The `@@`-directive migrations for SillyTavern extension fields are not ported yet;
/// they belong with the activation engine that consumes them.
fn convert_charbook(charbook: &Value) -> Vec<LoreBook> {
    let Some(entries) = charbook.get("entries").and_then(Value::as_array) else {
        return Vec::new();
    };

    entries
        .iter()
        .map(|entry| {
            let keys = str_vec_field(entry, "keys");
            let secondary = str_vec_field(entry, "secondary_keys");
            let constant = entry
                .get("constant")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let selective = entry
                .get("selective")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // `use_regex` only counts when the key is actually written as a regex
            // literal (characterCards.ts:1059).
            let use_regex = entry
                .get("use_regex")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && keys.first().is_some_and(|k| k.starts_with('/'));

            LoreBook {
                key: keys.join(", "),
                secondkey: secondary.join(", "),
                insertorder: entry
                    .get("insertion_order")
                    .and_then(Value::as_i64)
                    .unwrap_or(100),
                comment: first_str(entry, &["comment", "name"]),
                content: str_field(entry, "content"),
                mode: if constant { "constant" } else { "normal" }.to_string(),
                always_active: constant,
                selective: selective && !secondary.is_empty(),
                use_regex,
                id: entry.get("id").map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                }),
            }
        })
        .collect()
}

fn str_field(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => s.clone(),
        // `character_version` is sometimes a number; upstream template-stringifies it.
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn first_str(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        let found = str_field(value, key);
        if !found.is_empty() {
            return found;
        }
    }
    String::new()
}

fn str_vec_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loads_a_v3_spec_card() {
        let card = json!({
            "spec": "chara_card_v3",
            "spec_version": "3.0",
            "data": {
                "name": "Seras",
                "description": "a knight",
                "personality": "brave",
                "scenario": "a castle",
                "first_mes": "Hello.",
                "mes_example": "<START>\n{{user}}: hi\n{{char}}: hey",
                "post_history_instructions": "stay in character",
                "alternate_greetings": ["Hi again."],
                "tags": ["fantasy"],
                "character_version": 2
            }
        });

        let character = from_value(&card).unwrap();
        assert_eq!(character.name, "Seras");
        assert_eq!(character.desc, "a knight");
        assert_eq!(character.first_message, "Hello.");
        // post_history_instructions lands on replaceGlobalNote, per upstream.
        assert_eq!(character.replace_global_note, "stay in character");
        assert_eq!(character.alternate_greetings, vec!["Hi again."]);
        assert_eq!(character.character_version, "2");
        assert_eq!(character.chats.len(), 1);
    }

    #[test]
    fn loads_an_off_spec_card() {
        let card = json!({
            "char_name": "Old",
            "char_persona": "legacy",
            "char_greeting": "yo",
            "world_scenario": "somewhere"
        });

        let character = from_value(&card).unwrap();
        assert_eq!(character.name, "Old");
        assert_eq!(character.desc, "legacy");
        assert_eq!(character.scenario, "somewhere");
    }

    #[test]
    fn converts_a_character_book() {
        let card = json!({
            "spec": "chara_card_v2",
            "data": {
                "name": "Seras",
                "character_book": {
                    "entries": [{
                        "keys": ["castle", "keep"],
                        "secondary_keys": ["night"],
                        "content": "The castle is cold.",
                        "insertion_order": 5,
                        "constant": true,
                        "selective": true,
                        "comment": "setting"
                    }]
                }
            }
        });

        let character = from_value(&card).unwrap();
        let entry = &character.global_lore[0];
        assert_eq!(entry.key, "castle, keep");
        assert_eq!(entry.secondkey, "night");
        assert_eq!(entry.insertorder, 5);
        assert_eq!(entry.mode, "constant");
        assert!(entry.always_active);
        assert!(entry.selective);
    }

    #[test]
    fn use_regex_requires_a_regex_literal_key() {
        let make = |key: &str| {
            json!({
                "spec": "chara_card_v3",
                "data": {
                    "name": "X",
                    "character_book": { "entries": [{
                        "keys": [key], "content": "c", "use_regex": true
                    }]}
                }
            })
        };
        assert!(!from_value(&make("plain")).unwrap().global_lore[0].use_regex);
        assert!(from_value(&make("/re/i")).unwrap().global_lore[0].use_regex);
    }

    #[test]
    fn rejects_unrecognized_json() {
        assert!(from_value(&json!({ "hello": "world" })).is_err());
    }
}
