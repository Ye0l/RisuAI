//! Character cards embedded in PNG `tEXt` chunks (M3-1).
//!
//! Upstream reads these at `src/ts/characterCards.ts:215-320` via `PngChunk`. The card
//! JSON is base64 in a chunk keyed `ccv3` (v3) or `chara` (v2); when both are present
//! `ccv3` wins (`characterCards.ts:310`). Assets live in `chara-ext-asset_*` chunks,
//! which are counted but not extracted — there is no asset store yet.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&SIGNATURE)
}

#[derive(Debug)]
pub struct PngCard {
    /// Decoded card JSON.
    pub json: Vec<u8>,
    /// Number of `chara-ext-asset_*` chunks seen.
    pub asset_chunks: usize,
}

/// Pull the embedded card out of a PNG.
pub fn extract(bytes: &[u8]) -> Result<PngCard> {
    if !is_png(bytes) {
        bail!("not a PNG");
    }

    let mut chara: Option<String> = None;
    let mut ccv3: Option<String> = None;
    let mut asset_chunks = 0;

    for chunk in Chunks::new(&bytes[SIGNATURE.len()..]) {
        let chunk = chunk?;
        // Scan to IEND, never stopping at IDAT: upstream's writer appends the card
        // chunk *after* the image data, just before IEND (`pngChunk.ts:11-46`), and its
        // reader scans the whole file to match (`pngChunk.ts:194`).
        if chunk.kind == *b"IEND" {
            break;
        }
        // Only tEXt is handled; RisuAI and SillyTavern both write uncompressed tEXt.
        // zTXt/iTXt cards exist in the wild but are rare enough to defer.
        if chunk.kind != *b"tEXt" {
            continue;
        }
        let Some((keyword, value)) = split_text_chunk(chunk.data) else {
            continue;
        };
        match keyword.as_str() {
            "chara" => chara = Some(value),
            "ccv3" => ccv3 = Some(value),
            key if key.starts_with("chara-ext-asset_") => asset_chunks += 1,
            _ => {}
        }
    }

    let encoded = match ccv3.or(chara) {
        Some(encoded) => encoded,
        None => bail!("PNG has no `ccv3` or `chara` metadata chunk — not a character card"),
    };

    // RisuAI's own encrypted/compressed variant. Detecting it explicitly beats failing
    // with a base64 error.
    if encoded.starts_with("rcc||") {
        bail!("this is an `rcc` (RisuAI compressed) PNG card, which is not supported yet");
    }

    let json = BASE64
        .decode(encoded.trim())
        .context("card chunk is not valid base64")?;

    Ok(PngCard { json, asset_chunks })
}

/// `keyword\0text`, with the keyword restricted to Latin-1 by the PNG spec.
fn split_text_chunk(data: &[u8]) -> Option<(String, String)> {
    let split = data.iter().position(|b| *b == 0)?;
    let keyword = String::from_utf8_lossy(&data[..split]).into_owned();
    let value = String::from_utf8_lossy(&data[split + 1..]).into_owned();
    Some((keyword, value))
}

struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
}

/// Walks `length | type | data | crc` records. CRCs are not verified: a card with a bad
/// CRC is still a card, and rejecting it would help nobody.
struct Chunks<'a> {
    rest: &'a [u8],
    done: bool,
}

impl<'a> Chunks<'a> {
    fn new(rest: &'a [u8]) -> Self {
        Chunks { rest, done: false }
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.rest.is_empty() {
            return None;
        }
        if self.rest.len() < 8 {
            self.done = true;
            return Some(Err(anyhow::anyhow!("truncated PNG chunk header")));
        }

        let length = u32::from_be_bytes([self.rest[0], self.rest[1], self.rest[2], self.rest[3]])
            as usize;
        let kind = [self.rest[4], self.rest[5], self.rest[6], self.rest[7]];

        // 8 header + length + 4 CRC
        let end = match 12usize.checked_add(length) {
            Some(end) if end <= self.rest.len() => end,
            _ => {
                self.done = true;
                return Some(Err(anyhow::anyhow!(
                    "PNG chunk claims {length} bytes but only {} remain",
                    self.rest.len().saturating_sub(12)
                )));
            }
        };

        let data = &self.rest[8..8 + length];
        self.rest = &self.rest[end..];
        Some(Ok(Chunk { kind, data }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC, unverified
        out
    }

    fn text_chunk(keyword: &str, value: &str) -> Vec<u8> {
        let mut data = keyword.as_bytes().to_vec();
        data.push(0);
        data.extend_from_slice(value.as_bytes());
        chunk(b"tEXt", &data)
    }

    fn png(chunks: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        for c in chunks {
            out.extend_from_slice(&c);
        }
        out
    }

    #[test]
    fn extracts_a_chara_chunk() {
        let payload = BASE64.encode(r#"{"spec":"chara_card_v2"}"#);
        let bytes = png(vec![text_chunk("chara", &payload)]);
        let card = extract(&bytes).unwrap();
        assert_eq!(card.json, br#"{"spec":"chara_card_v2"}"#);
    }

    #[test]
    fn ccv3_wins_over_chara() {
        let bytes = png(vec![
            text_chunk("chara", &BASE64.encode("v2")),
            text_chunk("ccv3", &BASE64.encode("v3")),
        ]);
        assert_eq!(extract(&bytes).unwrap().json, b"v3");

        // Order in the file must not matter.
        let bytes = png(vec![
            text_chunk("ccv3", &BASE64.encode("v3")),
            text_chunk("chara", &BASE64.encode("v2")),
        ]);
        assert_eq!(extract(&bytes).unwrap().json, b"v3");
    }

    #[test]
    fn counts_asset_chunks() {
        let bytes = png(vec![
            text_chunk("chara", &BASE64.encode("{}")),
            text_chunk("chara-ext-asset_:0", "AAAA"),
            text_chunk("chara-ext-asset_:1", "AAAA"),
        ]);
        assert_eq!(extract(&bytes).unwrap().asset_chunks, 2);
    }

    #[test]
    fn finds_metadata_written_after_the_image_data() {
        // This is where upstream actually puts it: IHDR, IDAT, then the card tEXt,
        // then IEND. Reading only the chunks before IDAT would miss every real card.
        let bytes = png(vec![
            chunk(b"IHDR", &[0; 13]),
            chunk(b"IDAT", b"pixels"),
            text_chunk("chara", &BASE64.encode(r#"{"spec":"chara_card_v2"}"#)),
            chunk(b"IEND", b""),
        ]);
        assert_eq!(extract(&bytes).unwrap().json, br#"{"spec":"chara_card_v2"}"#);
    }

    #[test]
    fn stops_at_iend() {
        // Anything after IEND is trailing garbage, not metadata.
        let bytes = png(vec![
            chunk(b"IEND", b""),
            text_chunk("chara", &BASE64.encode("{}")),
        ]);
        assert!(extract(&bytes).is_err());
    }

    #[test]
    fn reports_a_plain_png_clearly() {
        let bytes = png(vec![chunk(b"IHDR", &[0; 13]), chunk(b"IEND", b"")]);
        let error = extract(&bytes).unwrap_err().to_string();
        assert!(error.contains("not a character card"), "{error}");
    }

    #[test]
    fn reports_rcc_cards_clearly() {
        let bytes = png(vec![text_chunk("chara", "rcc||something")]);
        let error = extract(&bytes).unwrap_err().to_string();
        assert!(error.contains("rcc"), "{error}");
    }

    #[test]
    fn rejects_non_png() {
        assert!(extract(b"not a png at all").is_err());
    }

    #[test]
    fn rejects_a_truncated_chunk() {
        let mut bytes = png(vec![text_chunk("chara", &BASE64.encode("{}"))]);
        bytes.truncate(bytes.len() - 6);
        assert!(extract(&bytes).is_err());
    }
}
