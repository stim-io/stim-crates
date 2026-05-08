//! Atomic helpers wrappers use to extract a sidecar's listening
//! endpoint from a child process's stdout.
//!
//! The renderer-wrapper pattern: a Rust wrapper crate spawns a
//! tool that doesn't speak the native sidecar ready-line protocol
//! (e.g. vite, pnpm dev, any "Local: http://..." style logger),
//! reads its stdout, and synthesizes a `SidecarReadyLine` itself.
//!
//! This module provides exactly the per-line primitives — ANSI
//! stripping and regex endpoint extraction. The wrapper owns the
//! reader-driving loop (sync or async, tee-to-parent-stdout or
//! not, retry-on-restart or not — wrapper's call), composing
//! `strip_ansi` + `extract_endpoint` as it sees fit.

use regex::Regex;

/// Strip ANSI CSI escape sequences (color codes etc.) from a
/// stdout line so a regex match doesn't have to account for the
/// escapes injected by tools like vite, cargo, or pnpm.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Try to extract a sidecar endpoint from one stdout line.
/// `pattern` should have one capture group whose match is the
/// endpoint URL.
///
/// Returns `Some(endpoint)` on first match, `None` otherwise.
/// The line is ANSI-stripped before matching, so wrappers can
/// hand `extract_endpoint` raw output without preprocessing.
pub fn extract_endpoint(line: &str, pattern: &Regex) -> Option<String> {
    let stripped = strip_ansi(line);
    pattern
        .captures(&stripped)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}
