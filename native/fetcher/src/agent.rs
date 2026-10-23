//! What the fetcher gives the engine's agent of its own: randomness, the
//! product it names itself as, and the links of the channel's descriptor
//! passed on.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

#[cfg(feature = "links")]
use neoteletext_web::agent::UserAgent;
#[cfg(feature = "links")]
use protocol::fetcher::FromFetcher;

#[cfg(feature = "links")]
use crate::process::{COOKIE_BYTES, COOKIE_SLOTS};

/// Randomness for the agent (WebSocket masks), from the operating system's
/// per-process random keys, which `RandomState` draws.
pub(crate) struct Random {
    pub(crate) state: RandomState,
    pub(crate) counter: u64,
}

impl neoteletext_web::port::RandomSource for Random {
    fn fill(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            self.counter = self.counter.wrapping_add(1);
            let mut hasher = self.state.build_hasher();
            hasher.write_u64(self.counter);
            let value = hasher.finish().to_le_bytes();
            for (byte, &random) in chunk.iter_mut().zip(&value) {
                *byte = random;
            }
        }
    }
}

/// The browser as it names itself to channels, as browsers should have
/// (requirement 5.14, the engine's REQ-WEB-031): `NeoTeletext/0.1
/// (macOS)`, its own product, so the engine is not named again.
pub(crate) fn product() -> Option<neoteletext_web::user_agent::Product> {
    neoteletext_web::user_agent::os_family().map(|family| neoteletext_web::user_agent::Product {
        name: "NeoTeletext",
        version: env!("CARGO_PKG_VERSION"),
        family,
        engine_own: true,
    })
}

/// The aliases and named services of the open channel's descriptor, as
/// written: untrusted, for the trusted core to parse. Aliases of numbers
/// that are not display pages are left out.
#[cfg(feature = "links")]
pub(crate) fn descriptor_links(agent: &UserAgent<'_, Random, COOKIE_SLOTS, COOKIE_BYTES>) -> Option<FromFetcher> {
    use protocol::links::{Alias, MAX_ALIASES, MAX_SERVICES, Service};
    let descriptor = agent.descriptor()?;
    let aliases: Vec<Alias> = descriptor
        .aliases()
        .filter(|alias| interface_core::channel::page_index(alias.page.mpp()).is_some())
        .map(|alias| Alias { page: alias.page.mpp(), target: descriptor.alias_target(alias).to_owned() })
        .take(MAX_ALIASES)
        .collect();
    let services: Vec<Service> = descriptor
        .services()
        .map(|service| Service { name: service.short_name.as_str().to_owned(), address: descriptor.service_address(service).to_owned() })
        .take(MAX_SERVICES)
        .collect();
    (!aliases.is_empty() || !services.is_empty()).then_some(FromFetcher::Links { aliases, services })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requirement 5.14, the engine's REQ-WEB-031: the browser names itself
    /// `NeoTeletext/0.1 (<family>)`.
    #[test]
    fn the_user_agent_names_the_browser() {
        let product = product().expect("a desktop names its family");
        let named = neoteletext_web::user_agent::UserAgentString::new(&product).expect("a valid user agent");
        let family = neoteletext_web::user_agent::os_family().expect("a desktop names its family");
        assert_eq!(named.as_str(), format!("NeoTeletext/0.1 ({family})"));
    }
}
