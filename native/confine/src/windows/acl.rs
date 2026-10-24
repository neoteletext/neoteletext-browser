//! Folder access lists: the core's settings and the updater's staging
//! folder granted to their containers, and the program folder to every
//! container (`installation-and-updates.md` section 4.2).

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::io;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{LocalFree, PSID};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
    TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows_sys::Win32::Security::{ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

use super::handles::{ContainerSid, Sid, wide};

/// `ALL APPLICATION PACKAGES` (`SECURITY_BUILTIN_PACKAGE_ANY_PACKAGE`).
const ALL_APPLICATION_PACKAGES_SID: &str = "S-1-15-2-1";
/// Read and run, for the program folder (`GENERIC_READ | GENERIC_EXECUTE`).
const GENERIC_READ_EXECUTE: u32 = 0x8000_0000 | 0x2000_0000;
/// Full access, for the core's settings folder (`GENERIC_ALL`).
const GENERIC_ALL: u32 = 0x1000_0000;
/// The grant reaches the folder's files and folders (`SUB_CONTAINERS_AND_OBJECTS_INHERIT`).
const INHERIT_TO_ALL: u32 = 0x3;

/// Grants the container `sid` full access to `folder` and everything in
/// it: the trusted core's settings (decision 56), the updater's staging
/// folder.
pub(super) fn grant(folder: &Path, sid: &ContainerSid) -> io::Result<()> {
    grant_to(folder, sid.raw(), GENERIC_ALL)
}

/// Grants `ALL APPLICATION PACKAGES` read and run on `folder` and
/// everything in it (`installation-and-updates.md` section 4.2).
pub(super) fn grant_packages_read(folder: &Path) -> io::Result<()> {
    let packages = Sid::parse(ALL_APPLICATION_PACKAGES_SID)?;
    grant_to(folder, packages.raw(), GENERIC_READ_EXECUTE)
}

/// Adds to `folder`'s access list an entry granting `sid` `permissions`,
/// inherited by everything in it.
fn grant_to(folder: &Path, sid: PSID, permissions: u32) -> io::Result<()> {
    let path = wide(folder);
    let mut old: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: the path is NUL-terminated and alive; the call writes the ACL pointer and
    // its owning descriptor into live locals, freed below.
    let read = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut old,
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if read != 0 {
        return Err(io::Error::from_raw_os_error(i32::try_from(read).unwrap_or(0)));
    }
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: permissions,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: INHERIT_TO_ALL,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: sid.cast(),
        },
    };
    let mut new: *mut ACL = ptr::null_mut();
    // SAFETY: `access` and `old` are alive (`old` owned by `descriptor`); the merged list
    // is written to a live local, freed below.
    let merged = unsafe { SetEntriesInAclW(1, &raw const access, old, &raw mut new) };
    let written = if merged == 0 {
        // SAFETY: the path is NUL-terminated and alive, and `new` the list SetEntriesInAclW
        // just made.
        unsafe {
            SetNamedSecurityInfoW(path.as_ptr().cast_mut(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, ptr::null_mut(), ptr::null_mut(), new, ptr::null())
        }
    } else {
        merged
    };
    // SAFETY: each pointer was allocated by the call that wrote it (or is null), and is
    // freed once, here.
    unsafe {
        LocalFree(new.cast());
        LocalFree(descriptor.cast());
    }
    if written != 0 { Err(io::Error::from_raw_os_error(i32::try_from(written).unwrap_or(0))) } else { Ok(()) }
}
