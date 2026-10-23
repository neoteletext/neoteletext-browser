//! The release's identity, built in (`installation-and-updates.md`
//! section 14): what the updater trusts (the update host, its port, The
//! Update Framework's first root) and what the launcher and the installer
//! accept (each platform's pinned signer). `build.rs` makes the constants
//! from the production identity in `production/`, or from a test
//! identity a test build names; nothing is read at run time.

#[allow(missing_docs, reason = "each generated constant carries its own documentation")]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/identity.rs"));
}

pub use generated::{HOST, MACOS_REQUIREMENT, NAME, PORT, RELEASE_KEY, TLS_ROOT, TUF_ROOT, WINDOWS_SIGNER};

/// The signers the launcher and the installer pin.
#[must_use]
pub fn keys() -> update::accept::Keys {
    update::accept::Keys { release: RELEASE_KEY, macos_requirement: MACOS_REQUIREMENT.map(str::to_owned), windows_signer: WINDOWS_SIGNER }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production identity names the update host of decision 25 on
    /// the port of TLS; its keys come at the key ceremony.
    #[test]
    fn the_production_identity_names_the_update_host() {
        if NAME == "production" {
            assert_eq!((HOST, PORT), ("update.neoteletext.com", 443));
        }
    }
}
