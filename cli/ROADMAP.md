# risu-cli — Rust CLI reimplementation roadmap

A minimal, headless reimplementation of RisuAI's core chat engine as a Rust CLI.

## Why

RisuAI is ~71k lines of TypeScript, but the part that actually makes it an *AI chatbot*
is a small pipeline:

```
character card → prompt assembly → LLM request → append to chat log → persist
```

Everything else — the Svelte UI, the iframe plugin sandbox, TTS, Stable Diffusion,
3D/VN modes, the translator, multiuser sync, the Realm hub — is peripheral to that
pipeline. This crate reimplements the pipeline and nothing else.

## The four load-bearing pieces

Ordered by how much of RisuAI's actual behaviour they account for:

| Piece | Upstream source |
|---|---|
| Prompt assembly (`formatingOrder` / `promptTemplate`) | `src/ts/process/index.svelte.ts:1190` |
| CBS parser (`{{char}}`, `{{#if}}`, 176 registered functions) | `src/ts/cbs.ts`, `src/ts/parser/parser.svelte.ts` |
| Lorebook activation engine | `src/ts/process/lorebook.svelte.ts:75` |
| Provider abstraction | `src/ts/process/request/` |

---

## M0 — minimal vertical slice

The smallest thing that is a working chatbot. Deliberately drops streaming,
multimodal, group chat, memory, modules, triggers and lorebooks.

| # | Item | Upstream reference | Status |
|---|---|---|---|
| 0-1 | Cargo crate + clap CLI skeleton | — | done |
| 0-2 | Data model structs (subset) | `database.svelte.ts:1342/1815/1846/1319/1307` | done |
| 0-3 | Character card loader — JSON (CCv2/v3) only | `characterCards.ts:720` | done |
| 0-4 | Prompt assembly — legacy `formatingOrder` path only | `index.svelte.ts:1190,1432` | done |
| 0-5 | One provider — OpenAI-compatible, non-streaming | `request/openAI/requests.ts` | done |
| 0-6 | Chat persistence — plain JSON, not msgpack `.bin` | replaces `risuSave.ts` | done |
| 0-7 | REPL loop | `index.svelte.ts:67` (`sendChat`) | done |

**M0 carries two deliberate stubs**, both replaced in M1:

- `cbs::stub` — literal `{{char}}`/`{{user}}`/`{{personality}}`/… replacement only.
  Not the real CBS parser; there are no blocks, no variables, no functions. Character
  descriptions are full of `{{char}}`, so shipping M0 without *some* substitution would
  produce visibly broken prompts.
- `token::approx` — `chars / 4` heuristic used for `maxContext` trimming, so the budget
  at least does something. Replaced by tiktoken in M1-4.

## M1 — the load-bearing core

| # | Item | Notes |
|---|---|---|
| 1-1 | **CBS parser** | 176 registered functions upstream. Scope hard-capped at the ~30 that matter: `char user description personality scenario persona lorebook getvar setvar tempvar calc random roll slot br blank lastmessage chatindex` + comparators (`equal notequal greater less and or not`) + time family. Block syntax (`{{#if}}…{{/if}}`) needs a separate parser pass. |
| 1-2 | **Lorebook activation engine** | key/secondkey scan, `scanDepth`, `insertorder`, `tokenBudget`, `constant`/`normal`/`selective`/`multiple` modes, `useRegex`, recursive scanning, `@@depth` / `@@role` / `@@position` directives. |
| 1-3 | **Regex scripts** | `customscript` with the four hooks: `editinput` / `editoutput` / `editprocess` / `editdisplay`. |
| 1-4 | **Tokenizer + context trimming** | `tiktoken-rs` (already a dependency of `src-tauri`). Replaces `token::approx`. |

## M2 — presets and providers

| # | Item |
|---|---|
| 2-1 | `promptTemplate` (`PromptItem`) path — the modern preset system: `plain/chat/persona/description/lorebook/authornote/memory/postEverything/cache` |
| 2-2 | SillyTavern preset import (`STCHAT` / `STCONTEXT` / `PARAMETERS`) — see `prompt.ts:detectPromptJSONType` |
| 2-3 | Anthropic Messages format |
| 2-4 | Google Gemini format |
| 2-5 | SSE streaming output |

## M3 — data interop

| # | Item | Notes |
|---|---|---|
| 3-1 | PNG `tEXt` chunk cards (`chara` / `ccv3`) | import + export |
| 3-2 | CHARX (zip) | import + export |
| 3-3 | RisuSave `.bin` reader | msgpack + 4 magic headers + gzip/fflate — `risuSave.ts:622`. Unlocks opening existing web saves from the CLI. |

## M4 — later

Memory systems (SupaMemory / HypaV2 / HypaV3), modules, trigger scripts, group chat,
personas, logit bias, alternate greetings.

## Explicitly out of scope

Svelte UI · plugin system (iframe sandbox) · TTS · Stable Diffusion · 3D / visual novel
modes · translator · multiuser sync · Realm hub · MCP.

---

## Layout

```
cli/
  src/
    main.rs           entry point, command dispatch
    cli.rs            clap definitions
    model.rs          Character / Chat / Message / LoreBook / CustomScript
    preset.rs         Preset + defaults (mirrors presetTemplate)
    card.rs           CCv2/v3 JSON → Character
    cbs.rs            M0 stub substituter (M1: real parser)
    token.rs          M0 approximate counter (M1: tiktoken)
    prompt.rs         formatingOrder assembly
    provider/
      mod.rs          provider trait + ChatMessage
      openai.rs       OpenAI-compatible chat completions
    store.rs          JSON data directory
    repl.rs           interactive loop
```

`cli/` is a standalone crate, not a workspace member of `src-tauri/`, so it builds and
tests without pulling in the Tauri toolchain. If the Tauri app ever wants to share this
code, the pipeline modules should be split out into a `risu-core` library crate first.

## Usage

```sh
cargo run -- card import path/to/card.json   # import a character card
cargo run -- ls                              # list characters
cargo run -- prompt -c <name>                # dry-run: print the assembled prompt
cargo run -- chat -c <name>                  # interactive chat
```

Data lives in `$RISU_CLI_HOME`, else `$XDG_DATA_HOME/risu-cli`, else `~/.risu-cli`.
