//! On Windows a fetcher runs in an `AppContainer` (decision 78), which
//! reaches this computer's own addresses only once an administrator has
//! exempted it from Windows' loopback isolation, with a command that names
//! the container by its security identifier. A per-user installer may not
//! run it, so the browser says why a channel on this computer was not
//! reached and gives the command (TODO-1251, `cookies.md` section 15
//! item 11): in the strip when such a channel is refused, and in the
//! settings panel beside the local network.

use std::net::IpAddr;

use neoteletext::address::Address;

/// What comes before the container's SID in the administrator's command:
/// `CheckNetIsolation` refuses the container's name and takes its SID.
const EXEMPTION_COMMAND: &str = "CheckNetIsolation.exe LoopbackExempt -a -p=";
/// The longest SID as text: the SDK's `SECURITY_MAX_SID_STRING_CHARACTERS`
/// (`winnt.h`), 187, less its terminating NUL.
const MAX_SID_TEXT: usize = 186;
const _: () = assert!(EXEMPTION_COMMAND.len() + MAX_SID_TEXT <= protocol::settings::MAX_LOOPBACK_EXEMPTION, "the panel is sent it whole");
/// What the strip says before the command.
const REFUSED: &str = "Windows blocks channels on this computer until an administrator runs ";
const _: () = assert!(REFUSED.len() + EXEMPTION_COMMAND.len() + MAX_SID_TEXT <= protocol::MAX_STRIP_NOTICE, "the strip says it whole");
/// The host every system resolves to this computer's own loopback.
const LOCALHOST: &str = "localhost";

/// The administrator's command that lets the fetcher's container reach
/// this computer, for the container `sid`.
pub(super) fn exemption_command(sid: &str) -> String {
    format!("{EXEMPTION_COMMAND}{sid}")
}

/// The command on this computer: on Windows, with the fetcher's container's
/// SID; elsewhere, where nothing like it is needed, none.
pub(super) fn this_computers_exemption() -> Option<String> {
    confine::fetcher_container().map(|sid| exemption_command(&sid))
}

/// Whether `address` is on this computer's own loopback: `localhost`, a
/// name under it, or a loopback address.
fn on_loopback(address: &Address) -> bool {
    let host = address.host.to_ascii_lowercase();
    if host == LOCALHOST || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// What the strip says of a channel at `address` that nothing answered,
/// given `exemption`, this computer's command: why, when Windows' loopback
/// isolation is the likely cause. The fetcher cannot tell it apart by its
/// error: measured in the Windows 11 virtual machine on 3 November 2026
/// (`a_fetcher_reaches_this_computer_only_with_the_exemption`), a fetcher
/// allowed the local network, connecting to a listener on 127.0.0.1, was
/// not refused but met silence, timing out after the three seconds it
/// was given.
pub(super) fn refused_text(address: &Address, exemption: Option<&str>) -> Option<String> {
    let command = exemption?;
    on_loopback(address).then(|| format!("{REFUSED}{command}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> Address {
        neoteletext::address::parse(text).unwrap()
    }

    const SID: &str = "S-1-15-2-1-2-3-4-5-6-7";

    /// REQ-2402, TODO-1251: on Windows a channel on this computer that nothing
    /// answered is said to be blocked by Windows, with the administrator's
    /// command and the fetcher container's SID in it; a channel elsewhere,
    /// or on a system with no such command, is not. Before, the strip said
    /// nothing of why.
    #[test]
    fn a_channel_on_this_computer_refused_on_windows_names_the_exemption() {
        let command = exemption_command(SID);
        assert_eq!(command, "CheckNetIsolation.exe LoopbackExempt -a -p=S-1-15-2-1-2-3-4-5-6-7");
        for here in ["http://127.0.0.1:8080/channel/", "http://localhost/channel/", "http://127.0.0.2/", "http://news.localhost/"] {
            assert_eq!(refused_text(&address(here), Some(&command)), Some(format!("{REFUSED}{command}")), "{here}");
            assert_eq!(refused_text(&address(here), None), None, "{here}: no command, as on macOS and Linux");
        }
        for elsewhere in ["https://news.example/teletext/", "http://192.168.1.20/", "http://10.0.0.1/"] {
            assert_eq!(refused_text(&address(elsewhere), Some(&command)), None, "{elsewhere}");
        }
    }

    /// The longest SID Windows writes (`SECURITY_MAX_SID_STRING_CHARACTERS`
    /// less its NUL) still fits the protocol's bound, and the strip's.
    #[test]
    fn the_longest_command_fits_its_bounds() {
        let longest = format!("S-1-{}{}", "9".repeat(15), "-4294967295".repeat(15));
        assert!(longest.len() <= MAX_SID_TEXT);
        assert!(exemption_command(&longest).len() <= protocol::settings::MAX_LOOPBACK_EXEMPTION);
        assert!(REFUSED.len() + exemption_command(&longest).len() <= protocol::MAX_STRIP_NOTICE);
    }
}
