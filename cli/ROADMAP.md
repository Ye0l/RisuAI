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
| 0-3 | Character card loader — JSON (CCv2/v3) | `characterCards.ts:720` | done |
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
  at least does something. Replaced by tiktoken in M1-4. The per-turn usage line prints
  the provider's real counts alongside it, so the size of the error stays visible.

### Context budget

`maxContext` bounds the whole prompt; `maxResponse` is reserved out of it before any
history is added. If `maxResponse` plus the static blocks (main, description, persona,
global note…) already fills `maxContext`, there is no room left for the conversation.

Trimming drops the oldest history first but **never the newest turn** — that is the
message being replied to, and removing it sends a prompt that silently omits what the
user just typed. When the budget still cannot be met the overrun is reported rather than
absorbed:

```
  · trimmed 1 old message(s) to fit maxContext
  ⚠ prompt is ~2867 tokens over maxContext (4000) even after trimming. maxResponse (6000)
    plus the static prompt blocks leave no room for the conversation — raise maxContext
    or lower maxResponse.
```

A `formatingOrder` with no `chats` entry is likewise reported instead of quietly sending
no history.

### Reply truncation

Replies are capped by `preset.maxResponse`, which defaults to 500 to match upstream
(`database.svelte.ts:59` — note `presetTemplate.maxResponse` is 300, but the DB-level
field is what requests actually use). That is small for modern models, so every turn
reports the provider's `finish_reason` and token usage:

```
  · 412 prompt + 500/500 reply tokens
  ⚠ cut off: hit max_tokens (500). Raise `preset.maxResponse` in config.json, or pass `--max-response N`.
```

A clean stop, a cap hit, a provider-side limit below the cap, a content filter, and a
gateway that reports no `finish_reason` at all are each distinguished — a truncated
reply is never silent. Streaming (M2-5) will need the same treatment on the
`finish_reason` field of the final chunk.

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

| # | Item | Notes | Status |
|---|---|---|---|
| 3-1 | PNG `tEXt` chunk cards (`chara` / `ccv3`) | **import done** (pulled forward — JSON-only import was not useful in practice, since cards are distributed as PNG/CHARX). Export still pending. | partial |
| 3-2 | CHARX (zip), incl. charx-embedded JPEG | **import done** (pulled forward). Export still pending. | partial |
| 3-3 | RisuSave `.bin` reader | msgpack + 4 magic headers + gzip/fflate — `risuSave.ts:622`. Unlocks opening existing web saves, and `module.risum` inside CHARX. | |

Import handles the container formats; two things inside them are recognised but not
yet stored, and `card import` says so explicitly when it sees them:

- **Embedded assets** (`assets/` in CHARX, `chara-ext-asset_*` PNG chunks) — needs an
  asset store, which nothing in the CLI uses yet.
- **`module.risum`** inside a CHARX — msgpack, carries lorebook/regex/trigger
  overrides. Blocked on M3-3's reader.

Also unsupported: `rcc||`-prefixed PNG cards (RisuAI's compressed variant), and
`zTXt`/`iTXt` chunks. Both are detected and reported rather than mis-parsed.

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
    card/
      mod.rs          container detection; CCv2/v3 + off-spec JSON → Character
      png.rs          tEXt chunk extraction
      charx.rs        zip extraction (also charx-embedded JPEG)
    cbs.rs            M0 stub substituter (M1: real parser)
    debug.rs          wire-level request/response logging, with redaction
    token.rs          M0 approximate counter (M1: tiktoken)
    prompt.rs         formatingOrder assembly
    provider/
      mod.rs          provider trait + ChatMessage
      openai.rs       OpenAI-compatible chat completions
      reformat.rs     message-shape normalization for strict endpoints
    store.rs          JSON data directory
    repl.rs           interactive loop
```

`cli/` is a standalone crate, not a workspace member of `src-tauri/`, so it builds and
tests without pulling in the Tauri toolchain. If the Tauri app ever wants to share this
code, the pipeline modules should be split out into a `risu-core` library crate first.

## Usage

```sh
cargo run -- card info  path/to/card.png     # inspect without importing
cargo run -- card import path/to/card.charx  # .json / .png / .charx / .jpg
cargo run -- ls                              # list characters
cargo run -- prompt -c <name>                # dry-run: print the assembled prompt
cargo run -- chat -c <name>                  # interactive chat
```

Data lives in `$RISU_CLI_HOME`, else `$XDG_DATA_HOME/risu-cli`, else `~/.risu-cli`.

### Endpoint compatibility

OpenAI accepts system messages anywhere and repeated roles; most other APIs do not.
RisuAI's `formatingOrder` violates their rules by construction — it interleaves system
blocks with chat history, and `globalNote` lands after `lastChat`, so the last message
is usually a system one. GLM/z.ai rejects that outright with
`messages parameter is illegal`.

`api.compat` picks the message shape, and `reformat.rs` ports upstream's `reformater()`
(`request.ts:345`) to apply it:

| value | behaviour |
|---|---|
| `auto` (default) | infer from `baseURL` and `model` |
| `openai` | no rewriting |
| `strict` | one leading system message, alternating roles, must start with user |

`strict` hoists leading system messages into one, demotes the rest to user turns wrapped
as `system: {{slot}}` (upstream's `systemContentReplacement`), merges adjacent same-role
turns, and prepends a user turn if needed. No content is lost — the trailing global note
survives, merged into the final user message.

Blank turns are dropped before the alternation pass (so a removal cannot leave two
same-role turns adjacent), and the synthetic leading user turn uses `.` rather than
upstream's literal `' '` (`request.ts:422`). z.ai counts a whitespace-only turn as no
prompt at all and answers "The prompt parameter was not received normally", which is
what a fresh chat hit: the character greets first, so that placeholder is the opening
turn.

`auto` resolves to `strict` for z.ai, open.bigmodel.cn, DeepSeek and Mistral hosts, and
for `glm*`/`deepseek*`/`mistral*` model ids including vendor-prefixed ones
(`z-ai/glm-4.6`). Aggregators that normalize server-side (OpenRouter, Together) stay on
`openai` so the prompt is not rewritten needlessly. The resolved value is shown by
`config` and in the chat header.

Per-model `LLMFlags` (M2) will replace this coarse profile.

### Inspecting what goes over the wire

```sh
cargo run -- prompt -c <name> --wire     # exact POST body, without sending
cargo run -- --debug chat -c <name>      # full request + response per turn
RISU_DEBUG=1 cargo run -- chat -c <name> # same, via env
```

In the REPL, `/wire` prints the next request body and `/debug` toggles logging
mid-session.

`--debug` logs the request **after** reqwest builds it — real headers, real serialized
body — so what is printed cannot drift from what is sent. `--wire` and the real request
share one `build_body`, for the same reason. Responses are logged raw, before parsing,
with status, elapsed time and headers; a non-JSON body (an HTML error page from a
proxy) is passed through as text rather than swallowed.

API keys are redacted everywhere, including keys embedded in `baseURL` by proxies —
registration happens at config-load time so no code path can print one. The redaction
keeps a short fingerprint (`sk-…89[redacted 35 chars]`) so two different wrong keys
stay distinguishable.
