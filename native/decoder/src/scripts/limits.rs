//! The bounds the decoder holds over a channel's scripts as the authority
//! (the script API's specification sections 10.5 and 10.6): a script host
//! holds each of these too, so that an author is told at the call, but a
//! script host that has been subverted cannot be relied on to count, so
//! one that passes any of them here is failed (TODO-7293 step 5).
//!
//! The script library names the same bounds (`neoteletext_script::limits`
//! and its renderers' modules), which this process may not link
//! (requirement 1.3): the script-host program's tests hold each pair
//! equal.

use neoteletext_rules::limits::TICKS_PER_SECOND;

/// The most bytes from a script file's start to the end of its manifest
/// (`MAX_MANIFEST_BYTES`, section 3.1).
pub const MAX_MANIFEST_BYTES: usize = 4096;
/// The most bytes of renderer output of every kind a channel's scripts
/// may give in any second of ticks (`MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND`,
/// section 10.5): four batches at their bound.
pub const MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND: usize = 1024 * 1024;
const _: () = assert!(MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND >= protocol::script::batch::MAX_BATCH_BYTES, "a batch at its bound is within a second's output");
/// The fewest ticks between two generations of a page's NAPLPS picture
/// (`NAPLPS_REPAINT_TICKS`, section 10.5): half a second.
pub const NAPLPS_REPAINT_TICKS: u64 = 30;
/// The most NAPLPS bytes appended in any second of ticks
/// (`NAPLPS_MAX_BYTES_PER_SECOND`).
pub const NAPLPS_MAX_BYTES_PER_SECOND: usize = 262_144;
/// The most redefinitions of one colour entry in any second of ticks
/// (`MAX_COLOUR_SETS_PER_SECOND`): three flashes, the most that is safe
/// for a viewer (the security review's finding S-10).
pub const MAX_COLOUR_SETS_PER_SECOND: usize = 6;
/// The most sound operations in any second of ticks
/// (`MAX_SOUND_OPERATIONS_PER_SECOND`).
pub const MAX_SOUND_OPERATIONS_PER_SECOND: usize = 256;
/// The most navigations a script may ask in any second of ticks
/// (`MAX_NAVIGATIONS_PER_SECOND`).
pub const MAX_NAVIGATIONS_PER_SECOND: usize = 2;
/// The ticks a rate's second holds.
pub const SECOND_TICKS: u64 = TICKS_PER_SECOND as u64;
/// The most pointers followed at once (`MAX_POINTERS`); the trusted core
/// gives one today, the mouse or the first contact.
pub const MAX_POINTERS: usize = 4;
/// The most input events that wait for a step (`MAX_QUEUED_EVENTS`,
/// section 4.7): the rule table's bound, by its name.
pub const MAX_QUEUED_EVENTS: usize = neoteletext_rules::limits::MAX_QUEUED_EVENTS;
/// The most script files a channel's decoder keeps whole: the channel's
/// and one for each page it may show at once.
pub const MAX_SCRIPTS_HELD: usize = 1 + protocol::MAX_TABS;
/// The API version a script is written for is at least this (section 3.1).
pub const FIRST_API: u16 = 1;
/// The most pages whose own file's script name is remembered: every page
/// a channel may name a script for (the engine's `MAX_SCRIPT_PAGES`).
pub const MAX_NAMED_PAGES: usize = neoteletext_web::scripts::MAX_SCRIPT_PAGES;
