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

pub use config::{Config, Price, Worker};
pub use lesson::{Lesson, Verification};
pub use signature::normalize;
pub use task::{Budget, EvalResult, Task, Usage};
pub use trust::Trust;
