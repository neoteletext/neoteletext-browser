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

/// The policy of the one partition a web client process serves: a jar of
/// windows (the ordinary windows' or one private window's) and a channel
/// identity (DEC-7264 item 2, the browser's DEC-7268). The process's
/// cookies, connections and remembered preflights are that partition's
/// alone, so a fault in it reaches no other channel's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChannelPolicy {
    /// The jar of windows: 0 the ordinary windows', another a private
    /// window's.
    windows: u32,
    /// The channel.
    channel: Channel,
}

impl ChannelPolicy {
    /// The policy of the channel whose identity is written `channel`, in
    /// the jar of windows `windows`; `None` when the text is no channel
    /// identity.
    pub(crate) fn new(windows: u32, channel: &str) -> Option<Self> {
        let channel = Channel::read(channel);
        channel.folder().is_some().then_some(Self { windows, channel })
    }

    /// The channel's folder, whose origin is its scripts' origin.
    pub(crate) const fn folder(&self) -> Option<&Url> {
        self.channel.folder()
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

    /// Whether a request naming the jar of windows `windows` and the
    /// channel identity `channel` is this partition's. One that is not
    /// ends the process.
    pub(crate) fn holds(&self, windows: u32, channel: &str) -> bool {
        self.windows == windows && self.channel == Channel::read(channel)
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

    /// DEC-7268: a process serves the one partition it was started for.
    #[test]
    fn a_policy_holds_only_its_own_partitions_requests() {
        let policy = ChannelPolicy::new(0, "https://host.example/home/").unwrap();
        assert!(policy.holds(0, "https://host.example/home/"));
        assert!(policy.holds(0, "https://host.example:443/home/"), "the same identity, written with its default port");
        assert!(!policy.holds(1, "https://host.example/home/"), "another jar of windows");
        assert!(!policy.holds(0, "https://host.example/demosite/"));
        assert!(!policy.holds(0, "https://host.example/home/inner/"));
        assert!(!policy.holds(0, ""));
        assert!(ChannelPolicy::new(0, "").is_none() && ChannelPolicy::new(0, "https://host.example/home").is_none(), "no channel is no partition");
    }

    /// Rules P1 and P2: an address is inside the channel, of its origin
    /// and outside it, or of another origin; only the first is
    /// same-origin.
    #[test]
    fn an_address_stands_to_the_channel_in_one_of_three_ways() {
        let policy = ChannelPolicy::new(0, "https://host.example/home/").unwrap();
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
