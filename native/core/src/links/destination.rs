//! Where a link leads (`hyperlinks.md` sections 7.1 and 11): a target
//! classified against the channel it is on and the channel's descriptor,
//! before anything is fetched.

use protocol::clean;
use protocol::links::{LinkKind, MAX_DESTINATION};

use crate::address::Address;

use super::target::{Target, page};

/// The page a channel opens at when a link names its folder and no page:
/// the teletext initial page (the engine's REQ-WEB-018 default).
pub(super) const INITIAL_PAGE: u16 = 0x100;
/// The files of a channel that are pages (the engine's REQ-WEB-019).
const PAGE_FILES: [&str; 2] = [".tti", ".t42"];

/// Where a link leads, decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A page of this channel: followed at once.
    Page(u16),
    /// Another channel, at the page its address names, if any: after the
    /// site-change screen.
    Channel(Address),
    /// A web page: after the web-browser prompt, in the system's web browser.
    Web(String),
}

impl Destination {
    /// The kind the interface draws.
    #[must_use]
    pub fn kind(&self) -> LinkKind {
        match self {
            Self::Page(_) => LinkKind::Page,
            Self::Channel(_) => LinkKind::Channel,
            Self::Web(_) => LinkKind::Web,
        }
    }

    /// Where it is, as the strip's questions name it, cleaned and cut to
    /// the strip's limit (requirement 11.5).
    #[must_use]
    pub fn name(&self) -> String {
        let text = match self {
            Self::Page(page) => format!("Page {page:03X}"),
            Self::Channel(address) => match address.page {
                Some(page) => format!("Page {page:03X} of {}", address.host),
                None => address.channel_text().to_owned(),
            },
            Self::Web(address) => address.clone(),
        };
        clean(&text, MAX_DESTINATION)
    }

    /// What the trusted strip says of it while the pointer or the focus is
    /// on it: where it is, and whether it leaves.
    #[must_use]
    pub fn describe(&self) -> String {
        let text = match self {
            Self::Page(_) => self.name(),
            Self::Channel(_) => format!("{} (leaves this channel)", self.name()),
            Self::Web(_) => format!("{} (opens in your web browser)", self.name()),
        };
        clean(&text, MAX_DESTINATION)
    }
}

/// The channel a link is on, as the trusted core knows it.
pub struct Channel<'a> {
    /// The channel's number.
    pub id: u32,
    /// Its address: a web channel's folder, or a file.
    pub address: &'a Address,
    /// Whether it has a page: listed in its contents, or arrived.
    pub exists: &'a dyn Fn(u16) -> bool,
}

impl Channel<'_> {
    /// The page of this channel an address names: its own folder with a
    /// page, or one of its page files (`hyperlinks.md` section 7.1).
    fn own_page(&self, address: &Address) -> Option<u16> {
        if self.address.file.is_some() {
            return None;
        }
        let folder = self.address.channel_text();
        let other = address.channel_text();
        if other == folder {
            return Some(address.page.unwrap_or(INITIAL_PAGE));
        }
        let file = other.strip_prefix(folder)?;
        let number = PAGE_FILES.iter().find_map(|extension| file.strip_suffix(extension))?;
        // Page files are named in decimal digits: `302.tti` is page 302.
        page(number).filter(|_| number.bytes().all(|byte| byte.is_ascii_digit()))
    }
}

/// The aliases and named services of a channel's descriptor, parsed.
#[derive(Clone, Debug, Default)]
pub(super) struct Descriptor {
    pub(super) aliases: Vec<(u16, Target)>,
    pub(super) services: Vec<(String, Address)>,
    /// Every service in the descriptor's own order, as a Fastext link names
    /// one by its place (the engine's `LinkTarget::service`); one whose
    /// address is refused leads nowhere.
    pub(super) by_place: Vec<Option<Address>>,
}

impl Descriptor {
    /// Where `target` leads from `channel`. A target that names an unknown
    /// service leads nowhere (TTI Link Extension rule E.2.2); a named
    /// service is another channel, at its own address
    /// (`hyperlinks-security.md` finding 1).
    pub(super) fn resolve(&self, target: &Target, channel: &Channel<'_>) -> Option<Destination> {
        match target {
            Target::Page(page) => Some(Destination::Page(*page)),
            Target::Service { name, page } => {
                let (_, address) = self.services.iter().find(|(known, _)| known == name)?;
                Some(Destination::Channel(address.with_page(*page)))
            }
            Target::Channel(address) => Some(channel.own_page(address).map_or_else(|| Destination::Channel(address.clone()), Destination::Page)),
            Target::Web(address) => Some(Destination::Web(address.clone())),
        }
    }
}
