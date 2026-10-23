//! The release tool (`installation-and-updates.md` section 10), one
//! command a step, run by the owner as the gates are:
//!
//! ```text
//! neoteletext-release layout <platform> <version> <programs> <out>
//! neoteletext-release manifest <folder> <platform> <version>
//! neoteletext-release sign-linux <folder> <version> <key>
//! neoteletext-release sign-file <file> <key>
//! neoteletext-release check <folder> <platform> [--last-launcher <file>] [--launcher-release]
//! neoteletext-release targets <folder> <platform> <out>
//! neoteletext-release root <out> <version> --root <key>... --targets <key> --snapshot <key> --timestamp <key> [--old-root <key>]...
//! neoteletext-release publish <repository> <root.json> <targets> --targets <key> --snapshot <key> --timestamp <key>
//! neoteletext-release timestamp <repository> <root.json> --timestamp <key>
//! neoteletext-release test-identity <folder> [--host H] [--port P] [--macos-requirement R] [--windows-signer K]
//! neoteletext-release key <out.pk8>
//! neoteletext-release public <key> | pem <key>
//! neoteletext-release sign-stdin <key.pk8>
//! neoteletext-release signer-key <program.exe>        (Windows)
//! ```
//!
//! A key is `file:<path>` (PKCS #8, DER) or `command:<public>:<program>`
//! (`keys`), so the same steps run with test keys now and the hardware
//! keys of the key ceremony later.

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

mod command;
mod keys;
mod package;
mod repo;
mod test_identity;

use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match command::run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-release: {error}");
            ExitCode::FAILURE
        }
    }
}
