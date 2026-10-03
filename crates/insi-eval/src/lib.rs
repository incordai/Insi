//! Insi's benchmark.
//!
//! - [`mine`] turns a repository's history into tasks: each fix commit that
//!   changed a separate test file becomes "start from the commit before, do
//!   what the commit message asks", judged by that test file — which is kept
//!   apart, as the hidden check.
//! - [`run`] runs one attempt in a throwaway worktree of a private clone: the
//!   worker, then the hidden check copied in only after the worker stopped.
//!   The same code proves a task fair: at its base the check must fail, at
//!   its reference commit it must pass.
//! - [`report`] sums attempts per task and per condition.
//! - [`interface`] reads what the hidden tests import that the base lacks,
//!   so the worker is told the interface a real issue would name.
//!
//! Nothing here edits the source repository: tasks are read from its
//! history, and every worktree lives in Insi's own clone.

pub mod git;
pub mod interface;
pub mod mine;
pub mod pool;
pub mod process;
pub mod report;
pub mod run;
pub mod store;
