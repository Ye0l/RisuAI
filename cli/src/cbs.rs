//! CBS (curly brace syntax) — **M0 stub**.
//!
//! Upstream registers 176 functions (`src/ts/cbs.ts`) on top of a block-structured
//! mini-language with variables, conditionals and recursion
//! (`src/ts/parser/parser.svelte.ts`). None of that is here.
//!
//! This module does literal replacement of the handful of placeholders that appear in
//! essentially every character card. It exists because descriptions are saturated with
//! `{{char}}`, so an M0 without any substitution would emit visibly broken prompts.
//! M1-1 replaces this wholesale with a real parser; treat the surface as unstable.

use crate::model::Character;

/// Values available to the stub substituter.
pub struct CbsContext<'a> {
    pub char_name: &'a str,
    pub user_name: &'a str,
    pub personality: &'a str,
    pub description: &'a str,
    pub scenario: &'a str,
    pub persona: &'a str,
}

impl<'a> CbsContext<'a> {
    pub fn from_character(character: &'a Character, user_name: &'a str, persona: &'a str) -> Self {
        CbsContext {
            char_name: &character.name,
            user_name,
            personality: &character.personality,
            description: &character.desc,
            scenario: &character.scenario,
            persona,
        }
    }
}

/// Replace the supported placeholders. Unknown `{{...}}` is left untouched rather than
/// blanked, so that unimplemented syntax stays visible instead of silently vanishing.
pub fn parse(text: &str, ctx: &CbsContext) -> String {
    if !text.contains("{{") {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < text.len() {
        if bytes[i] == b'{' && i + 1 < text.len() && bytes[i + 1] == b'{' {
            if let Some(end) = text[i + 2..].find("}}") {
                let inner = &text[i + 2..i + 2 + end];
                match substitute(inner, ctx) {
                    Some(value) => out.push_str(&value),
                    // Not a placeholder we know: emit verbatim.
                    None => out.push_str(&text[i..i + 2 + end + 2]),
                }
                i += 2 + end + 2;
                continue;
            }
        }
        // Push one whole UTF-8 character, never a partial byte.
        let ch = text[i..].chars().next().expect("index is on a char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn substitute(name: &str, ctx: &CbsContext) -> Option<String> {
    // Upstream lowercases and strips whitespace before dispatch.
    let key = name.trim().to_lowercase();
    let value = match key.as_str() {
        "char" | "bot" => ctx.char_name,
        "user" => ctx.user_name,
        "personality" | "char_personality" => ctx.personality,
        "description" | "char_desc" => ctx.description,
        "scenario" => ctx.scenario,
        "persona" | "user_persona" => ctx.persona,
        "br" | "newline" => return Some("\n".to_string()),
        "blank" | "none" => return Some(String::new()),
        _ => return None,
    };
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CbsContext<'static> {
        CbsContext {
            char_name: "Seras",
            user_name: "Alucard",
            personality: "brave",
            description: "a knight",
            scenario: "a castle",
            persona: "a wanderer",
        }
    }

    #[test]
    fn substitutes_known_placeholders() {
        assert_eq!(parse("{{char}} met {{user}}", &ctx()), "Seras met Alucard");
    }

    #[test]
    fn is_case_and_space_insensitive() {
        assert_eq!(parse("{{ CHAR }}", &ctx()), "Seras");
    }

    #[test]
    fn leaves_unknown_syntax_verbatim() {
        // M1-1 will actually evaluate these; until then they must not disappear.
        assert_eq!(parse("{{#if x}}y{{/if}}", &ctx()), "{{#if x}}y{{/if}}");
        assert_eq!(parse("{{getvar::hp}}", &ctx()), "{{getvar::hp}}");
    }

    #[test]
    fn handles_unterminated_braces() {
        assert_eq!(parse("{{char", &ctx()), "{{char");
    }

    #[test]
    fn preserves_multibyte_text() {
        assert_eq!(parse("안녕 {{char}} 님", &ctx()), "안녕 Seras 님");
    }

    #[test]
    fn br_expands_to_newline() {
        assert_eq!(parse("a{{br}}b", &ctx()), "a\nb");
    }
}
