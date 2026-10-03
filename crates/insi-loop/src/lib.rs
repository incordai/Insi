//! The improvement loop's parts that decide, and nothing that runs.
//!
//! The loop changes the harness — the folder of prompts and settings a
//! worker runs with — one file at a time, and keeps a change only when the
//! benchmark says it is better:
//!
//! - [`harness`]: a harness as a set of text files, its content hash, and a
//!   change applied to it.
//! - [`proposal`]: the ONE change a proposer answers with, checked before it
//!   touches anything.
//! - [`accept`]: the rule. A candidate must beat the incumbent by a margin
//!   on the selection tasks, over the same task × seed pairs, and not be
//!   worse on tasks never used for selection.
//! - [`bandit`]: UCB1 over named proposal strategies.
//! - [`audit`]: every state change, one JSON line each.
//!
//! Running attempts, the proposer and the worker is the `insi improve`
//! command's job; everything here is deterministic and tested on its own.

pub mod accept;
pub mod audit;
pub mod bandit;
pub mod harness;
pub mod proposal;
