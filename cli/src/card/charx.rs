//! CHARX archives (M3-2).
//!
//! A `.charx` is a zip holding `card.json` at the root plus an `assets/` tree; upstream
//! streams it with `CharXImporter` (`src/ts/process/processzip.ts:160`, card handling at
//! :368). A `.jpg` "charx-embedded" card is the same zip appended to a JPEG — the zip
//! reader locates the central directory from the end of the file, so a leading image is
//! transparent.
//!
//! Assets are counted, not extracted: there is no asset store yet. `module.risum`
//! (msgpack, carries lorebook/regex/trigger overrides) is detected and reported, since
//! silently dropping it would lose lorebook entries the card depends on.

use std::io::{Cursor, Read};

use anyhow::{bail, Context, Result};

pub fn looks_like_zip(bytes: &[u8]) -> bool {
    // Local file header. A charx-embedded JPEG starts with FFD8 instead, so callers
    // fall back to attempting the zip reader when this is false.
    bytes.starts_with(b"PK\x03\x04")
}

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

#[derive(Debug)]
pub struct CharxCard {
    /// Raw `card.json` bytes.
    pub json: Vec<u8>,
    /// Number of entries under `assets/`.
    pub assets: usize,
    /// Whether `module.risum` was present.
    pub has_module: bool,
}

/// 64 MiB. `card.json` is text; anything this large is malformed or hostile.
const MAX_CARD_JSON: u64 = 64 * 1024 * 1024;

pub fn extract(bytes: &[u8]) -> Result<CharxCard> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .context("could not read the CHARX archive (not a zip?)")?;

    let mut assets = 0;
    let mut has_module = false;
    let mut card_index: Option<usize> = None;

    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .with_context(|| format!("could not read zip entry {index}"))?;
        // `enclosed_name` rejects absolute paths and `..` traversal. We only read
        // `card.json` here, but the name is still used for classification.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name = name.to_string_lossy().replace('\\', "/");

        if name == "card.json" {
            card_index = Some(index);
        } else if name == "module.risum" {
            has_module = true;
        } else if name.starts_with("assets/") && !entry.is_dir() {
            assets += 1;
        }
    }

    let Some(card_index) = card_index else {
        bail!("CHARX archive has no `card.json` at its root");
    };

    let entry = archive.by_index(card_index)?;
    if entry.size() > MAX_CARD_JSON {
        bail!("card.json is implausibly large ({} bytes)", entry.size());
    }
    let mut json = Vec::with_capacity(entry.size() as usize);
    entry
        .take(MAX_CARD_JSON)
        .read_to_end(&mut json)
        .context("could not decompress card.json")?;

    Ok(CharxCard {
        json,
        assets,
        has_module,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use zip::write::SimpleFileOptions;

    fn build(entries: &[(&str, &[u8])], prefix: &[u8]) -> Vec<u8> {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(prefix);
        {
            let mut cursor = Cursor::new(&mut buffer);
            // Append after the prefix rather than overwriting it.
            cursor.seek(SeekFrom::End(0)).unwrap();
            let mut writer = zip::ZipWriter::new(cursor);
            for (name, data) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(data).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer
    }

    #[test]
    fn extracts_card_json() {
        let bytes = build(
            &[
                ("card.json", br#"{"spec":"chara_card_v3"}"#),
                ("assets/one.png", b"x"),
                ("assets/two.png", b"y"),
            ],
            b"",
        );
        let card = extract(&bytes).unwrap();
        assert_eq!(card.json, br#"{"spec":"chara_card_v3"}"#);
        assert_eq!(card.assets, 2);
        assert!(!card.has_module);
    }

    #[test]
    fn detects_an_embedded_module() {
        let bytes = build(&[("card.json", b"{}"), ("module.risum", b"\x00")], b"");
        assert!(extract(&bytes).unwrap().has_module);
    }

    #[test]
    fn reads_a_zip_appended_to_a_jpeg() {
        // charxJpeg: JPEG bytes first, zip second.
        let bytes = build(&[("card.json", b"{\"ok\":true}")], &[0xFF, 0xD8, 0xFF, 0xE0]);
        assert!(is_jpeg(&bytes));
        assert!(!looks_like_zip(&bytes));
        assert_eq!(extract(&bytes).unwrap().json, b"{\"ok\":true}");
    }

    #[test]
    fn reports_a_missing_card_json() {
        let bytes = build(&[("assets/one.png", b"x")], b"");
        let error = extract(&bytes).unwrap_err().to_string();
        assert!(error.contains("card.json"), "{error}");
    }

    #[test]
    fn rejects_non_zip_input() {
        assert!(extract(b"definitely not a zip").is_err());
    }
}
