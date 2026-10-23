//! The `AppContainer` profiles, one a role, and their capabilities.

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::PSID;
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName};

use super::handles::{ContainerSid, wide};
use crate::Role;

/// The internet client capability (`SECURITY_CAPABILITY_INTERNET_CLIENT`).
pub(super) const INTERNET_CLIENT_SID: &str = "S-1-15-3-1";
/// The private network client and server capability.
pub(super) const PRIVATE_NETWORK_SID: &str = "S-1-15-3-3";
/// The `AppContainer` profiles, one a role, registered once for the user.
const CONTAINER_DECODER: &str = "NeoTeletext.Decoder";
const CONTAINER_NETWORK: &str = "NeoTeletext.Fetcher";
const CONTAINER_CORE: &str = "NeoTeletext.Core";
const CONTAINER_SOUND: &str = "NeoTeletext.Sound";
const CONTAINER_UPDATER: &str = "NeoTeletext.Updater";
/// The interface's, with no capabilities (decision 90).
const CONTAINER_INTERFACE: &str = "NeoTeletext.Interface";
/// Every container's name, for the uninstaller to remove them.
const CONTAINERS: [&str; 6] = [CONTAINER_DECODER, CONTAINER_NETWORK, CONTAINER_CORE, CONTAINER_SOUND, CONTAINER_UPDATER, CONTAINER_INTERFACE];

/// Removes the browser's `AppContainer` profiles, which the first start of
/// each helper made for the user (requirement 13.20). A profile that is
/// not there is no error.
pub(super) fn remove_containers() {
    for name in CONTAINERS {
        let name = wide(name);
        // The name is NUL-terminated and outlives the call; its result is
        // not needed, since a profile not there is not an error here.
        let _ = unsafe { DeleteAppContainerProfile(name.as_ptr()) };
    }
}

/// The `AppContainer`'s SID for a role, its profile made on first use.
pub(super) fn container(role: Role) -> io::Result<ContainerSid> {
    let name = wide(match role {
        Role::Decoder => CONTAINER_DECODER,
        Role::Core => CONTAINER_CORE,
        Role::Sound => CONTAINER_SOUND,
        Role::Updater { .. } => CONTAINER_UPDATER,
        Role::Network => CONTAINER_NETWORK,
        Role::Interface => CONTAINER_INTERFACE,
    });
    let display = wide("NeoTeletext");
    let description = wide("A helper process of the NeoTeletext browser");
    let mut sid: PSID = ptr::null_mut();
    let made = unsafe { CreateAppContainerProfile(name.as_ptr(), display.as_ptr(), description.as_ptr(), ptr::null(), 0, &raw mut sid) };
    if made >= 0 {
        return Ok(ContainerSid(sid));
    }
    // It exists from an earlier run.
    if unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &raw mut sid) } < 0 {
        return Err(io::Error::other("cannot make or find the AppContainer profile"));
    }
    Ok(ContainerSid(sid))
}
