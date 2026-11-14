#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "test fixtures fail immediately on broken setup or assertions"
    )
)]
//! Insi's shared vocabulary: the per-repo config, a benchmark task and its
//! budget, the result of one run, and the error-signature normalizer that
//! lets one failure be recognised the next time it happens.
//!
//! Nothing here touches the network, git or a process: it is the part every
//! other crate (and, later, any project that adds Insi as a library) agrees
//! on. The one piece of filesystem machinery, [`lock`], is here because every
//! crate that writes a shared file needs the same one.

pub mod config;
pub mod lesson;
pub mod lock;
pub mod secrets;
pub mod signature;
pub mod task;
pub mod topic;
pub mod trust;

pub use config::{Config, Places, Price, Worker};
pub use lesson::{Lesson, Verification};
pub use signature::normalize;
pub use task::{Budget, EvalResult, Isolation, Task, Usage};
pub use trust::Trust;

/// Extract the first complete object of the requested type, ignoring braces
/// in surrounding prose. Proposal and lesson parsers retain their own validation.
pub fn first_json_object<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    for (start, _) in text.match_indices('{') {
        if let Some(Ok(value)) = serde_json::Deserializer::from_str(&text[start..])
            .into_iter::<T>()
            .next()
        {
            return Ok(value);
        }
    }
    Err("the answer has no valid JSON object".into())
}

/// Prose braces and fenced output must not hide the first typed object.
#[test]
fn json_extraction_ignores_prose_and_keeps_the_first_object() {
    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Answer {
        value: u32,
    }
    for text in [
        "before {placeholder} {\"value\":1} after {anything}",
        "```json\n{\"value\":1}\n```",
        "{\"value\":1} {\"value\":2}",
    ] {
        assert_eq!(first_json_object::<Answer>(text), Ok(Answer { value: 1 }));
    }
    assert!(first_json_object::<Answer>("no object").is_err());
}
