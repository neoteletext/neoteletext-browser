//! The open page's rule table in the decoder (neoteletext-common's
//! `docs/page-rules.md`, RULE-58): the confined decoder of a page's
//! channel decodes the RULES record with the engine's `neoteletext-rules`,
//! holds the table and runs its evaluator on the presentation clock, sixty
//! ticks a second (RULE-4). The trusted core never decodes or evaluates a
//! table; it gates what goes in and out (RULE-52).

pub(crate) mod host;
pub mod picture;

pub use host::MAX_KEPT_TABLES;
pub use picture::{Picture, Still};
