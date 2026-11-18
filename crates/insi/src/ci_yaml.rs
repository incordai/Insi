// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! A reader for the YAML that GitHub workflows are written in, for `insi ci` (`ci.rs`).
//!
//! Insi carries no YAML dependency, and a workflow uses a small part of the language: block mappings and
//! sequences by indentation, plain, single- and double-quoted scalars, block scalars (`|`, `|-`, `>`, `>-`) for the
//! `run:` scripts, one-line flow sequences and mappings (`[a, b]`, `{ a: b }`) and comments. That is what this reads.
//! What it does not read (anchors and aliases, tags, a flow collection over several lines, a plain scalar continued
//! on the next line, tab indentation, more than one document) is an error naming the line, never a wrong guess: a
//! workflow `insi ci` cannot read is reported as unreadable.
//!
//! Called by `ci.rs` (`load`), which reads `.github/workflows/*.yml`. No other module reads YAML; the engine's and
//! Watchman's settings are JSON and TOML.

/// One YAML value. Every scalar is text (`true`, `3.9` and an empty value are all `Str`), as a workflow's own
/// expressions treat them; a mapping keeps its keys in the file's order.
#[derive(Debug, Clone, PartialEq)]
pub enum Yaml {
    Str(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    /// The value under `key` of a mapping.
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The text of a scalar.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Yaml::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The items of a sequence.
    pub fn as_seq(&self) -> Option<&[Yaml]> {
        match self {
            Yaml::Seq(items) => Some(items),
            _ => None,
        }
    }

    /// The pairs of a mapping, in the file's order.
    pub fn as_map(&self) -> Option<&[(String, Yaml)]> {
        match self {
            Yaml::Map(pairs) => Some(pairs),
            _ => None,
        }
    }
}

/// `text` as one YAML document, or why not (with the line).
pub fn parse(text: &str) -> Result<Yaml, String> {
    let mut parser = Parser {
        lines: text
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect(),
        at: 0,
    };
    if parser
        .lines
        .iter()
        .any(|l| l.starts_with('\t') || l.trim_start_matches(' ').starts_with('\t'))
    {
        return Err("a tab in the indentation (YAML indents with spaces)".into());
    }
    parser.skip_to_content();
    if parser.peek().is_some_and(|(_, c)| c == "---") {
        parser.at += 1;
    }
    let Some((indent, _)) = parser.peek() else {
        return Ok(Yaml::Map(Vec::new()));
    };
    let value = parser.block(indent)?;
    if let Some((_, content)) = parser.peek() {
        if content == "---" || content == "..." {
            return Err(format!(
                "line {}: more than one YAML document is not supported",
                parser.at + 1
            ));
        }
        return Err(format!(
            "line {}: unexpected content after the document",
            parser.at + 1
        ));
    }
    Ok(value)
}

struct Parser {
    lines: Vec<String>,
    at: usize,
}

impl Parser {
    /// Past blank lines and comment lines.
    fn skip_to_content(&mut self) {
        while self.at < self.lines.len() {
            let t = self.lines[self.at].trim();
            if t.is_empty() || t.starts_with('#') {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    /// The next content line as (indent, text after the indent), without moving.
    fn peek(&mut self) -> Option<(usize, String)> {
        self.skip_to_content();
        let line = self.lines.get(self.at)?;
        let indent = line.len() - line.trim_start_matches(' ').len();
        Some((indent, line[indent..].trim_end().to_string()))
    }

    /// The value that starts at the next content line, at `indent`.
    fn block(&mut self, indent: usize) -> Result<Yaml, String> {
        let Some((_, content)) = self.peek() else {
            return Ok(Yaml::Str(String::new()));
        };
        if content == "-" || content.starts_with("- ") {
            self.sequence(indent)
        } else if split_key(&content).is_some() {
            self.mapping(indent)
        } else {
            self.at += 1;
            scalar_or_flow(&content, self.at)
        }
    }

    fn sequence(&mut self, indent: usize) -> Result<Yaml, String> {
        let mut items = Vec::new();
        while let Some((at, content)) = self.peek() {
            if at != indent || !(content == "-" || content.starts_with("- ")) {
                if at > indent {
                    return Err(format!("line {}: unexpected indentation", self.at + 1));
                }
                break;
            }
            let rest = content[1..].trim_start();
            if rest.is_empty() {
                self.at += 1;
                match self.peek() {
                    Some((next, _)) if next > indent => items.push(self.block(next)?),
                    _ => items.push(Yaml::Str(String::new())),
                }
            } else {
                // `- key: value` and its continuation lines sit at the column after the dash: re-read this line as
                // the first line of a block at that column.
                let column = indent + (content.len() - rest.len());
                self.lines[self.at] = format!("{}{rest}", " ".repeat(column));
                items.push(self.block(column)?);
            }
        }
        Ok(Yaml::Seq(items))
    }

    fn mapping(&mut self, indent: usize) -> Result<Yaml, String> {
        let mut pairs: Vec<(String, Yaml)> = Vec::new();
        while let Some((at, content)) = self.peek() {
            if at < indent {
                break;
            }
            if at > indent {
                return Err(format!("line {}: unexpected indentation", self.at + 1));
            }
            if content == "---" || content == "..." {
                break;
            }
            let Some((key, rest)) = split_key(&content) else {
                if content == "-" || content.starts_with("- ") {
                    break;
                }
                return Err(format!("line {}: expected `key: value`", self.at + 1));
            };
            let line = self.at + 1;
            self.at += 1;
            let value = if rest.is_empty() {
                match self.peek() {
                    Some((next, _)) if next > indent => self.block(next)?,
                    // A sequence may sit at its key's own indentation.
                    Some((next, c)) if next == indent && (c == "-" || c.starts_with("- ")) => {
                        self.sequence(indent)?
                    }
                    _ => Yaml::Str(String::new()),
                }
            } else if let Some(header) = block_scalar_header(&rest) {
                Yaml::Str(self.block_scalar(indent, header))
            } else {
                scalar_or_flow(&rest, line)?
            };
            if pairs.iter().any(|(k, _)| *k == key) {
                return Err(format!("line {line}: the key `{key}` is given twice"));
            }
            pairs.push((key, value));
        }
        Ok(Yaml::Map(pairs))
    }

    /// The text of a block scalar whose key is at `indent`: the lines indented deeper, less their common indent.
    fn block_scalar(&mut self, indent: usize, header: BlockHeader) -> String {
        let mut body: Vec<String> = Vec::new();
        while self.at < self.lines.len() {
            let line = &self.lines[self.at];
            if line.trim().is_empty() {
                body.push(String::new());
                self.at += 1;
                continue;
            }
            let at = line.len() - line.trim_start_matches(' ').len();
            if at <= indent {
                break;
            }
            body.push(line.clone());
            self.at += 1;
        }
        // Trailing blank lines belong to what follows, not to the text.
        while body.last().is_some_and(String::is_empty) {
            body.pop();
        }
        let common = body
            .iter()
            .filter(|l| !l.is_empty())
            .map(|l| l.len() - l.trim_start_matches(' ').len())
            .min()
            .unwrap_or(0);
        let lines: Vec<String> = body
            .iter()
            .map(|l| {
                if l.is_empty() {
                    String::new()
                } else {
                    l[common..].to_string()
                }
            })
            .collect();
        let mut text = if header.folded {
            let mut out = String::new();
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    out.push(if line.is_empty() || lines[i - 1].is_empty() {
                        '\n'
                    } else {
                        ' '
                    });
                }
                out.push_str(line);
            }
            out
        } else {
            lines.join("\n")
        };
        if !header.strip && !text.is_empty() {
            text.push('\n');
        }
        text
    }
}

struct BlockHeader {
    folded: bool,
    strip: bool,
}

/// `|`, `|-`, `|+`, `>`, `>-`, `>+` (an indentation digit is accepted and ignored), possibly with a comment.
fn block_scalar_header(rest: &str) -> Option<BlockHeader> {
    let rest = rest.split(" #").next().unwrap_or(rest).trim();
    let mut chars = rest.chars();
    let folded = match chars.next()? {
        '|' => false,
        '>' => true,
        _ => return None,
    };
    let mut strip = false;
    for c in chars {
        match c {
            '-' => strip = true,
            '+' | '0'..='9' => {}
            _ => return None,
        }
    }
    Some(BlockHeader { folded, strip })
}

/// `key: rest` of a mapping line: the key, unquoted, and the text after the colon (without a trailing comment).
/// `None` when the line is not a mapping line (a plain scalar, even one holding a colon not followed by a space).
fn split_key(content: &str) -> Option<(String, String)> {
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' if i == 0 => quote = Some(c),
                '{' | '[' if i == 0 => return None,
                '$' if chars.get(i + 1) == Some(&'{') && chars.get(i + 2) == Some(&'{') => {
                    depth += 1
                }
                '}' if depth > 0 && chars.get(i + 1) == Some(&'}') => depth -= 1,
                ':' if depth == 0 && (i + 1 == chars.len() || chars[i + 1] == ' ') => {
                    let key: String = chars[..i].iter().collect();
                    let rest: String = chars[i + 1..].iter().collect();
                    let key = unquote_key(key.trim());
                    return Some((key, strip_comment(rest.trim())));
                }
                '#' if i > 0 && chars[i - 1] == ' ' => return None,
                _ => {}
            },
        }
        i += 1;
    }
    None
}

fn unquote_key(key: &str) -> String {
    for q in ['\'', '"'] {
        if key.len() >= 2 && key.starts_with(q) && key.ends_with(q) {
            return key[1..key.len() - 1].to_string();
        }
    }
    key.to_string()
}

/// `text` without a trailing ` # comment` (a `#` inside quotes is kept).
fn strip_comment(text: &str) -> String {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in text.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if (c == '\'' || c == '"')
                    && (i == 0 || prev == ' ' || prev == '[' || prev == ',' || prev == '{')
                {
                    quote = Some(c);
                } else if c == '#' && prev == ' ' {
                    return text[..i].trim_end().to_string();
                }
            }
        }
        prev = c;
    }
    text.to_string()
}

/// A value written on one line: a flow sequence or mapping, or a scalar.
fn scalar_or_flow(text: &str, line: usize) -> Result<Yaml, String> {
    let text = strip_comment(text.trim());
    if text.starts_with('[') || text.starts_with('{') {
        let (value, used) = flow(&text, line)?;
        if text[used..].trim().is_empty() {
            return Ok(value);
        }
        return Err(format!(
            "line {line}: unexpected text after a flow collection"
        ));
    }
    if text.starts_with('&') || text.starts_with('*') {
        return Err(format!(
            "line {line}: YAML anchors and aliases are not supported"
        ));
    }
    scalar(&text, line)
}

fn scalar(text: &str, line: usize) -> Result<Yaml, String> {
    if let Some(inner) = text.strip_prefix('\'') {
        let body = inner.strip_suffix('\'').ok_or_else(|| {
            format!("line {line}: a single-quoted text is not closed on its line")
        })?;
        return Ok(Yaml::Str(body.replace("''", "'")));
    }
    if let Some(inner) = text.strip_prefix('"') {
        let body = inner.strip_suffix('"').ok_or_else(|| {
            format!("line {line}: a double-quoted text is not closed on its line")
        })?;
        let mut out = String::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        return Ok(Yaml::Str(out));
    }
    Ok(Yaml::Str(text.to_string()))
}

/// One flow collection at the start of `text`: the value and how many bytes it used.
fn flow(text: &str, line: usize) -> Result<(Yaml, usize), String> {
    let open = text.chars().next().unwrap_or(' ');
    let close = if open == '[' { ']' } else { '}' };
    let mut items: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    let mut used = None;
    for (i, c) in text.char_indices().skip(1) {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    current.push(c);
                }
                '[' | '{' => {
                    depth += 1;
                    current.push(c);
                }
                ']' | '}' if depth > 0 => {
                    depth -= 1;
                    current.push(c);
                }
                c2 if c2 == close => {
                    used = Some(i + 1);
                    break;
                }
                ',' if depth == 0 => items.push(std::mem::take(&mut current)),
                _ => current.push(c),
            },
        }
    }
    let used = used
        .ok_or_else(|| format!("line {line}: a flow collection that is not closed on its line"))?;
    if !current.trim().is_empty() {
        items.push(current);
    }
    if open == '[' {
        let mut seq = Vec::new();
        for item in items {
            seq.push(scalar_or_flow(item.trim(), line)?);
        }
        return Ok((Yaml::Seq(seq), used));
    }
    let mut pairs = Vec::new();
    for item in items {
        let item = item.trim();
        let (key, rest) =
            split_key(item).ok_or_else(|| format!("line {line}: `{item}` is not `key: value`"))?;
        pairs.push((
            key,
            if rest.is_empty() {
                Yaml::Str(String::new())
            } else {
                scalar_or_flow(&rest, line)?
            },
        ));
    }
    Ok((Yaml::Map(pairs), used))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(y: &Yaml, key: &str) -> String {
        y.get(key)
            .and_then(Yaml::as_str)
            .unwrap_or("<none>")
            .to_string()
    }

    /// A workflow's shapes: nested mappings, a sequence of mappings with `- key:` starts, block scalars, quoted
    /// scalars holding colons and hashes, a flow sequence, comments and a sequence at its key's own indentation.
    #[test]
    fn reads_the_shapes_a_workflow_uses() {
        let y = parse(
            "name: ci # the name\n\
             on:\n  push:\n    branches: [main, 'release/*']\n  pull_request:\n\
             env:\n  RUST_LOG: \"a: b # not a comment\"\n\
             jobs:\n  test:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, windows-latest]\n\
             \x20   steps:\n      - uses: actions/checkout@v7 # pinned\n        with:\n          persist-credentials: false\n\
             \x20     - name: Build\n        run: |\n          cargo build\n          cargo test --locked\n\n\
             \x20     - run: >-\n          one\n          two\n\
             \x20 other:\n    needs: [test]\n    steps:\n    - run: echo hi\n",
        )
        .unwrap();
        assert_eq!(text(&y, "name"), "ci");
        assert_eq!(
            y.get("on")
                .and_then(|o| o.get("push"))
                .and_then(|p| p.get("branches"))
                .and_then(Yaml::as_seq)
                .map(<[Yaml]>::len),
            Some(2)
        );
        assert_eq!(
            y.get("env").map(|e| text(e, "RUST_LOG")).as_deref(),
            Some("a: b # not a comment")
        );
        let test = y.get("jobs").and_then(|j| j.get("test")).unwrap();
        assert_eq!(text(test, "runs-on"), "${{ matrix.os }}");
        assert_eq!(
            test.get("strategy")
                .and_then(|s| s.get("matrix"))
                .and_then(|m| m.get("os"))
                .and_then(Yaml::as_seq)
                .map(<[Yaml]>::len),
            Some(2)
        );
        let steps = test.get("steps").and_then(Yaml::as_seq).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(text(&steps[0], "uses"), "actions/checkout@v7");
        assert_eq!(
            steps[0]
                .get("with")
                .map(|w| text(w, "persist-credentials"))
                .as_deref(),
            Some("false")
        );
        assert_eq!(text(&steps[1], "run"), "cargo build\ncargo test --locked\n");
        assert_eq!(text(&steps[2], "run"), "one two");
        let other = y.get("jobs").and_then(|j| j.get("other")).unwrap();
        assert_eq!(
            other
                .get("needs")
                .and_then(Yaml::as_seq)
                .and_then(|n| n[0].as_str()),
            Some("test")
        );
        assert_eq!(
            other.get("steps").and_then(Yaml::as_seq).map(<[Yaml]>::len),
            Some(1)
        );
    }

    /// What it does not read is an error naming the line, never a wrong guess.
    #[test]
    fn refuses_what_it_cannot_read() {
        for (text, why) in [
            ("a: &x 1\n", "anchors"),
            ("a: *x\n", "anchors"),
            ("a: [1,\n  2]\n", "not closed"),
            ("a: 1\n---\nb: 2\n", "more than one"),
            ("a: 1\na: 2\n", "twice"),
            ("a:\n\tb: 1\n", "tab"),
            ("a: 'open\n", "not closed"),
        ] {
            let err = parse(text).unwrap_err();
            assert!(err.contains(why), "{text:?}: {err}");
        }
    }

    /// Insi's own workflows are read whole: every one has jobs, and every job has runs-on or a reusable workflow.
    #[test]
    fn reads_the_workflows_of_this_repository() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "yml") {
                continue;
            }
            let y = parse(&std::fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let jobs = y
                .get("jobs")
                .and_then(Yaml::as_map)
                .unwrap_or_else(|| panic!("{}: no jobs", path.display()));
            for (id, job) in jobs {
                assert!(
                    job.get("runs-on").is_some() || job.get("uses").is_some(),
                    "{}: job {id}",
                    path.display()
                );
            }
            count += 1;
        }
        assert!(count >= 4, "found {count} workflows");
    }
}
