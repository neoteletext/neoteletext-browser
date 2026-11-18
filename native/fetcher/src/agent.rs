//! What the fetcher gives the engine's agent of its own: randomness, the
//! product it names itself as, and the links of the channel's descriptor
//! passed on.

#[cfg(feature = "links")]
use neoteletext_web::agent::UserAgent;
#[cfg(feature = "links")]
use protocol::fetcher::FromFetcher;

/// Randomness for the agent: each WebSocket frame's masking key, drawn
/// afresh from the operating system's cryptographic random source, as
/// RFC 6455 section 5.3 requires ("derived from a strong source of
/// entropy"), so that no page or program can predict the masked bytes
/// (TODO-6472). It holds no state of its own: nothing in the process
/// predicts the next mask.
pub(crate) struct Random(ring::rand::SystemRandom);

impl Random {
    /// The system's random source.
    pub(crate) fn new() -> Self {
        Self(ring::rand::SystemRandom::new())
    }
}

impl neoteletext_web::port::RandomSource for Random {
    fn fill(&mut self, bytes: &mut [u8]) {
        use ring::rand::SecureRandom as _;
        // A fetcher that cannot draw random bytes cannot mask safely: it
        // ends, which the trusted core reports, rather than send a frame
        // with a mask anyone could predict.
        if self.0.fill(bytes).is_err() {
            eprintln!("neoteletext-fetcher: the system's random source failed; no frame is sent with a predictable mask");
            std::process::abort();
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
pub(crate) fn descriptor_links(agent: &UserAgent<'_, Random>) -> Option<FromFetcher> {
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

    /// TODO-6472, RFC 6455 section 5.3: the masks come from the system's
    /// cryptographic source and from nothing the process keeps. Before,
    /// they were a hash of a counter under the process's `RandomState`
    /// keys, so the source held a key and a counter (24 bytes) from which
    /// every later mask followed; it fails here, both by its type and by
    /// its size.
    #[test]
    fn masks_come_from_the_systems_random_source_alone() {
        use neoteletext_web::port::RandomSource as _;
        let Random(system) = Random::new();
        let _: ring::rand::SystemRandom = system;
        assert_eq!(size_of::<Random>(), 0, "no key or counter kept in the process");
        let mut random = Random::new();
        let masks: std::collections::BTreeSet<[u8; 4]> = (0..64)
            .map(|_| {
                let mut mask = [0_u8; 4];
                random.fill(&mut mask);
                mask
            })
            .collect();
        // 64 draws of 32 bits collide with a chance below one in a million.
        assert_eq!(masks.len(), 64, "fresh masks");
    }

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
