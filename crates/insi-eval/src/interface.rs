//! What the hidden tests need from the code, told to the worker as an
//! interface.
//!
//! A task's prompt is its fix commit's message, and the tests that check it
//! often import a module the fix created or a function it exported — names
//! the message never mentions. No worker can guess `isHiddenEditorHost` or
//! `src/lib/ompInterjections.ts`; measured on the first pilot, every failure
//! was one of these. A real issue would name the interface it wants, so the
//! worker is told it: the module paths and exported names the checking tests
//! import that do not exist at the base. Never the tests themselves — no
//! test name, assertion or expected value leaves the private store.
//!
//! Read from the tests' relative `import` statements (JavaScript and
//! TypeScript). Type-only imports are left out: they cannot fail a run.

use std::path::{Component, Path};

/// Something a checking test imports that is not there at the base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    /// Repository-relative module path as imported, without extension.
    pub module: String,
    /// The module does not exist at all.
    pub missing_module: bool,
    /// Value names imported that the module does not export (`default` for
    /// a default import). All of them when the module is missing.
    pub names: Vec<String>,
}

/// One relative import: its specifier and the value names it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Import {
    spec: String,
    names: Vec<String>,
}

/// The relative value imports in `source`.
fn imports(source: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut statement = String::new();
    let mut open = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if !open && trimmed.starts_with("import ") {
            open = true;
            statement.clear();
        }
        if !open {
            continue;
        }
        statement.push(' ');
        statement.push_str(trimmed);
        let Some(spec) = specifier(&statement) else {
            if trimmed.ends_with(';') {
                open = false;
            }
            continue;
        };
        open = false;
        if !spec.starts_with('.') {
            continue;
        }
        let clause = statement
            .trim()
            .trim_start_matches("import")
            .split(" from ")
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if clause.starts_with("type ") || clause.is_empty() {
            continue;
        }
        out.push(Import {
            spec,
            names: value_names(&clause),
        });
    }
    out
}

/// The quoted specifier after ` from `, once the statement has one.
fn specifier(statement: &str) -> Option<String> {
    let at = statement.rfind(" from ")?;
    let rest = statement[at + 6..].trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = rest[1..].find(quote)?;
    Some(rest[1..1 + end].to_string())
}

/// The value names an import clause takes: `{ a, b as c, type T }` gives
/// `a`, `b`; `X, { a }` gives `default`, `a`; `* as ns` gives nothing (the
/// whole module).
fn value_names(clause: &str) -> Vec<String> {
    let mut names = Vec::new();
    let (head, braced) = match (clause.find('{'), clause.rfind('}')) {
        (Some(open), Some(close)) if close > open => (&clause[..open], &clause[open + 1..close]),
        _ => (clause, ""),
    };
    let head = head.trim().trim_end_matches(',').trim();
    if !head.is_empty() && !head.starts_with('*') {
        names.push("default".to_string());
    }
    for item in braced.split(',') {
        let item = item.trim();
        if item.is_empty() || item.starts_with("type ") {
            continue;
        }
        let name = item.split(" as ").next().unwrap_or(item).trim();
        if !name.is_empty() {
            names.push(name.to_string());
        }
    }
    names
}

/// `spec` imported from `test`, as a repository-relative path.
fn resolve(test: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<String> = Path::new(test)
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| match c {
                    Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    for piece in spec.split('/') {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name.to_string()),
        }
    }
    Some(parts.join("/"))
}

/// Reads a repository-relative file at the base: from a worktree, or from a
/// commit (`git show base:path`) when there is none.
pub type Read<'a> = &'a dyn Fn(&str) -> Option<String>;

/// A [`Read`] over the files of a checkout.
pub fn in_tree(tree: &Path) -> impl Fn(&str) -> Option<String> + '_ {
    move |path| std::fs::read_to_string(tree.join(path)).ok()
}

/// The source of the file `module` names, trying the extensions a bundler
/// would, and `.js` written for a `.ts` source.
fn module_file(read: Read<'_>, module: &str) -> Option<String> {
    let stem = module
        .strip_suffix(".js")
        .or_else(|| module.strip_suffix(".jsx"))
        .unwrap_or(module);
    let mut candidates = vec![module.to_string()];
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        candidates.push(format!("{stem}.{ext}"));
        candidates.push(format!("{stem}/index.{ext}"));
    }
    candidates.into_iter().find_map(|c| read(&c))
}

/// Whether `source` exports `name`: on an `export` line, in an `export {}`
/// list, or anything at all when the module re-exports with `export *`
/// (which cannot be followed here, so it is not reported missing).
fn exports(source: &str, name: &str) -> bool {
    if name == "default" {
        return source
            .lines()
            .any(|l| l.trim().starts_with("export default"));
    }
    let word = |line: &str| {
        line.match_indices(name).any(|(at, _)| {
            let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
            line[..at].chars().next_back().is_none_or(|c| !ident(c))
                && line[at + name.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !ident(c))
        })
    };
    let mut in_list = false;
    for line in source.lines() {
        let t = line.trim();
        if t.starts_with("export *") {
            return true;
        }
        if t.starts_with("export") {
            if word(t) {
                return true;
            }
            in_list = t.contains('{') && !t.contains('}');
        } else if in_list {
            if word(t) {
                return true;
            }
            if t.contains('}') {
                in_list = false;
            }
        }
    }
    false
}

/// What the tests at `tests` (path, source) import that the base, read
/// through `read`, lacks. Modules that are themselves among the tests are
/// left out: they are copied in with the check.
pub fn needs(tests: &[(String, String)], read: Read<'_>) -> Vec<Need> {
    let own: Vec<String> = tests.iter().map(|(p, _)| p.replace('\\', "/")).collect();
    let mut out: Vec<Need> = Vec::new();
    for (path, source) in tests {
        for import in imports(source) {
            let Some(module) = resolve(&path.replace('\\', "/"), &import.spec) else {
                continue;
            };
            // A module that IS one of the copied files (a fixture beside the
            // tests): its path without the extension. Not a prefix match —
            // `x.test.ts` would make every module `x` look copied.
            if own
                .iter()
                .any(|t| t.rsplit_once('.').map(|(stem, _)| stem) == Some(module.as_str()))
            {
                continue;
            }
            let (missing_module, names) = match module_file(read, &module) {
                None => (true, import.names.clone()),
                Some(text) => {
                    let lacking: Vec<String> = import
                        .names
                        .iter()
                        .filter(|n| !exports(&text, n))
                        .cloned()
                        .collect();
                    if lacking.is_empty() {
                        continue;
                    }
                    (false, lacking)
                }
            };
            match out.iter_mut().find(|n| n.module == module) {
                Some(need) => {
                    for name in names {
                        if !need.names.contains(&name) {
                            need.names.push(name);
                        }
                    }
                }
                None => out.push(Need {
                    module,
                    missing_module,
                    names,
                }),
            }
        }
    }
    out
}

/// The interface, as a paragraph for the worker's prompt; empty when
/// nothing is missing.
pub fn brief(needs: &[Need]) -> String {
    if needs.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "The change is checked by tests you cannot see. They use this interface, which does \
         not exist yet:",
    );
    for need in needs {
        let names = need
            .names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        if need.missing_module {
            out.push_str(&format!("\n- create the module `{}`", need.module));
            if !names.is_empty() {
                out.push_str(&format!(", exporting {names}"));
            }
        } else {
            out.push_str(&format!("\n- export {names} from `{}`", need.module));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_imports_are_read_and_types_left_out() {
        let source = "import { describe, it } from \"vitest\";\n\
                      import type { Block } from \"../lib/session\";\n\
                      import { a, b as c, type T } from \"./mod\";\n\
                      import Thing, { x } from './thing';\n\
                      import * as all from \"./all\";\n\
                      import {\n  one,\n  two,\n} from \"./multi\";\n";
        let got = imports(source);
        assert_eq!(
            got,
            [
                Import {
                    spec: "./mod".into(),
                    names: vec!["a".into(), "b".into()]
                },
                Import {
                    spec: "./thing".into(),
                    names: vec!["default".into(), "x".into()]
                },
                Import {
                    spec: "./all".into(),
                    names: vec![]
                },
                Import {
                    spec: "./multi".into(),
                    names: vec!["one".into(), "two".into()]
                },
            ]
        );
    }

    #[test]
    fn exports_are_found_on_lines_and_in_lists() {
        let source = "export function decode(x) {}\nconst hidden = 1;\n\
                      export {\n  alpha,\n  beta as gamma,\n};\nexport default thing;\n";
        assert!(exports(source, "decode"));
        assert!(exports(source, "alpha"));
        assert!(exports(source, "default"));
        assert!(!exports(source, "hidden"), "defined but not exported");
        assert!(!exports(source, "deco"), "a prefix is not the name");
        assert!(exports("export * from './x';", "anything"));
    }

    /// **WHAT THE CHECK NEEDS AND THE BASE LACKS IS NAMED; NOTHING ELSE.**
    /// A missing module with its names; a name an existing module does not
    /// export; nothing for what is already there or for the tests' own files.
    #[test]
    fn missing_modules_and_names_are_named() {
        let dir = std::env::temp_dir().join(format!(
            "insi-interface-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(dir.join("src/surfaces")).expect("dir");
        std::fs::write(
            dir.join("src/surfaces/editorSearch.ts"),
            "export function findInEditor() {}\nfunction isHiddenEditorHost() {}\n",
        )
        .expect("write");
        let tests = vec![
            (
                "src/surfaces/editorSearch.test.ts".to_string(),
                "import { findInEditor, isHiddenEditorHost } from \"./editorSearch\";\n\
                 import { helper } from \"./fixtures\";\n"
                    .to_string(),
            ),
            (
                "src/surfaces/editorDoc.test.ts".to_string(),
                "import { docFor, lineAt } from \"./editorDoc\";\n".to_string(),
            ),
            ("src/surfaces/fixtures.ts".to_string(), String::new()),
        ];
        let got = needs(&tests, &in_tree(&dir));
        assert_eq!(
            got,
            [
                Need {
                    module: "src/surfaces/editorSearch".into(),
                    missing_module: false,
                    names: vec!["isHiddenEditorHost".into()]
                },
                Need {
                    module: "src/surfaces/editorDoc".into(),
                    missing_module: true,
                    names: vec!["docFor".into(), "lineAt".into()]
                },
            ]
        );
        let text = brief(&got);
        assert!(text.contains("export `isHiddenEditorHost` from `src/surfaces/editorSearch`"));
        assert!(text
            .contains("create the module `src/surfaces/editorDoc`, exporting `docFor`, `lineAt`"));
        assert!(!text.contains(".test."), "no test file is named: {text}");
        assert_eq!(brief(&[]), "");
    }

    #[test]
    fn a_path_climbs_out_of_the_test_folder() {
        assert_eq!(
            resolve("src/a/b/x.test.ts", "../../lib/y").as_deref(),
            Some("src/lib/y")
        );
        assert_eq!(resolve("x.test.ts", "../y"), None, "above the repository");
    }
}
