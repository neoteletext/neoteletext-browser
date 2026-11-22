//! Requests to this computer and the local network, allowed as the owner
//! decided (neoteletext-common's DEC-7315, which names the browser whose
//! behaviour is followed, and the browser's TODO-7316, which lists that
//! behaviour as it was verified on 22 November 2026): each address is of
//! one of three address spaces, and a page loaded from the internet needs
//! the viewer's permission to ask this computer or the local network.
//! Until the browser can ask for it, needing it is being refused.
//!
//! The decision is made on the address a name resolved to, and again on
//! the address a connection reached, so a name that is given a public
//! address and then a local one gains nothing.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use neoteletext_transport::web_address;
use protocol::web::{AddressSpace, Reason};

/// "This network", 0.0.0.0/8 (RFC 1122 section 3.2.1.3): its first
/// address is this computer, the rest local.
const THIS_NETWORK_FIRST_OCTET: u8 = 0;
/// Carrier-grade NAT's shared addresses, 100.64.0.0/10 (RFC 6598), which
/// are a network's own as private addresses are.
const SHARED_FIRST_OCTET: u8 = 100;
const SHARED_SECOND_OCTET_MASK: u8 = 0b1100_0000;
const SHARED_SECOND_OCTET: u8 = 0b0100_0000;
/// IPv6's local ranges, each a prefix of an address's first 16 bits with
/// the mask of its length: unique local `fc00::/7` (RFC 4193), link-local
/// `fe80::/10` (RFC 4291 section 2.5.6) and the old site-local `fec0::/10`
/// (RFC 3879).
const V6_LOCAL_PREFIXES: [(u16, u16); 3] = [(0xFE00, 0xFC00), (0xFFC0, 0xFE80), (0xFFC0, 0xFEC0)];
/// IPv6's documentation ranges, as an address's first two groups:
/// `2001:db8::/32` (RFC 3849), and `3fff::/20` (RFC 9637), whose twenty
/// bits are the first group and the top four of the second.
const V6_DOCUMENTATION: [u16; 2] = [0x2001, 0x0DB8];
const V6_DOCUMENTATION_WIDE: u16 = 0x3FFF;
const V6_DOCUMENTATION_WIDE_SECOND_MASK: u16 = 0xF000;

fn v4_space(address: Ipv4Addr) -> AddressSpace {
    let [first, second, ..] = address.octets();
    if address.is_loopback() || address.is_unspecified() {
        AddressSpace::Loopback
    } else if first == THIS_NETWORK_FIRST_OCTET
        || address.is_private()
        || address.is_link_local()
        || (first == SHARED_FIRST_OCTET && second & SHARED_SECOND_OCTET_MASK == SHARED_SECOND_OCTET)
    {
        AddressSpace::Local
    } else {
        AddressSpace::Public
    }
}

fn v6_space(address: Ipv6Addr) -> AddressSpace {
    let [first, second, ..] = address.segments();
    if address.is_loopback() || address.is_unspecified() {
        AddressSpace::Loopback
    } else if V6_LOCAL_PREFIXES.iter().any(|&(mask, prefix)| first & mask == prefix)
        || [first, second] == V6_DOCUMENTATION
        || (first == V6_DOCUMENTATION_WIDE && second & V6_DOCUMENTATION_WIDE_SECOND_MASK == 0)
    {
        AddressSpace::Local
    } else {
        AddressSpace::Public
    }
}

/// The address space of `ip`. An IPv4 address written as IPv6 is the IPv4
/// address. Whatever else the engine's own rule keeps a channel from (its
/// REQ-SEC-006 and decision E.115: multicast, broadcast, the benchmarking
/// and protocol-assignment ranges, and addresses that embed a private
/// one) is local too: stricter than the behaviour followed, which leaves
/// those public, because each is a way to a network's own machines.
pub(crate) fn space_of(ip: IpAddr) -> AddressSpace {
    let ip = ip.to_canonical();
    let space = match ip {
        IpAddr::V4(address) => v4_space(address),
        IpAddr::V6(address) => v6_space(address),
    };
    if space == AddressSpace::Public && web_address(ip).is_blocked() { AddressSpace::Local } else { space }
}

/// The page asking: where it was loaded from, and whether securely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Asker {
    /// The address space the page was loaded from.
    pub(crate) space: AddressSpace,
    /// Whether the page is a secure context: loaded over `https`, or from
    /// this computer itself.
    pub(crate) secure: bool,
}

/// What a request to an address is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// It may be made.
    Allowed,
    /// It needs the viewer's permission to reach devices on the local
    /// network, which the browser cannot yet ask for: refused, and told
    /// apart so that the trusted core can ask.
    NeedsLocalNetwork,
    /// It needs the viewer's permission to reach this computer's own
    /// services: a permission of its own, refused and told apart
    /// likewise.
    NeedsThisDevice,
    /// Refused without asking: the page is from the internet and is not
    /// a secure context, which is never asked about.
    Refused,
}

impl Access {
    /// The failure the request ends with, if it may not be made.
    pub(crate) const fn failure(self) -> Option<Reason> {
        match self {
            Self::Allowed => None,
            Self::NeedsLocalNetwork => Some(Reason::LocalPermission),
            Self::NeedsThisDevice => Some(Reason::DevicePermission),
            Self::Refused => Some(Reason::LocalRefused),
        }
    }
}

/// The one decision: whether the page `asker` may ask the address
/// `target`. A page loaded from this computer or the local network asks
/// any address; a page from the internet asks a public address freely,
/// and for a local one or this computer needs the viewer's permission if
/// it is a secure context and is refused if it is not. An address in
/// `overrides` is taken as public whatever it is: those the viewer named
/// when the browser was started (`--allow-local`).
///
/// A request to the page's own origin is judged as any other, where the
/// behaviour followed exempts it: stricter, so that a name pointed at the
/// local network after its page was loaded is not that page's to ask.
pub(crate) fn local_access(asker: Asker, target: IpAddr, overrides: &[IpAddr]) -> Access {
    let target = target.to_canonical();
    if asker.space != AddressSpace::Public || overrides.iter().any(|named| named.to_canonical() == target) {
        return Access::Allowed;
    }
    match (space_of(target), asker.secure) {
        (AddressSpace::Public, _) => Access::Allowed,
        (_, false) => Access::Refused,
        (AddressSpace::Local, true) => Access::NeedsLocalNetwork,
        (AddressSpace::Loopback, true) => Access::NeedsThisDevice,
    }
}

/// Whether an address is known to be of the local network before its
/// name is resolved (the Local Network Access draft, sections 2.5 and
/// 3.2; TODO-7316 item 10): the script declared it so
/// (`targetAddressSpace: "local"`), its host is an address of the local
/// network written out, or its host's name ends in `.local`. Such a
/// request is not mixed content from a page loaded securely; the rule on
/// the local network still decides whether it is made.
pub(crate) fn known_local(url: &url::Url, declared: Option<AddressSpace>) -> bool {
    /// The top-level name of multicast DNS (RFC 6762 section 3).
    const LOCAL_NAME: &str = "local";
    declared == Some(AddressSpace::Local)
        || match url.host() {
            Some(url::Host::Ipv4(address)) => space_of(IpAddr::V4(address)) == AddressSpace::Local,
            Some(url::Host::Ipv6(address)) => space_of(IpAddr::V6(address)) == AddressSpace::Local,
            Some(url::Host::Domain(name)) => {
                let name = name.strip_suffix('.').unwrap_or(name);
                name.rsplit('.').next().is_some_and(|last| last.eq_ignore_ascii_case(LOCAL_NAME)) && name.contains('.')
            }
            None => false,
        }
}

/// Whether an answer's address is where the script said the address
/// would be (`targetAddressSpace`; TODO-7316 item 10, "wrong
/// declaration"). `peer` is the address connected to, none through a
/// proxy, where the server's address is not known and is taken as
/// public, as the behaviour followed takes it.
pub(crate) fn is_as_declared(peer: Option<IpAddr>, declared: Option<AddressSpace>) -> bool {
    declared.is_none_or(|declared| peer.map_or(AddressSpace::Public, space_of) == declared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    /// TODO-7316 item 10: what is known to be local before a name is
    /// resolved, and whether an answer came from where the script said.
    #[test]
    fn a_declared_or_written_local_address_is_known_and_a_wrong_declaration_is_seen() {
        let host = |text: &str| url::Url::parse(text).unwrap();
        for (address, known) in [
            ("http://192.168.1.10/", true),
            ("http://10.0.0.1:8080/", true),
            ("http://[fe80::1]/", true),
            ("http://printer.local/", true),
            ("http://printer.LOCAL./", true),
            ("http://local/", false),
            ("http://notlocal/", false),
            ("http://printer.localhost.example/", false),
            ("http://127.0.0.1/", false),
            ("http://93.184.216.34/", false),
            ("http://device.example/", false),
        ] {
            assert_eq!(known_local(&host(address), None), known, "{address}");
        }
        assert!(known_local(&host("http://device.example/"), Some(AddressSpace::Local)));
        assert!(!known_local(&host("http://device.example/"), Some(AddressSpace::Loopback)), "this computer by name is asked over https or as localhost");
        assert!(!known_local(&host("http://device.example/"), Some(AddressSpace::Public)));
        for (peer, declared, holds) in [
            (Some("192.168.1.10"), None, true),
            (Some("192.168.1.10"), Some(AddressSpace::Local), true),
            (Some("192.168.1.10"), Some(AddressSpace::Public), false),
            (Some("127.0.0.1"), Some(AddressSpace::Local), false),
            (Some("127.0.0.1"), Some(AddressSpace::Loopback), true),
            (Some("93.184.216.34"), Some(AddressSpace::Local), false),
            (None, Some(AddressSpace::Public), true),
            (None, Some(AddressSpace::Local), false),
            (None, None, true),
        ] {
            assert_eq!(is_as_declared(peer.map(ip), declared), holds, "{peer:?} {declared:?}");
        }
    }

    /// TODO-7316 item 5: the ranges of each space, as verified, and the
    /// engine's wider list kept where it is stricter.
    #[test]
    fn each_address_is_of_one_space() {
        for address in ["127.0.0.1", "127.255.255.254", "::1", "0.0.0.0", "::", "::ffff:127.0.0.1"] {
            assert_eq!(space_of(ip(address)), AddressSpace::Loopback, "{address}");
        }
        for address in [
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.1",
            "0.255.255.255",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "feff::1",
            "2001:db8::1",
            "3fff::1",
            "3fff:fff::1",
            "::ffff:192.168.1.1",
        ] {
            assert_eq!(space_of(ip(address)), AddressSpace::Local, "{address}");
        }
        for address in [
            "8.8.8.8",
            "1.0.0.1",
            "172.32.0.1",
            "100.128.0.1",
            "100.63.255.255",
            "192.167.1.1",
            "2606:4700::1111",
            "2001:db9::1",
            "3ffe::1",
            "3fff:1000::1",
            "4000::1",
            "fbff::1",
            "::ffff:8.8.8.8",
        ] {
            assert_eq!(space_of(ip(address)), AddressSpace::Public, "{address}");
        }
        // The engine's wider list: nothing it blocks is public here.
        for address in ["224.0.0.1", "255.255.255.255", "192.0.0.1", "198.18.0.1", "ff02::1"] {
            let address = ip(address);
            assert!(web_address(address).is_blocked(), "{address} is on the engine's list");
            assert_eq!(space_of(address), AddressSpace::Local, "{address}");
        }
    }

    /// TODO-7316 item 7: a page from this computer or the local network
    /// asks anything; a page from the internet asks this computer or the
    /// local network only with the viewer's permission, one permission
    /// for each, and not at all unless it is a secure context.
    #[test]
    fn only_a_public_page_needs_permission_and_only_a_secure_one_may_ask() {
        use Access::{Allowed, NeedsLocalNetwork, NeedsThisDevice, Refused};
        use AddressSpace::{Local, Loopback, Public};
        let (loopback, local, public) = (ip("127.0.0.1"), ip("192.168.1.10"), ip("8.8.8.8"));
        for (space, secure, target, access) in [
            (Public, true, public, Allowed),
            (Public, true, local, NeedsLocalNetwork),
            (Public, true, loopback, NeedsThisDevice),
            (Public, false, public, Allowed),
            (Public, false, local, Refused),
            (Public, false, loopback, Refused),
            (Local, false, public, Allowed),
            (Local, false, local, Allowed),
            (Local, false, loopback, Allowed),
            (Loopback, true, public, Allowed),
            (Loopback, true, local, Allowed),
            (Loopback, true, loopback, Allowed),
        ] {
            assert_eq!(local_access(Asker { space, secure }, target, &[]), access, "{space:?}, secure {secure}, to {target}");
        }
        // An address the viewer named is taken as public, in any
        // spelling, and nothing else is.
        let page = Asker { space: Public, secure: false };
        assert_eq!(local_access(page, ip("::ffff:192.168.1.10"), &[local]), Allowed);
        assert_eq!(local_access(page, local, &[ip("::ffff:192.168.1.10")]), Allowed);
        assert_eq!(local_access(page, ip("192.168.1.11"), &[local]), Refused);
        assert_eq!(local_access(page, loopback, &[local]), Refused);
        assert_eq!(
            (Allowed.failure(), NeedsLocalNetwork.failure(), NeedsThisDevice.failure(), Refused.failure()),
            (None, Some(Reason::LocalPermission), Some(Reason::DevicePermission), Some(Reason::LocalRefused))
        );
    }
}
