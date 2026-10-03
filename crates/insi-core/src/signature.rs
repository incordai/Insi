//! Error signatures: one failure, recognised the next time it happens.
//!
//! The same mistake prints differently every time — another line number,
//! another temp folder, another timestamp or hash. A lesson keyed on the raw
//! text would never match again. [`normalize`] removes exactly those parts
//! and keeps the words that say what went wrong.

/// The failure's lasting shape: terminal colour codes removed, line/column
/// numbers, paths, hex ids, timestamps and other numbers replaced by
/// placeholders, whitespace collapsed, and only the first `MAX_LINES`
/// meaningful lines kept. When some lines say what failed (an error, an
/// assertion, a failing test), only those are kept: a test runner also lists
/// the tests that PASSED, and those say nothing about the mistake.
pub fn normalize(text: &str) -> String {
    let text = strip_ansi(text);
    let failing: Vec<&str> = text.lines().filter(|l| says_what_failed(l)).collect();
    let source: Vec<&str> = if failing.is_empty() {
        text.lines().collect()
    } else {
        failing
    };
    let mut lines = Vec::new();
    for raw in source {
        let line = normalize_line(raw);
        if line.is_empty() || lines.last() == Some(&line) {
            continue;
        }
        lines.push(line);
        if lines.len() == MAX_LINES {
            break;
        }
    }
    lines.join("\n")
}

/// A signature longer than this says more about the run than the mistake.
pub const MAX_LINES: usize = 8;

/// Words that mark a line as saying what went wrong, across test runners and
/// compilers: `FAIL`, `Error:`, `panicked`, `assertion`, `expected`.
fn says_what_failed(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    ["fail", "error", "panicked", "assert", "expected"]
        .iter()
        .any(|w| l.contains(w))
}

/// Remove terminal escape sequences (`ESC [ … letter`, and a lone `ESC x`),
/// which colour a runner's output and differ by terminal, not by mistake.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // Parameters and intermediates, up to the final letter.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() || c == '~' {
                    break;
                }
            }
        } else {
            chars.next();
        }
    }
    out
}

fn normalize_line(raw: &str) -> String {
    let mut out = Vec::new();
    for word in raw.split_whitespace() {
        out.push(normalize_word(word));
    }
    // Placeholders that ended up side by side say the same thing once.
    out.dedup();
    out.join(" ")
}

fn normalize_word(word: &str) -> String {
    // Keep the punctuation that wraps a word, judge what is inside it.
    let start = word
        .find(|c: char| c.is_alphanumeric() || c == '/' || c == '\\' || c == '.' || c == '~')
        .unwrap_or(word.len());
    let end = word
        .rfind(|c: char| c.is_alphanumeric() || c == '/' || c == '\\')
        .map_or(start, |i| i + 1)
        .max(start);
    let (lead, core, tail) = (&word[..start], &word[start..end], &word[end..]);
    let core = if core.is_empty() {
        String::new()
    } else if is_path(core) {
        "<path>".to_string()
    } else if is_hex_id(core) {
        "<hex>".to_string()
    } else if core.chars().any(|c| c.is_ascii_digit()) {
        replace_digit_runs(core)
    } else {
        core.to_string()
    };
    format!("{lead}{core}{tail}")
}

fn is_path(s: &str) -> bool {
    let has_sep = s.contains('/') || s.contains('\\');
    has_sep
        && (s.starts_with('/')
            || s.starts_with('.')
            || s.starts_with('~')
            || s.contains(':')
            || s.matches(['/', '\\']).count() >= 2)
}

fn is_hex_id(s: &str) -> bool {
    s.len() >= 7
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.chars().any(|c| c.is_ascii_digit())
}

/// `line 42:7` → `line <n>:<n>`, `2026-10-02T09:15:00Z` → `<n>-<n>-<n>T<n>:<n>:<n>Z`.
fn replace_digit_runs(s: &str) -> String {
    let mut out = String::new();
    let mut in_digits = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push_str("<n>");
                in_digits = true;
            }
        } else {
            in_digits = false;
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **ONE MISTAKE, ONE SIGNATURE.** Two runs of the same failure differ in
    /// line numbers, temp paths, hashes and timestamps; their signatures do
    /// not.
    #[test]
    fn the_same_failure_on_two_runs_has_one_signature() {
        let first = "error[E0308]: mismatched types\n  --> /tmp/run-8812/src/lib.rs:42:7\n   expected `u32`, found `String` at 2026-10-02T09:15:00Z (commit 1a2b3c4d5e)";
        let second = "error[E0308]: mismatched types\n  --> /tmp/run-9930/src/lib.rs:57:11\n   expected `u32`, found `String` at 2026-10-03T11:01:59Z (commit 9f8e7d6c5b)";
        assert_eq!(normalize(first), normalize(second));
        assert!(normalize(first).contains("mismatched types"));
        assert!(normalize(first).contains("found `String`"));
    }

    #[test]
    fn different_failures_stay_different() {
        assert_ne!(
            normalize("TypeError: cannot read properties of undefined (reading 'id')"),
            normalize("TypeError: cannot read properties of undefined (reading 'name')")
        );
    }

    /// **A TEST RUN'S SIGNATURE IS ITS FAILURE.** Colour codes are gone, and
    /// the tests that passed are not part of it.
    #[test]
    fn colour_and_passing_tests_are_left_out() {
        let run = "\u{1b}[32m✓\u{1b}[39m parser > reads a header \u{1b}[2m3ms\u{1b}[22m\n\
                   \u{1b}[31m×\u{1b}[39m parser > reads a chunk\n\
                   \u{1b}[41m FAIL \u{1b}[49m src/parse.test.ts > parser > reads a chunk\n\
                   \u{1b}[31mTypeError\u{1b}[39m: decodeChunk is not a function\n\
                   \u{1b}[32m✓\u{1b}[39m parser > reads a footer 1ms\n";
        let sig = normalize(run);
        assert!(!sig.contains('\u{1b}') && !sig.contains("[31m"), "{sig}");
        assert!(
            sig.contains("TypeError: decodeChunk is not a function"),
            "{sig}"
        );
        assert!(!sig.contains("reads a header"), "{sig}");
        // Unmarked output is still a signature, from every line.
        assert_eq!(
            normalize("segmentation fault at 0x0"),
            "segmentation fault at <n>x<n>"
        );
    }

    #[test]
    fn windows_paths_and_long_output_are_bounded() {
        let sig = normalize("failed at C:\\Users\\me\\AppData\\Local\\Temp\\x\\a.ts:3");
        assert_eq!(sig, "failed at <path>");
        let long = (0..50)
            .map(|i| format!("line {i} differs {}", "x".repeat(i)))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(normalize(&long).lines().count() <= MAX_LINES);
    }
}
