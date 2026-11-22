//! Requests to this computer and the local network, allowed as the owner
//! decided (neoteletext-common's DEC-7315, which names the browser whose
//! behaviour is followed): each address is of one of
//! three address spaces, and a request from a page of one space to an
//! address of a less public one needs the viewer's permission. Until the
//! browser can ask for it, needing it is being refused.
//!
//! The spaces and their order are those of the Local Network Access
//! draft the decision points at (loopback, then local, then public); the
//! ranges of each were written from the draft as remembered, on 22
//! November 2026, and wait to be checked against the research the
//! decision asks for. The decision is made on the address a name resolved
//! to, and again on the address a connection reached, so a name that is
//! given a public address and then a local one gains nothing.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use neoteletext_transport::web_address;
use protocol::web::AddressSpace;

/// Carrier-grade NAT's shared addresses, 100.64.0.0/10 (RFC 6598), which
/// are a network's own as private addresses are.
const SHARED_FIRST_OCTET: u8 = 100;
const SHARED_SECOND_OCTET_MASK: u8 = 0b1100_0000;
const SHARED_SECOND_OCTET: u8 = 0b0100_0000;
/// IPv6's unique local addresses, `fc00::/7` (RFC 4193), and its link-local
/// ones, `fe80::/10` (RFC 4291 section 2.5.6).
const UNIQUE_LOCAL_MASK: u16 = 0xFE00;
const UNIQUE_LOCAL: u16 = 0xFC00;
const LINK_LOCAL_MASK: u16 = 0xFFC0;
const LINK_LOCAL: u16 = 0xFE80;

fn v4_space(address: Ipv4Addr) -> AddressSpace {
    let [first, second, ..] = address.octets();
    if address.is_loopback() || address.is_unspecified() {
        AddressSpace::Loopback
    } else if address.is_private() || address.is_link_local() || (first == SHARED_FIRST_OCTET && second & SHARED_SECOND_OCTET_MASK == SHARED_SECOND_OCTET) {
        AddressSpace::Local
    } else {
        AddressSpace::Public
    }
}

fn v6_space(address: Ipv6Addr) -> AddressSpace {
    let first = address.segments().first().copied().unwrap_or(0);
    if address.is_loopback() || address.is_unspecified() {
        AddressSpace::Loopback
    } else if first & UNIQUE_LOCAL_MASK == UNIQUE_LOCAL || first & LINK_LOCAL_MASK == LINK_LOCAL {
        AddressSpace::Local
    } else {
        AddressSpace::Public
    }
}

/// The address space of `ip`. An IPv4 address written as IPv6 is the IPv4
/// address; and whatever else the engine's own rule keeps a channel from
/// (its REQ-SEC-006: the ranges reserved for a network's own use) is
/// local, so nothing that rule held back is public here.
pub(crate) fn space_of(ip: IpAddr) -> AddressSpace {
    let ip = ip.to_canonical();
    let space = match ip {
        IpAddr::V4(address) => v4_space(address),
        IpAddr::V6(address) => v6_space(address),
    };
    if space == AddressSpace::Public && web_address(ip).is_blocked() { AddressSpace::Local } else { space }
}

/// How private a space is: a request to a space more private than its
/// page's is the one that needs permission.
const fn privacy(space: AddressSpace) -> u8 {
    match space {
        AddressSpace::Public => 0,
        AddressSpace::Local => 1,
        AddressSpace::Loopback => 2,
    }
}

/// What a request to an address is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// It may be made.
    Allowed,
    /// It needs the viewer's permission, which the browser cannot yet ask
    /// for: refused, and told apart so that the trusted core can ask.
    NeedsPermission,
}

/// The one decision: whether a page loaded from the address space `page`
/// may ask the address `target`. A page may ask its own space and any
/// more public one; an address of a more private space needs the viewer's
/// permission, which `permitted` holds for the addresses the viewer has
/// already allowed (today, those the browser was started with).
pub(crate) fn local_access(page: AddressSpace, target: IpAddr, permitted: &[IpAddr]) -> Access {
    let target = target.to_canonical();
    if privacy(space_of(target)) <= privacy(page) || permitted.iter().any(|allowed| allowed.to_canonical() == target) {
        Access::Allowed
    } else {
        Access::NeedsPermission
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

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
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert_eq!(space_of(ip(address)), AddressSpace::Local, "{address}");
        }
        for address in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "100.63.255.255", "192.167.1.1", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert_eq!(space_of(ip(address)), AddressSpace::Public, "{address}");
        }
        // Whatever the engine's rule blocks is never public.
        for address in ["224.0.0.1", "255.255.255.255", "ff02::1", "192.0.2.1", "198.18.0.1"] {
            let address = ip(address);
            assert!(!web_address(address).is_blocked() || space_of(address) != AddressSpace::Public, "{address}");
        }
    }

    /// DEC-7315: a page asks its own space and any more public one freely;
    /// a more private one needs the viewer's permission.
    #[test]
    fn a_more_private_address_needs_the_viewers_permission() {
        use Access::{Allowed, NeedsPermission};
        use AddressSpace::{Local, Loopback, Public};
        let (loopback, local, public) = (ip("127.0.0.1"), ip("192.168.1.10"), ip("8.8.8.8"));
        for (page, target, access) in [
            (Public, public, Allowed),
            (Public, local, NeedsPermission),
            (Public, loopback, NeedsPermission),
            (Local, public, Allowed),
            (Local, local, Allowed),
            (Local, loopback, NeedsPermission),
            (Loopback, public, Allowed),
            (Loopback, local, Allowed),
            (Loopback, loopback, Allowed),
        ] {
            assert_eq!(local_access(page, target, &[]), access, "{page:?} to {target}");
        }
        // What the viewer already allowed is allowed, in any spelling,
        // and nothing else is.
        assert_eq!(local_access(Public, ip("::ffff:192.168.1.10"), &[local]), Allowed);
        assert_eq!(local_access(Public, local, &[ip("::ffff:192.168.1.10")]), Allowed);
        assert_eq!(local_access(Public, ip("192.168.1.11"), &[local]), NeedsPermission);
        assert_eq!(local_access(Public, loopback, &[local]), NeedsPermission);
    }
}
