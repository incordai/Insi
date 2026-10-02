//! Credential and personal-data shapes in what is about to be pushed.
//!
//! Before Insi pushes a branch or opens a pull request, the lines the branch
//! adds are checked for things shaped like credentials or personal data, and
//! the files it adds whose names make them credentials. A finding names the
//! file and line and what it looked like — never the value.
//!
//! Personal data ([`pii_in`]) is held to the same precision: a card number
//! must pass the Luhn check and be written like one, a US social security
//! number must be in a range that is issued, and an email address must not
//! be a reserved or placeholder one (`example.com`, `.test`, `.invalid`,
//! `noreply`), which is what fixtures and docs use.
//!
//! The same high-precision shapes as incode's `/pr` scan (a prefixed key, a
//! cloud key id, a JWT, a private-key block, a bearer token), matched by hand:
//! Insi has no regular-expression dependency and these do not need one. The
//! generic `token: <value>` shape is left out on purpose — in code it matches
//! `token: String` on every other line.

/// One credential-shaped thing in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    /// The line in the new file (1-based); 0 for a whole file.
    pub line: usize,
    pub what: &'static str,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.what)
        } else {
            write!(f, "{}:{}: {}", self.file, self.line, self.what)
        }
    }
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `text` holds `prefix` at a word start followed by at least
/// `min` characters of `allowed`.
fn prefixed(text: &str, prefix: &str, min: usize, allowed: fn(char) -> bool) -> bool {
    text.match_indices(prefix).any(|(at, _)| {
        let starts_word = text[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_');
        starts_word
            && text[at + prefix.len()..]
                .chars()
                .take_while(|c| allowed(*c))
                .count()
                >= min
    })
}

fn jwt(text: &str) -> bool {
    text.match_indices("eyJ").any(|(at, _)| {
        let rest: String = text[at..]
            .chars()
            .take_while(|c| token_char(*c) || *c == '.')
            .collect();
        let parts: Vec<&str> = rest.split('.').collect();
        parts.len() >= 3 && parts.iter().take(3).all(|p| p.len() >= 8)
    })
}

/// What credential `line` holds the shape of, if any.
pub fn shape_in(line: &str) -> Option<&'static str> {
    let upper_or_digit = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit();
    if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|p| prefixed(line, p, 20, |c| c.is_ascii_alphanumeric()))
        || prefixed(line, "github_pat_", 20, |c| {
            c.is_ascii_alphanumeric() || c == '_'
        })
    {
        return Some("a GitHub token");
    }
    if prefixed(line, "sk-", 20, token_char) {
        return Some("an API key (sk-…)");
    }
    if ["xoxa-", "xoxb-", "xoxp-", "xoxr-", "xoxs-"]
        .iter()
        .any(|p| prefixed(line, p, 10, |c| c.is_ascii_alphanumeric() || c == '-'))
    {
        return Some("a Slack token");
    }
    if prefixed(line, "AKIA", 16, upper_or_digit) {
        return Some("a cloud access key id");
    }
    if jwt(line) {
        return Some("a JSON web token");
    }
    if line.contains("-----BEGIN ") && line.contains("PRIVATE KEY-----") {
        return Some("a private key");
    }
    let lower = line.to_ascii_lowercase();
    if let Some(at) = lower.find("bearer ") {
        let token: String = line[at + 7..]
            .trim_start()
            .chars()
            .take_while(|c| token_char(*c) || "._~+/=".contains(*c))
            .collect();
        if token.len() >= 20 {
            return Some("a bearer token");
        }
    }
    None
}

/// What personal data `line` holds the shape of, if any.
pub fn pii_in(line: &str) -> Option<&'static str> {
    if card_number(line) {
        return Some("a payment card number");
    }
    if us_ssn(line) {
        return Some("a US social security number");
    }
    if personal_email(line) {
        return Some("an email address");
    }
    None
}

/// Whether `digits` passes the Luhn check card numbers carry.
fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// A run of digits, spaces and dashes in `line` that is written like a card
/// number: four groups of four, or a bare 15–16 digits starting the way
/// issuers' numbers start (4, 51–55, 2221–2720, 34, 37, 6011, 65) — and in
/// both cases passing Luhn. Timestamps and ids are bare digits without a
/// card prefix or fail the check.
fn card_number(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || (i > 0 && bytes[i - 1].is_ascii_alphanumeric()) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_digit()
                || ((bytes[i] == b' ' || bytes[i] == b'-')
                    && i + 1 < bytes.len()
                    && bytes[i + 1].is_ascii_digit()))
        {
            i += 1;
        }
        if i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
            continue;
        }
        let run = &line[start..i];
        let groups: Vec<&str> = run.split([' ', '-']).collect();
        let digits: Vec<u32> = run.chars().filter_map(|c| c.to_digit(10)).collect();
        let grouped = groups.len() == 4 && groups.iter().all(|g| g.len() == 4);
        let bare = groups.len() == 1 && (15..=16).contains(&digits.len()) && {
            let prefix = |n: usize| run[..n].parse::<u32>().unwrap_or(0);
            run.starts_with('4')
                || (51..=55).contains(&prefix(2))
                || (2221..=2720).contains(&prefix(4))
                || [34, 37, 65].contains(&prefix(2))
                || prefix(4) == 6011
        };
        let same = digits.windows(2).all(|w| w[0] == w[1]);
        if (grouped || bare) && !same && luhn(&digits) {
            return true;
        }
    }
    false
}

/// `ddd-dd-dddd` as a word of its own, in a range the US issues: not area
/// 000, 666 or 900–999, not group 00, not serial 0000.
fn us_ssn(line: &str) -> bool {
    let b = line.as_bytes();
    (0..b.len().saturating_sub(10)).any(|i| {
        let shape = (0..11).all(|k| match k {
            3 | 6 => b[i + k] == b'-',
            _ => b[i + k].is_ascii_digit(),
        });
        let alone = (i == 0 || (!b[i - 1].is_ascii_alphanumeric() && b[i - 1] != b'-'))
            && b.get(i + 11)
                .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'-');
        if !shape || !alone {
            return false;
        }
        let area: u32 = line[i..i + 3].parse().unwrap_or(0);
        let group = &line[i + 4..i + 6];
        let serial = &line[i + 7..i + 11];
        area != 0 && area != 666 && area < 900 && group != "00" && serial != "0000"
    })
}

/// An email address whose domain is a real one: not a reserved or
/// placeholder domain, not a `noreply` address, not a `git@host` remote.
fn personal_email(line: &str) -> bool {
    let local_char = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain_char = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    line.match_indices('@').any(|(at, _)| {
        let local: String = line[..at]
            .chars()
            .rev()
            .take_while(|c| local_char(*c))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let domain: String = line[at + 1..]
            .chars()
            .take_while(|c| domain_char(*c))
            .collect();
        let domain = domain.trim_end_matches(['.', '-']).to_ascii_lowercase();
        let local = local.trim_start_matches('.').to_ascii_lowercase();
        let Some((_, tld)) = domain.rsplit_once('.') else {
            return false;
        };
        if local.is_empty() || tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
            return false;
        }
        let reserved = ["example.com", "example.org", "example.net", "localhost"]
            .iter()
            .any(|d| domain == *d || domain.ends_with(&format!(".{d}")))
            || ["example", "invalid", "test", "localhost", "local"].contains(&tld)
            || domain.ends_with("users.noreply.github.com");
        let placeholder = local == "git" || local.contains("noreply") || local.contains("no-reply");
        !reserved && !placeholder
    })
}

/// What a file is, when its name alone makes it a credential.
pub fn credential_file(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name == ".env"
        || (name.starts_with(".env.")
            && ![".env.example", ".env.sample", ".env.template", ".env.dist"]
                .contains(&name.as_str()))
    {
        return Some("an environment file");
    }
    if ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"].contains(&name.as_str()) {
        return Some("a private key file");
    }
    if [".pem", ".p12", ".pfx"].iter().any(|e| name.ends_with(e)) {
        return Some("a key or certificate store");
    }
    None
}

/// Every credential and piece of personal data in what a unified diff
/// ADDS. Removed lines are not looked at: deleting a secret is the fix, not
/// the leak.
pub fn scan_diff(diff: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut file = String::new();
    let mut new_line = 0usize;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            file = path.strip_prefix("b/").unwrap_or(path).to_string();
            if file != "/dev/null" {
                if let Some(what) = credential_file(&file) {
                    found.push(Finding {
                        file: file.clone(),
                        line: 0,
                        what,
                    });
                }
            }
        } else if line.starts_with("--- ") || line.starts_with("diff --git") {
        } else if let Some(rest) = line.strip_prefix("@@ ") {
            new_line = rest
                .split_whitespace()
                .find_map(|p| p.strip_prefix('+'))
                .and_then(|s| s.split(',').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        } else if let Some(added) = line.strip_prefix('+') {
            if let Some(what) = shape_in(added).or_else(|| pii_in(added)) {
                found.push(Finding {
                    file: file.clone(),
                    line: new_line,
                    what,
                });
            }
            new_line += 1;
        } else if line.starts_with(' ') {
            new_line += 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shaped like credentials and not ones: built at run time, so nothing
    // real is written down and no scanner reading this file is fooled.
    fn fake(prefix: &str, n: usize) -> String {
        format!("{prefix}{}", "Q".repeat(n))
    }

    #[test]
    fn each_shape_is_found() {
        for (line, what) in [
            (
                format!("token = \"{}\"", fake("ghp_", 36)),
                "a GitHub token",
            ),
            (format!("key: {}", fake("sk-", 30)), "an API key (sk-…)"),
            (fake("xoxb-", 20), "a Slack token"),
            (format!("id={}", fake("AKIA", 16)), "a cloud access key id"),
            (
                format!(
                    "jwt {}.{}.{}",
                    fake("eyJ", 10),
                    "Q".repeat(10),
                    "Q".repeat(10)
                ),
                "a JSON web token",
            ),
            (
                format!("-----BEGIN {}PRIVATE KEY-----", "RSA "),
                "a private key",
            ),
            (
                format!("Authorization: Bearer {}", "Q".repeat(30)),
                "a bearer token",
            ),
        ] {
            assert_eq!(shape_in(&line), Some(what), "{line}");
        }
    }

    /// **ORDINARY CODE IS NOT A CREDENTIAL.** Code that names a token, a
    /// short prefix, a prefix inside a longer word.
    #[test]
    fn ordinary_code_is_not_flagged() {
        for line in [
            "let token: String = read_token();",
            "const askForKey = (name) => name;",
            "risk-assessment",
            "task-1234567890123456789012345",
            "ghp_short",
            "Bearer token is read from the keychain",
        ] {
            assert_eq!(shape_in(line), None, "{line}");
        }
    }

    /// **PERSONAL DATA IS FOUND, AND WHAT LOOKS LIKE IT IS NOT.** Card
    /// numbers written like card numbers that pass Luhn, issued-range SSNs,
    /// real-domain emails — and not epoch milliseconds, version strings,
    /// placeholder emails, git remotes or dates.
    #[test]
    fn personal_data_is_found_and_lookalikes_are_not() {
        // Standard test card numbers: Luhn-valid, never issued to anyone.
        let visa = ["4111", "1111", "1111", "1111"];
        for (line, what) in [
            (format!("card: {}", visa.join(" ")), "a payment card number"),
            (format!("card={}", visa.join("-")), "a payment card number"),
            (format!("pan {}", visa.concat()), "a payment card number"),
            (
                format!("ssn: {}-{}-{}", "123", "45", "6789"),
                "a US social security number",
            ),
            (
                format!("contact {}@{}", "jane.doe", "gmail.com"),
                "an email address",
            ),
        ] {
            assert_eq!(pii_in(&line), Some(what), "{line}");
        }
        for line in [
            "const now = 1696212345678;",
            "id = 4111111111111112",
            "let n = 1234 5678 9012 3456;",
            "version 1.2.3-4567-89",
            "date 2026-10-02",
            "ssn: 000-12-3456",
            "author: Jo <jo@example.com>",
            "Co-Authored-By: bot <noreply@anthropic.com>",
            "url = git@github.com:o/r.git",
            "npm i @scope/pkg@1.2.3",
            "send to admin@service.test",
            "1234567890123456",
        ] {
            assert_eq!(pii_in(line), None, "{line}");
        }
    }

    #[test]
    fn a_diff_is_scanned_by_its_added_lines_and_new_files() {
        let token = fake("gho_", 30);
        let diff = format!(
            "--- a/src/a.ts\n+++ b/src/a.ts\n@@ -3,2 +3,3 @@\n context\n+const t = \"{token}\";\n-const old = \"{}\";\n\
             --- /dev/null\n+++ b/.env\n@@ -0,0 +1 @@\n+MODE=dev\n\
             --- /dev/null\n+++ b/.env.example\n@@ -0,0 +1 @@\n+API_KEY=\n",
            fake("sk-", 30)
        );
        let found = scan_diff(&diff);
        let shown: Vec<String> = found.iter().map(|f| f.to_string()).collect();
        assert_eq!(
            shown,
            ["src/a.ts:4: a GitHub token", ".env: an environment file"]
        );
        assert!(!shown.join("").contains(&token), "the value leaked");
    }
}
