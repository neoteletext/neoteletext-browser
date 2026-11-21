//! The macOS keychain, through Security.framework's item calls (Apple's
//! "Keychain Services": `SecItemCopyMatching`, `SecItemAdd`,
//! `SecItemUpdate`, `SecItemDelete`) and the Core Foundation types they
//! take. An entry is a generic password (`kSecClassGenericPassword`) whose
//! service (`kSecAttrService`) and account (`kSecAttrAccount`) name it, in
//! the user's default keychain: without `kSecUseDataProtectionKeychain`,
//! the item calls use the file-based keychains, where `keyring` 4.2.0's
//! store (`SecKeychainAddGenericPassword` on the login keychain) kept it.
//!
//! The calls are foreign functions, so this module is allowed unsafe code;
//! every block says why it is sound.
#![expect(unsafe_code, reason = "Security.framework and Core Foundation are reached only through foreign calls (DEC-7168, requirement 10.12)")]

use std::ffi::c_void;

use crate::{Error, Secret};

type CFTypeRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFIndex = isize;
type CFTypeID = usize;
type CFStringEncoding = u32;
type OSStatus = i32;
type Boolean = u8;

/// `CFDictionaryKeyCallBacks` and `CFDictionaryValueCallBacks`
/// (`CFDictionary.h`): only their addresses are passed.
#[repr(C)]
struct KeyCallBacks {
    version: CFIndex,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
    hash: *const c_void,
}
#[repr(C)]
struct ValueCallBacks {
    version: CFIndex,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
}

/// `kCFStringEncodingUTF8` (`CFString.h`).
const UTF8: CFStringEncoding = 0x0800_0100;
/// `SecBase.h`: success, no such item, and an item already there.
const SUCCESS: OSStatus = 0;
const ITEM_NOT_FOUND: OSStatus = -25300;
const DUPLICATE_ITEM: OSStatus = -25299;
/// `SecBase.h`: no keychain could be found, or none is available.
const NO_SUCH_KEYCHAIN: OSStatus = -25294;
const NO_DEFAULT_KEYCHAIN: OSStatus = -25307;
const NOT_AVAILABLE: OSStatus = -25291;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: KeyCallBacks;
    static kCFTypeDictionaryValueCallBacks: ValueCallBacks;
    static kCFBooleanTrue: CFTypeRef;
    fn CFRelease(object: CFTypeRef);
    fn CFGetTypeID(object: CFTypeRef) -> CFTypeID;
    fn CFDataGetTypeID() -> CFTypeID;
    fn CFDataCreate(allocator: CFAllocatorRef, bytes: *const u8, length: CFIndex) -> CFTypeRef;
    fn CFDataGetLength(data: CFTypeRef) -> CFIndex;
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
    fn CFStringCreateWithBytes(allocator: CFAllocatorRef, bytes: *const u8, length: CFIndex, encoding: CFStringEncoding, external: Boolean) -> CFTypeRef;
    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: CFIndex,
        key_callbacks: *const KeyCallBacks,
        value_callbacks: *const ValueCallBacks,
    ) -> CFTypeRef;
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: CFTypeRef;
    static kSecClassGenericPassword: CFTypeRef;
    static kSecAttrService: CFTypeRef;
    static kSecAttrAccount: CFTypeRef;
    static kSecAttrLabel: CFTypeRef;
    static kSecValueData: CFTypeRef;
    static kSecReturnData: CFTypeRef;
    static kSecMatchLimit: CFTypeRef;
    static kSecMatchLimitOne: CFTypeRef;
    fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemAdd(attributes: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemUpdate(query: CFTypeRef, attributes: CFTypeRef) -> OSStatus;
    fn SecItemDelete(query: CFTypeRef) -> OSStatus;
}

/// A Core Foundation object this module owns (Core Foundation's "Create
/// Rule": made by a function with `Create` or `Copy` in its name), released
/// when dropped.
struct Owned(CFTypeRef);

impl Owned {
    /// Takes `object`, or none for a null one, which a create function
    /// returns when it fails.
    fn new(object: CFTypeRef) -> Option<Self> {
        (!object.is_null()).then_some(Self(object))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the object is non-null and owned by this value under the
        // Create Rule, and this is its one release.
        unsafe { CFRelease(self.0) };
    }
}

fn out_of_memory() -> Error {
    Error::Refused(String::from("Core Foundation could not make an object"))
}

fn string(text: &str) -> Result<Owned, Error> {
    let length = CFIndex::try_from(text.len()).map_err(|_| out_of_memory())?;
    // SAFETY: the pointer and length are `text`'s, valid UTF-8 for the
    // call, which copies them (not external); the result is checked for
    // null.
    Owned::new(unsafe { CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), length, UTF8, 0) }).ok_or_else(out_of_memory)
}

fn data(bytes: &[u8]) -> Result<Owned, Error> {
    let length = CFIndex::try_from(bytes.len()).map_err(|_| out_of_memory())?;
    // SAFETY: the pointer and length are `bytes`', read for the call, which
    // copies them; the result is checked for null.
    Owned::new(unsafe { CFDataCreate(std::ptr::null(), bytes.as_ptr(), length) }).ok_or_else(out_of_memory)
}

/// A dictionary of `pairs`, which retains each key and value (the
/// `kCFType...CallBacks`), so they may be released after it is made.
fn dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> Result<Owned, Error> {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|(key, _)| *key).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|(_, value)| *value).collect();
    let count = CFIndex::try_from(pairs.len()).map_err(|_| out_of_memory())?;
    // SAFETY: `keys` and `values` hold `count` valid Core Foundation
    // objects each, alive for the call; the callbacks are Core
    // Foundation's own statics; the result is checked for null.
    Owned::new(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            count,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    })
    .ok_or_else(out_of_memory)
}

/// The query naming the entry, with `extra` pairs.
fn query(service: &Owned, account: &Owned, extra: &[(CFTypeRef, CFTypeRef)]) -> Result<Owned, Error> {
    // SAFETY: the `kSec...` constants are Security.framework's immutable
    // statics, initialised when the framework is loaded, before `main`.
    let mut pairs = unsafe { vec![(kSecClass, kSecClassGenericPassword), (kSecAttrService, service.0), (kSecAttrAccount, account.0)] };
    pairs.extend_from_slice(extra);
    dictionary(&pairs)
}

/// What a status means to the caller.
fn status_error(status: OSStatus) -> Error {
    match status {
        NO_SUCH_KEYCHAIN | NO_DEFAULT_KEYCHAIN | NOT_AVAILABLE => Error::NoStore(format!("the keychain is not available (OSStatus {status})")),
        _ => Error::Refused(format!("the keychain answered OSStatus {status}")),
    }
}

pub(crate) fn read(service: &str, account: &str) -> Result<Option<Secret>, Error> {
    let (service, account) = (string(service)?, string(account)?);
    // SAFETY: Security.framework's immutable statics, as in `query`.
    let extra = unsafe { [(kSecReturnData, kCFBooleanTrue), (kSecMatchLimit, kSecMatchLimitOne)] };
    let query = query(&service, &account, &extra)?;
    let mut result: CFTypeRef = std::ptr::null();
    // SAFETY: `query` is a valid dictionary and `result` a place for one
    // pointer, written only on success with an object the caller owns
    // (the Copy Rule), which `Owned` releases.
    let status = unsafe { SecItemCopyMatching(query.0, &raw mut result) };
    match status {
        SUCCESS => {}
        ITEM_NOT_FOUND => return Ok(None),
        _ => return Err(status_error(status)),
    }
    let found = Owned::new(result).ok_or_else(|| Error::Refused(String::from("the keychain answered no data")))?;
    // SAFETY: `found` is a live Core Foundation object.
    if unsafe { CFGetTypeID(found.0) != CFDataGetTypeID() } {
        return Err(Error::Refused(String::from("the keychain answered something other than data")));
    }
    // SAFETY: `found` is a CFData, whose length is not negative and whose
    // bytes are valid for that length while it lives; the length is
    // bounded before anything is copied, and the bytes are copied before
    // `found` is released.
    let secret = unsafe {
        let length = usize::try_from(CFDataGetLength(found.0)).map_err(|_| Error::Refused(String::from("the keychain answered a negative length")))?;
        if length > crate::MAX_SECRET_BYTES {
            return Err(Error::TooLong);
        }
        let bytes = CFDataGetBytePtr(found.0);
        if bytes.is_null() && length != 0 {
            return Err(Error::Refused(String::from("the keychain answered no bytes")));
        }
        Secret::copied(if length == 0 { &[] } else { std::slice::from_raw_parts(bytes, length) })?
    };
    Ok(Some(secret))
}

pub(crate) fn write(service_name: &str, account_name: &str, secret: &[u8]) -> Result<(), Error> {
    let (service, account) = (string(service_name)?, string(account_name)?);
    let value = data(secret)?;
    // SAFETY: Security.framework's immutable static, as in `query`.
    let value_key = unsafe { kSecValueData };
    let update = dictionary(&[(value_key, value.0)])?;
    let existing = query(&service, &account, &[])?;
    // SAFETY: both dictionaries are valid for the call, which reads them.
    let status = unsafe { SecItemUpdate(existing.0, update.0) };
    match status {
        SUCCESS => return Ok(()),
        ITEM_NOT_FOUND => {}
        _ => return Err(status_error(status)),
    }
    // The label is the service, as the keychain shows an item added with
    // `SecKeychainAddGenericPassword`, which `keyring` used.
    // SAFETY: Security.framework's immutable statics, as in `query`.
    let extra = unsafe { [(kSecAttrLabel, service.0), (kSecValueData, value.0)] };
    let added = query(&service, &account, &extra)?;
    // SAFETY: `added` is valid for the call; no result is asked for.
    let status = unsafe { SecItemAdd(added.0, std::ptr::null_mut()) };
    match status {
        SUCCESS => Ok(()),
        // Added by another process between the two calls: replace it.
        DUPLICATE_ITEM => {
            // SAFETY: as for the first update.
            let status = unsafe { SecItemUpdate(existing.0, update.0) };
            if status == SUCCESS { Ok(()) } else { Err(status_error(status)) }
        }
        _ => Err(status_error(status)),
    }
}

pub(crate) fn delete(service: &str, account: &str) -> Result<(), Error> {
    let (service, account) = (string(service)?, string(account)?);
    let query = query(&service, &account, &[])?;
    // SAFETY: `query` is valid for the call, which reads it.
    match unsafe { SecItemDelete(query.0) } {
        SUCCESS | ITEM_NOT_FOUND => Ok(()),
        status => Err(status_error(status)),
    }
}
