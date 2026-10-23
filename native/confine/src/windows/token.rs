//! The restricted token at low integrity: the interface's sandbox, which
//! needs the desktop a container cannot reach.

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{
    CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::System::SystemServices::{SE_GROUP_INTEGRITY, SE_GROUP_USE_FOR_DENY_ONLY};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::handles::{Owned, Sid, last_error};

/// The low integrity level's SID (`SECURITY_MANDATORY_LOW_RID`).
const LOW_INTEGRITY_SID: &str = "S-1-16-4096";
/// The administrators' group, for deny only.
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";

/// The trusted core's own token, restricted: the administrators' group
/// for deny only, no privileges, and low integrity.
pub(super) fn restricted_token() -> io::Result<Owned> {
    let mut ours: HANDLE = 0;
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT, &raw mut ours) } == 0 {
        return Err(last_error());
    }
    let ours = Owned(ours);
    let administrators = Sid::parse(ADMINISTRATORS_SID)?;
    let deny = [SID_AND_ATTRIBUTES { Sid: administrators.0, Attributes: u32::try_from(SE_GROUP_USE_FOR_DENY_ONLY).unwrap_or(0) }];
    let mut restricted: HANDLE = 0;
    if unsafe { CreateRestrictedToken(ours.0, DISABLE_MAX_PRIVILEGE, 1, deny.as_ptr(), 0, ptr::null(), 0, ptr::null(), &raw mut restricted) } == 0 {
        return Err(last_error());
    }
    let restricted = Owned(restricted);
    let low = Sid::parse(LOW_INTEGRITY_SID)?;
    let label = TOKEN_MANDATORY_LABEL { Label: SID_AND_ATTRIBUTES { Sid: low.0, Attributes: u32::try_from(SE_GROUP_INTEGRITY).unwrap_or(0) } };
    if unsafe {
        SetTokenInformation(restricted.0, TokenIntegrityLevel, ptr::from_ref(&label).cast(), u32::try_from(size_of::<TOKEN_MANDATORY_LABEL>()).unwrap_or(0))
    } == 0
    {
        return Err(last_error());
    }
    Ok(restricted)
}
