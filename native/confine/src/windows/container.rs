//! The `AppContainer` profiles, one a role, and their capabilities.

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::io;
use std::ptr;
use std::sync::{Mutex, PoisonError};

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
        remove(name);
    }
}

/// Removes one `AppContainer` profile; one that is not there is no error.
pub(super) fn remove(name: &str) {
    let name = wide(name);
    // SAFETY: the name is NUL-terminated and outlives the call; its result is not needed,
    // since a profile not there is not an error here.
    let _ = unsafe { DeleteAppContainerProfile(name.as_ptr()) };
}

/// Held while a profile is made or found. `CreateAppContainerProfile`
/// answers "already exists" to a second caller while a first is still
/// making the profile, and a process started in it then fails with "the
/// system cannot find the file specified". Measured in the Windows 11
/// virtual machine, 24 October 2026, eight threads each making the same
/// new profile and starting a decoder in it: without the lock, starts
/// failed in 7 of 20 rounds (and in one of three runs of this crate's tests
/// with no profiles yet, two tests starting decoders at once); with it, in
/// none of 40.
static MAKING: Mutex<()> = Mutex::new(());

/// The `AppContainer`'s SID for a role, its profile made on first use.
pub(super) fn container(role: Role) -> io::Result<ContainerSid> {
    profile(match role {
        Role::Decoder => CONTAINER_DECODER,
        Role::Core => CONTAINER_CORE,
        Role::Sound => CONTAINER_SOUND,
        Role::Updater { .. } => CONTAINER_UPDATER,
        Role::Network => CONTAINER_NETWORK,
        Role::Interface => CONTAINER_INTERFACE,
    })
}

/// The SID of the `AppContainer` profile `name`, the profile made when it
/// is not there, and whole before any process of this one starts in it.
pub(super) fn profile(name: &str) -> io::Result<ContainerSid> {
    // Nothing the lock guards can be left half done by a panic.
    let _making = MAKING.lock().unwrap_or_else(PoisonError::into_inner);
    let name = wide(name);
    let display = wide("NeoTeletext");
    let description = wide("A helper process of the NeoTeletext browser");
    let mut sid: PSID = ptr::null_mut();
    // SAFETY: the names are NUL-terminated and alive; no capabilities are passed; the SID
    // is written to a live local and owned by `ContainerSid`.
    let made = unsafe { CreateAppContainerProfile(name.as_ptr(), display.as_ptr(), description.as_ptr(), ptr::null(), 0, &raw mut sid) };
    if made >= 0 {
        return Ok(ContainerSid::new(sid));
    }
    // It exists from an earlier run.
    // SAFETY: the name is NUL-terminated and alive; the SID is written to a live local and
    // owned by `ContainerSid`.
    if unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &raw mut sid) } < 0 {
        return Err(io::Error::other("cannot make or find the AppContainer profile"));
    }
    Ok(ContainerSid::new(sid))
}
