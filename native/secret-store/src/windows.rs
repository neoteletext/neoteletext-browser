//! Windows Credential Manager (`wincred.h`: `CredReadW`, `CredWriteW`,
//! `CredDeleteW`, `CredFree`). An entry is a generic credential
//! (`CRED_TYPE_GENERIC`) whose target name is `<account>.<service>`, its
//! user name the account and its persistence `CRED_PERSIST_ENTERPRISE`,
//! as `keyring` 4.2.0's Windows store made it, so a key it stored is read.
//!
//! The calls are foreign functions, so this module is allowed unsafe code;
//! every block says why it is sound.
#![expect(unsafe_code, reason = "Credential Manager is reached only through foreign calls (DEC-7168, requirement 10.12)")]

use windows_sys::Win32::Foundation::{ERROR_NO_SUCH_LOGON_SESSION, ERROR_NOT_FOUND, FILETIME, GetLastError};
use windows_sys::Win32::Security::Credentials::{
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_MAX_GENERIC_TARGET_NAME_LENGTH, CRED_PERSIST_ENTERPRISE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
    CredReadW, CredWriteW,
};

use crate::{Error, Secret, wipe};

// The store's bound is within the Credential Manager's own.
const _: () = assert!(crate::MAX_SECRET_BYTES <= CRED_MAX_CREDENTIAL_BLOB_SIZE as usize, "a secret fits a credential");

/// The longest target name, in UTF-16 units (`wincred.h`).
const MAX_TARGET_UNITS: usize = CRED_MAX_GENERIC_TARGET_NAME_LENGTH as usize;

/// The credential's target name, as `keyring`'s default store wrote it:
/// the account, a dot, and the service.
fn target_name(service: &str, account: &str) -> String {
    format!("{account}.{service}")
}

/// `text` as a zero-ended UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn checked_target(service: &str, account: &str) -> Result<Vec<u16>, Error> {
    let target = target_name(service, account);
    if target.encode_utf16().count() > MAX_TARGET_UNITS {
        return Err(Error::Refused(String::from("the credential's name is too long")));
    }
    Ok(wide(&target))
}

/// The last error of a failed call, as the caller sees it.
fn last_error() -> Error {
    // SAFETY: reads the calling thread's last-error value; no arguments.
    let code = unsafe { GetLastError() };
    if code == ERROR_NO_SUCH_LOGON_SESSION {
        Error::NoStore(String::from("the Credential Manager has no logon session"))
    } else {
        Error::Refused(format!("the Credential Manager answered error {code}"))
    }
}

pub(crate) fn read(service: &str, account: &str) -> Result<Option<Secret>, Error> {
    let target = checked_target(service, account)?;
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target` is a zero-ended UTF-16 string alive for the call, and
    // `found` a place for one pointer, written only on success with a
    // buffer the caller frees with `CredFree`, which is done below.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &raw mut found) } == 0 {
        // SAFETY: as in `last_error`.
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            return Ok(None);
        }
        return Err(last_error());
    }
    // SAFETY: on success `found` points to one CREDENTIALW the system
    // allocated, whose blob is `CredentialBlobSize` bytes at
    // `CredentialBlob` (or none); the bytes are copied, then set to zero in
    // the system's buffer, which is the caller's to write until it is
    // freed, and the buffer is freed once.
    unsafe {
        let credential = &mut *found;
        let length = usize::try_from(credential.CredentialBlobSize).map_err(|_| Error::TooLong)?;
        let secret = if credential.CredentialBlob.is_null() || length == 0 {
            Secret::copied(&[])
        } else {
            let blob = std::slice::from_raw_parts_mut(credential.CredentialBlob, length);
            let copied = Secret::copied(blob);
            wipe(blob);
            copied
        };
        CredFree(found.cast());
        secret.map(Some)
    }
}

pub(crate) fn write(service: &str, account: &str, secret: &[u8]) -> Result<(), Error> {
    let mut target = checked_target(service, account)?;
    let mut user = wide(account);
    let mut empty_comment = wide("");
    let mut empty_alias = wide("");
    let mut blob = secret.to_vec();
    let length = u32::try_from(blob.len()).map_err(|_| Error::TooLong)?;
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        Comment: empty_comment.as_mut_ptr(),
        LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
        CredentialBlobSize: length,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_ENTERPRISE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: empty_alias.as_mut_ptr(),
        UserName: user.as_mut_ptr(),
    };
    // SAFETY: every pointer in `credential` is to a buffer of this function
    // alive for the call (the strings zero-ended, the blob `length` bytes),
    // which the system only reads; CredWriteW replaces a credential of the
    // same target and type.
    let written = unsafe { CredWriteW(&raw const credential, 0) };
    let result = if written == 0 { Err(last_error()) } else { Ok(()) };
    wipe(&mut blob);
    result
}

pub(crate) fn delete(service: &str, account: &str) -> Result<(), Error> {
    let target = checked_target(service, account)?;
    // SAFETY: `target` is a zero-ended UTF-16 string alive for the call.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        // SAFETY: as in `last_error`.
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            return Ok(());
        }
        return Err(last_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `keyring` 4.2.0's Windows store's target name for the broker's two
    /// entries, so that a key it stored is read.
    #[test]
    fn the_target_name_is_keyrings() {
        assert_eq!(target_name("NeoTeletext viewer keys", "viewer-keys.bin"), "viewer-keys.bin.NeoTeletext viewer keys");
        assert_eq!(target_name("NeoTeletext cookie file", "cookies.bin"), "cookies.bin.NeoTeletext cookie file");
    }
}
