//! The teletext engine as the decoder process uses it. Bytes of any format
//! the engine reads (T42 recordings, TTI page files, transport streams,
//! sliced records) go through its format detection and readers into its
//! decoder and page store; pages are composed at the presentation level of
//! the engine's default profile and rendered by its row renderer with its
//! bundled font, Bedstead, into palette indices.
//!
//! Everything here reads untrusted bytes, which is why it runs in a process
//! of its own with no network and no files (design section 5).

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

mod changes;
mod engine;
mod flash;
#[cfg(feature = "interaction")]
mod input_records;
#[cfg(feature = "links")]
pub mod links;
pub mod netscape;
pub mod process;
pub mod sample;
mod title;
mod voice_sink;

pub use changes::Changed;
#[cfg(feature = "links")]
pub use engine::Found;
pub use engine::{ChannelLog, Engine, Exported, Rendered};
pub use flash::{FlashRates, Flashing};
#[cfg(feature = "interaction")]
pub use input_records::InputRecords;

const _: () = assert!(protocol::decoder::MAX_CHANNEL_LOG == neoteletext_formats::log::MAX_LOG_TEXT_BYTES, "a log line's bound is the engine's");

// The interface core, which does not depend on the engine, restates the
// engine's bound on the sub-pages listed; the two must agree (requirement
// 2.4).
const _: () = assert!(interface_core::controls::MAX_SUBPAGES == neoteletext_core::viewer::MAX_LISTED_SUBPAGES, "the engine's sub-page bound");
