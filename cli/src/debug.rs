//! Wire-level debug logging.
//!
//! Prints the request exactly as built and the response exactly as received, so that
//! "is this an error or is this really what the model said" can be answered by looking
//! rather than guessing.
//!
//! The enabled flag is process-global, the way a log level is: it has to be togglable
//! from the REPL mid-session (`/debug`) without threading a parameter through every
//! call site.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Secrets to blank out before anything is printed. Debug output gets pasted into bug
/// reports; an API key must never survive that trip.
static SECRETS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn secrets() -> &'static Mutex<Vec<String>> {
    SECRETS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Register a value that must be redacted wherever it appears in debug output.
pub fn add_secret(secret: &str) {
    // Very short values would redact half the output; a real key is nowhere near this.
    if secret.len() < 8 {
        return;
    }
    let mut guard = secrets().lock().expect("secrets mutex poisoned");
    if !guard.iter().any(|existing| existing == secret) {
        guard.push(secret.to_string());
    }
}

/// Replace every registered secret with a fingerprint that is still distinguishable
/// (so you can tell "wrong key" from "no key") but not usable.
pub fn redact(text: &str) -> String {
    let guard = secrets().lock().expect("secrets mutex poisoned");
    let mut out = text.to_string();
    for secret in guard.iter() {
        if !out.contains(secret.as_str()) {
            continue;
        }
        let head: String = secret.chars().take(3).collect();
        let tail: String = secret.chars().rev().take(2).collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        out = out.replace(
            secret.as_str(),
            &format!("{head}…{tail}[redacted {} chars]", secret.len()),
        );
    }
    out
}

const WIDTH: usize = 78;

fn rule(label: &str) -> String {
    let label = format!("── {label} ");
    let padding = WIDTH.saturating_sub(label.chars().count());
    format!("{label}{}", "─".repeat(padding))
}

/// Log an outgoing HTTP request.
pub fn request(method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>) {
    if !is_enabled() {
        return;
    }
    eprintln!("{}", rule("request"));
    eprintln!("{method} {}", redact(url));
    for (name, value) in headers {
        eprintln!("{name}: {}", redact(value));
    }
    if let Some(body) = body {
        eprintln!();
        eprintln!("{}", pretty_json(body));
    }
}

/// Log an HTTP response.
pub fn response(status: u16, elapsed: Duration, headers: &[(String, String)], body: &str) {
    if !is_enabled() {
        return;
    }
    eprintln!(
        "{}",
        rule(&format!("response {status} · {:.2}s", elapsed.as_secs_f64()))
    );
    for (name, value) in headers {
        eprintln!("{name}: {}", redact(value));
    }
    eprintln!();
    eprintln!("{}", pretty_json(body.as_bytes()));
    eprintln!("{}", "─".repeat(WIDTH));
}

/// Pretty-print if it parses as JSON, otherwise pass the bytes through as text. A
/// provider returning an HTML error page must still be readable.
fn pretty_json(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => redact(&serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string())),
        Err(_) => redact(&text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_registered_secrets() {
        add_secret("sk-supersecretkey123456");
        let out = redact("authorization: Bearer sk-supersecretkey123456");
        assert!(!out.contains("supersecretkey"), "{out}");
        // Still identifiable enough to tell two different wrong keys apart.
        assert!(out.contains("sk-"), "{out}");
        assert!(out.contains("redacted"), "{out}");
    }

    #[test]
    fn ignores_values_too_short_to_be_keys() {
        add_secret("abc");
        assert_eq!(redact("abc def"), "abc def");
    }

    #[test]
    fn pretty_prints_json_bodies() {
        let out = pretty_json(br#"{"a":1}"#);
        assert!(out.contains("\"a\": 1"), "{out}");
    }

    #[test]
    fn passes_non_json_through() {
        let out = pretty_json(b"<html>502 Bad Gateway</html>");
        assert_eq!(out, "<html>502 Bad Gateway</html>");
    }

    #[test]
    fn redacts_a_key_embedded_in_a_url() {
        // Some proxies put the key in the path, which is why registration happens at
        // config-load time rather than at the request site.
        add_secret("sk-inurl-abcdefgh12345");
        let out = redact("POST https://proxy.example/v1/sk-inurl-abcdefgh12345/chat");
        assert!(!out.contains("abcdefgh12345"), "{out}");
    }

    #[test]
    fn redaction_is_idempotent_for_repeated_registration() {
        add_secret("sk-duplicate-key-value");
        add_secret("sk-duplicate-key-value");
        let out = redact("sk-duplicate-key-value");
        assert_eq!(out.matches("redacted").count(), 1, "{out}");
    }
}
