//! Insi's lessons.
//!
//! - [`writer`] turns one failed attempt and its verified fix into a lesson:
//!   the prompt a model is given, and the parsing of what it answers.
//! - [`store`] keeps lessons. The benchmark keeps them in a file of its own,
//!   so each experiment's lessons can be reset and reproduced and never mix
//!   with a person's working memory; Incord Memory is a second store behind
//!   the same trait.
//! - [`briefing`] turns stored lessons into the text put in front of a task.

pub mod briefing;
pub mod store;
pub mod writer;
