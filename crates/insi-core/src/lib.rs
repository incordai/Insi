//! Insi's shared vocabulary: the per-repo config, a benchmark task and its
//! budget, the result of one run, and the error-signature normalizer that
//! lets one failure be recognised the next time it happens.
//!
//! Nothing here touches the network, git or a process: it is the part every
//! other crate (and, later, any project that adds Insi as a library) agrees
//! on.

pub mod config;
pub mod lesson;
pub mod signature;
pub mod task;

pub use config::{Config, Worker};
pub use lesson::{Lesson, Verification};
pub use signature::normalize;
pub use task::{Budget, EvalResult, Task};
