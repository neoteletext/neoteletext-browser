//! The channel as the unit of isolation (the browser's REQ-712,
//! neoteletext-common's DEC-7264): which channel and jar of windows a
//! process serves, and how an address stands to that channel. The policy
//! is [`ChannelPolicy`] and nothing else: every same-origin test of the
//! Fetch algorithm asks it, and it is built only from what the trusted
//! core started the process with, never from anything a script sent
//! (neoteletext-common's `docs/script-api.md` section 2, rules P1 to P3).
//!
//! This departs from the Fetch standard and from RFC 6265bis, which
//! isolate by origin and by site: many channels share one origin
//! (`https://host/home/`, `https://host/demosite/`), and each is its own
//! security principal.

use url::Url;

/// A channel's identity: its origin and its folder, as the trusted core
/// gave it. Never anything a script said.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Channel {
    /// The folder's address, or none for a page of no channel.
    folder: Option<Url>,
}

impl Channel {
    /// The identity written as `text`: an `http` or `https` address of a
    /// folder, ending in `/`, with no user name, query or fragment. Any
    /// other text is no channel, which nothing is inside.
    fn read(text: &str) -> Self {
        let folder = Url::parse(text).ok().filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.path().ends_with('/')
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
        Self { folder }
    }

    /// The folder's address.
    const fn folder(&self) -> Option<&Url> {
        self.folder.as_ref()
    }
}

/// What a server that decodes a path before it resolves it could follow
/// out of a folder, beyond the encoded dot, slash and backslash the
/// engine's rule refuses: a path parameter, written or encoded, which
/// some servers drop before resolving `..;/`; an encoded percent sign,
/// which a second decoding turns into any of the others; and an encoded
/// NUL, at which some servers cut a path. Compared without regard to
/// case.
const PATH_PARAMETER: u8 = b';';
const ENCODED_ESCAPES: [&[u8; 3]; 3] = [b"%25", b"%00", b"%3B"];

/// How an address stands to a channel (rule P1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relation {
    /// Inside the channel: the Fetch standard's "same origin".
    InChannel,
    /// Of the channel's origin and outside its folder: another origin to
    /// the Fetch standard's rules (rule P2).
    SameOriginOutside,
    /// Of another origin.
    CrossOrigin,
}

/// Whether `url` is inside `channel`: of its origin, with a path under its
/// folder that no server could read as leaving it.
///
/// The engine's rule decides (`neoteletext_web::channel::is_inside`, its
/// REQ-3518, finding F-45): the same origin, the folder's path as a
/// prefix, and no `%2E`, `%2F` or `%5C` after it. To that are added a `;`,
/// `%3B`, `%25` and `%00` after the folder (finding S-02, TODO-7267). The address
/// judged is the address sent, byte for byte: nothing is rewritten.
///
/// A channel inside another's folder (`/a/b/` under `/a/`) cannot be told
/// from the outer channel's own pages here: the process is told only its
/// own channel. The outer channel's requests into it are same-channel by
/// this test; they still carry only the outer channel's jar.
fn same_channel(channel: &Channel, url: &Url) -> bool {
    let Some(folder) = channel.folder() else { return false };
    let (Ok(engine_folder), Ok(engine_url)) = (neoteletext_web::url::Url::parse(folder.as_str()), neoteletext_web::url::Url::parse(url.as_str())) else {
        return false;
    };
    let Some(rest) = url.path().strip_prefix(folder.path()) else { return false };
    let escapes = rest.as_bytes().windows(3).any(|window| ENCODED_ESCAPES.iter().any(|escape| window.eq_ignore_ascii_case(*escape)));
    folder.origin() == url.origin()
        && url.origin().is_tuple()
        && neoteletext_web::channel::is_inside(&engine_folder, &engine_url)
        && !rest.as_bytes().contains(&PATH_PARAMETER)
        && !escapes
}

/// The policy of the one partition a web client process serves: one
/// channel identity in one jar of windows (DEC-7264 item 2, the browser's
/// DEC-7268). The process is started for its partition and knows a start
/// meant for another by its token (`protocol::web::PartitionToken`); its
/// cookies, connections and remembered preflights are that partition's
/// alone, so a fault in it reaches no other channel's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChannelPolicy {
    /// The channel.
    channel: Channel,
}

impl ChannelPolicy {
    /// The policy of the channel whose identity is written `channel`, or
    /// of a page opened from disk when it is
    /// [`protocol::web::FILE_PAGE_IDENTITY`]; `None` when the text is
    /// neither.
    pub(crate) fn new(channel: &str) -> Option<Self> {
        if channel == protocol::web::FILE_PAGE_IDENTITY {
            return Some(Self { channel: Channel { folder: None } });
        }
        let channel = Channel::read(channel);
        channel.folder().is_some().then_some(Self { channel })
    }

    /// The channel's folder, whose origin is its scripts' origin; none
    /// for a page opened from disk, which has no channel.
    pub(crate) const fn folder(&self) -> Option<&Url> {
        self.channel.folder()
    }

    /// Whether the partition is a page opened from disk (DEC-7315 version
    /// 2): its origin is written `null`, no address is its own, and its
    /// jar is never kept.
    pub(crate) const fn is_file_page(&self) -> bool {
        self.channel.folder().is_none()
    }

    /// The origin of the partition's scripts as a request writes it.
    pub(crate) fn origin(&self) -> String {
        self.folder().map_or_else(|| "null".to_owned(), |folder| folder.origin().ascii_serialization())
    }

    /// Whether the partition's pages are a secure context (the W3C
    /// Secure Contexts section 3.1): loaded over `https`, from this
    /// computer by name or address, or from disk.
    pub(crate) fn is_secure_context(&self) -> bool {
        self.folder().is_none_or(|folder| folder.scheme() == "https" || crate::web::fetch::is_loopback(folder))
    }

    /// How `url` stands to the channel (rule P1).
    pub(crate) fn relation(&self, url: &Url) -> Relation {
        if same_channel(&self.channel, url) {
            Relation::InChannel
        } else if self.channel.folder().is_some_and(|folder| folder.origin() == url.origin()) {
            Relation::SameOriginOutside
        } else {
            Relation::CrossOrigin
        }
    }

    /// Whether `url` is same-origin to a request of the channel: what
    /// stands wherever the Fetch standard asks whether a request is
    /// same-origin with its client, so an address of the channel's own
    /// origin outside its folder is cross-origin (rule P2, DEC-7264 item
    /// 3).
    pub(crate) fn is_same_origin(&self, url: &Url) -> bool {
        self.relation(url) == Relation::InChannel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(channel: &str, url: &str) -> bool {
        same_channel(&Channel::read(channel), &Url::parse(url).unwrap())
    }

    /// DEC-7264 item 3: an address of the channel's origin outside its
    /// folder is not the channel's, and neither is anything of another
    /// origin.
    #[test]
    fn only_addresses_under_the_folder_are_the_channels() {
        let channel = "https://host.example/home/";
        for url in ["https://host.example/home/", "https://host.example/home/100.tti", "https://host.example/home/a/b?x=1#f", "https://host.example:443/home/x"]
        {
            assert!(inside(channel, url), "{url}");
        }
        for url in [
            "https://host.example/",
            "https://host.example/home",
            "https://host.example/homepage/",
            "https://host.example/demosite/100.tti",
            "https://host.example/home/../demosite/secret",
            "https://host.example/home/%2e%2e/demosite/secret",
            // Finding S-02 (TODO-7267): what a server that decodes a path
            // before resolving it follows out of the folder.
            "https://host.example/home/..%2fdemosite/secret",
            "https://host.example/home/%2e%2e%2fdemosite/secret",
            "https://host.example/home/..;/demosite/x",
            "https://host.example/home/%2E%2E%2Fdemosite/secret",
            "https://host.example/home/..%5cdemosite/secret",
            "https://host.example/home/%252e%252e%252fdemosite/secret",
            "https://host.example/home/a%00b",
            "https://host.example/home/a;b=c/x",
            "https://host.example/home/..%3b/demosite/x",
            "https://host.example/Home/x",
            "http://host.example/home/x",
            "https://host.example:8443/home/x",
            "https://other.example/home/x",
            "https://host.example.evil.example/home/x",
        ] {
            assert!(!inside(channel, url), "{url}");
        }
        // TODO-7265 (d): the inner channel's requests to the outer are not
        // its own.
        assert!(!inside("https://host.example/a/b/", "https://host.example/a/secret"));
        // A channel at the origin's root holds the whole origin.
        assert!(inside("https://host.example/", "https://host.example/anything"));
    }

    /// Text that is no folder's address is no channel, and nothing is
    /// inside it.
    #[test]
    fn what_is_no_identity_holds_nothing() {
        for channel in [
            "",
            "https://host.example/home",
            "ftp://host.example/home/",
            "https://user@host.example/home/",
            "https://host.example/home/?q",
            "data:text/plain,/",
            "home/",
        ] {
            assert!(Channel::read(channel).folder().is_none(), "{channel}");
            assert!(!inside(channel, "https://host.example/home/x"), "{channel}");
        }
    }

    /// What is no channel identity has no policy, and so no web client.
    #[test]
    fn only_a_channel_identity_has_a_policy() {
        assert!(ChannelPolicy::new("https://host.example/home/").is_some());
        assert!(ChannelPolicy::new("").is_none() && ChannelPolicy::new("https://host.example/home").is_none() && ChannelPolicy::new("file:///home/").is_none());
    }

    /// DEC-7315 version 2 (TODO-7395 step 1): a page opened from disk has
    /// a policy by its one identity and by no path; its origin is written
    /// `null`, it is a secure context, and no address is its own, another
    /// file's least of all.
    #[test]
    fn a_page_from_disk_has_the_origin_null_and_owns_no_address() {
        let policy = ChannelPolicy::new(protocol::web::FILE_PAGE_IDENTITY).unwrap();
        assert!(policy.is_file_page() && policy.is_secure_context() && policy.folder().is_none());
        assert_eq!(policy.origin(), "null");
        for url in ["file:///home/viewer/page.tti", "http://localhost/", "https://host.example/home/", "data:text/plain,x"] {
            assert_eq!(policy.relation(&Url::parse(url).unwrap()), Relation::CrossOrigin, "{url}");
        }
        let channel = ChannelPolicy::new("https://localhost/a/").unwrap();
        assert!(!channel.is_file_page() && channel.is_secure_context());
        assert_eq!(channel.origin(), "https://localhost");
        assert!(ChannelPolicy::new("http://localhost:8080/a/").unwrap().is_secure_context());
        assert!(!ChannelPolicy::new("http://host.example/a/").unwrap().is_secure_context());
    }

    /// Rules P1 and P2: an address is inside the channel, of its origin
    /// and outside it, or of another origin; only the first is
    /// same-origin.
    #[test]
    fn an_address_stands_to_the_channel_in_one_of_three_ways() {
        let policy = ChannelPolicy::new("https://host.example/home/").unwrap();
        for (url, relation) in [
            ("https://host.example/home/100.tti", Relation::InChannel),
            ("https://host.example/demosite/100.tti", Relation::SameOriginOutside),
            ("https://host.example/home/..%2fdemosite/secret", Relation::SameOriginOutside),
            ("https://host.example/home/..;/demosite/x", Relation::SameOriginOutside),
            ("https://other.example/home/100.tti", Relation::CrossOrigin),
            ("http://host.example/home/100.tti", Relation::CrossOrigin),
            ("data:text/plain,x", Relation::CrossOrigin),
        ] {
            let url = Url::parse(url).unwrap();
            assert_eq!(policy.relation(&url), relation, "{url}");
            assert_eq!(policy.is_same_origin(&url), relation == Relation::InChannel, "{url}");
        }
    }
}
