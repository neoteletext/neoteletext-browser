//! Windows: Authenticode's check of a program with its signer's public
//! key pinned (`installation-and-updates.md` section 5), the per-user
//! uninstall entry Settings › Apps reads (section 4.2), and the message
//! box the uninstaller asks with (requirement 13.20).

#![allow(unsafe_code, reason = "the Win32 trust, certificate, registry and message box calls (requirement 10.12, as amended for this crate)")]
#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "each call's safety is argued once, at the module's head: every pointer passed is to memory this module owns for the call's length, and every handle is one this module opened and closes once"
)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HANDLE, LocalFree};
use windows_sys::Win32::Security::Cryptography::{CRYPT_ENCODE_ALLOC_FLAG, CryptEncodeObjectEx, X509_ASN_ENCODING, X509_PUBLIC_KEY_INFO};
use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW};

use update::accept::Signer;
use update::manifest::Manifest;
use update::platform::{PROGRAMS, Platform};

/// `text` as a zero-terminated UTF-16 string.
fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// A structure's size, as the calls take it.
fn size_of_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

pub fn ask(title: &str, question: &str) -> bool {
    let (title, question) = (wide(title), wide(question));
    unsafe { MessageBoxW(0, question.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

pub fn tell(title: &str, message: &str) {
    let (title, message) = (wide(title), wide(message));
    unsafe { MessageBoxW(0, message.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

/// The SHA-256 of the public key of the certificate that signed `path`,
/// once `WinVerifyTrust` has found its Authenticode signature valid.
///
/// # Errors
/// When the signature is missing or not valid, or its signer cannot be read.
pub fn signer_key(path: &Path) -> Result<[u8; update::SHA256_BYTES], String> {
    let name = wide(path);
    let mut file =
        WINTRUST_FILE_INFO { cbStruct: size_of_u32::<WINTRUST_FILE_INFO>(), pcwszFilePath: name.as_ptr(), hFile: 0, pgKnownSubject: ptr::null_mut() };
    let mut data: WINTRUST_DATA = unsafe { std::mem::zeroed() };
    data.cbStruct = size_of_u32::<WINTRUST_DATA>();
    data.dwUIChoice = WTD_UI_NONE;
    // The signer's key is pinned, so a revoked certificate of ours is
    // replaced by a launcher release; revocation's network check would
    // stop every start on a machine offline.
    data.fdwRevocationChecks = WTD_REVOKE_NONE;
    data.dwUnionChoice = WTD_CHOICE_FILE;
    data.Anonymous = WINTRUST_DATA_0 { pFile: &raw mut file };
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let status = unsafe { WinVerifyTrust(0, &raw mut action, ptr::from_mut(&mut data).cast()) };
    let key = if status == 0 { pinned_key(data.hWVTStateData) } else { Err(format!("its Authenticode signature is not valid (0x{status:08x})")) };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe { WinVerifyTrust(0, &raw mut action, ptr::from_mut(&mut data).cast()) };
    key
}

/// The SHA-256 of the DER `SubjectPublicKeyInfo` of the first signer's
/// certificate in a verified state.
fn pinned_key(state: HANDLE) -> Result<[u8; update::SHA256_BYTES], String> {
    let provider = unsafe { WTHelperProvDataFromStateData(state) };
    if provider.is_null() {
        return Err(String::from("no signer"));
    }
    let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, 0, 0) };
    if signer.is_null() {
        return Err(String::from("no signer"));
    }
    let (count, chain) = unsafe { ((*signer).csCertChain, (*signer).pasCertChain) };
    if count == 0 || chain.is_null() {
        return Err(String::from("no signer certificate"));
    }
    // The chain's first certificate is the signer's own.
    let certificate = unsafe { (*chain).pCert };
    if certificate.is_null() || unsafe { (*certificate).pCertInfo }.is_null() {
        return Err(String::from("no signer certificate"));
    }
    let info = unsafe { &raw const (*(*certificate).pCertInfo).SubjectPublicKeyInfo };
    let mut encoded: *mut u8 = ptr::null_mut();
    let mut length = 0_u32;
    let made = unsafe {
        CryptEncodeObjectEx(
            X509_ASN_ENCODING,
            X509_PUBLIC_KEY_INFO,
            info.cast(),
            CRYPT_ENCODE_ALLOC_FLAG,
            ptr::null(),
            (&raw mut encoded).cast(),
            &raw mut length,
        )
    };
    if made == 0 || encoded.is_null() {
        return Err(String::from("the signer's key cannot be read"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(encoded, usize::try_from(length).unwrap_or(0)) };
    let key = update::sha256(bytes);
    unsafe { LocalFree(encoded.cast()) };
    Ok(key)
}

/// Windows's check (section 5): every program of the version carries a
/// valid Authenticode signature by the pinned signer, after
/// `update::accept::accept` has checked the manifest's signature by the
/// release key (decision 92).
#[derive(Debug)]
pub struct WindowsSigner {
    /// SHA-256 of the pinned signer's public key.
    pub key: Option<[u8; update::SHA256_BYTES]>,
    /// The pinned release key, which signs the manifest.
    pub release: Option<[u8; update::signature::PUBLIC_KEY_BYTES]>,
}

impl WindowsSigner {
    /// Whether one program is signed by the pinned signer.
    ///
    /// # Errors
    /// Why not.
    pub fn check_program(&self, path: &Path) -> Result<(), String> {
        let pinned = self.key.as_ref().ok_or("this build carries no signer key, so it accepts no program")?;
        let key = signer_key(path).map_err(|error| format!("{}: {error}", path.display()))?;
        if &key == pinned { Ok(()) } else { Err(format!("{} is signed by another signer", path.display())) }
    }
}

impl Signer for WindowsSigner {
    fn release_key(&self) -> Option<&[u8; update::signature::PUBLIC_KEY_BYTES]> {
        self.release.as_ref()
    }

    fn check(&self, folder: &Path, _manifest: &Manifest) -> Result<(), String> {
        for program in PROGRAMS {
            self.check_program(&folder.join(Platform::WindowsX86_64.program_file(program)))?;
        }
        Ok(())
    }
}

/// Where Settings › Apps finds a per-user program.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\NeoTeletext";

/// What the uninstall entry says.
#[derive(Debug)]
pub struct UninstallEntry<'a> {
    /// The name shown.
    pub name: &'a str,
    /// The version shown.
    pub version: &'a str,
    /// Who publishes it.
    pub publisher: &'a str,
    /// The installation's folder.
    pub location: &'a Path,
    /// The command that removes it.
    pub uninstall: &'a str,
    /// The program whose icon is shown.
    pub icon: &'a Path,
    /// The size installed, in kibibytes.
    pub size_kib: u32,
}

/// A registry key, closed when dropped.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn set_text(key: &Key, name: &str, value: &str) -> Result<(), String> {
    let (name, value) = (wide(name), wide(value));
    let bytes = u32::try_from(value.len().saturating_mul(2)).map_err(|_| String::from("a value too long"))?;
    let written = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_SZ, value.as_ptr().cast(), bytes) };
    if written == 0 { Ok(()) } else { Err(format!("the registry refused a value ({written})")) }
}

fn set_number(key: &Key, name: &str, value: u32) -> Result<(), String> {
    let name = wide(name);
    let bytes = value.to_le_bytes();
    let written = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_DWORD, bytes.as_ptr(), size_of_u32::<u32>()) };
    if written == 0 { Ok(()) } else { Err(format!("the registry refused a value ({written})")) }
}

/// Writes the per-user uninstall entry (section 4.2), so Settings › Apps
/// lists the browser and removes it with `entry.uninstall`. No
/// administrator: the user's own hive.
///
/// # Errors
/// When the registry refuses.
pub fn uninstall_entry(entry: &UninstallEntry<'_>) -> Result<(), String> {
    let path = wide(UNINSTALL_KEY);
    let mut key: HKEY = 0;
    let made = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, ptr::null(), &raw mut key, ptr::null_mut())
    };
    if made != 0 {
        return Err(format!("the registry refused the uninstall entry ({made})"));
    }
    let key = Key(key);
    set_text(&key, "DisplayName", entry.name)?;
    set_text(&key, "DisplayVersion", entry.version)?;
    set_text(&key, "Publisher", entry.publisher)?;
    set_text(&key, "InstallLocation", &entry.location.display().to_string())?;
    set_text(&key, "UninstallString", entry.uninstall)?;
    set_text(&key, "DisplayIcon", &entry.icon.display().to_string())?;
    set_number(&key, "EstimatedSize", entry.size_kib)?;
    // Nothing to modify or repair: an update is the updater's, and there
    // is no Windows Installer to put old files back (section 4.2).
    set_number(&key, "NoModify", 1)?;
    set_number(&key, "NoRepair", 1)
}

/// Removes the uninstall entry, if it is there.
///
/// # Errors
/// When the registry refuses.
pub fn remove_uninstall_entry() -> Result<(), String> {
    let path = wide(UNINSTALL_KEY);
    let removed = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
    if removed == 0 || removed == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(format!("the registry refused to remove the uninstall entry ({removed})")) }
}
