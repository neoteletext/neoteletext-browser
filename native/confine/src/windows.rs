//! The helpers' sandboxes on Windows, which the trusted core applies as
//! it starts them, as every sandboxed program on Windows does since
//! Vista: the token is the sandbox.
//!
//! - **A job object** with kill-on-close, one process at most and the UI
//!   restrictions the role allows: the helper dies with the trusted core
//!   and can start nothing.
//! - **A restricted token at low integrity**, with the administrators'
//!   group for deny only and every privilege removed: it cannot write to
//!   the user's files, registry or other processes.
//! - **Process mitigation policies:** DEP, ASLR, control-flow guard,
//!   strict handle checks, no extension points, no remote or
//!   low-integrity images, no dynamic code (Rust needs none), no child
//!   processes; and for a helper with no window, the Win32k system-call
//!   filter, so it cannot reach the window system at all.
//! - **An `AppContainer`** for every process but the interface: the
//!   decoder and the trusted core in a less privileged one with no
//!   capabilities, the sound helper with none, the fetcher and keyboard
//!   with the internet client capability (and the private network one
//!   when the user allowed local addresses). The program's folder must
//!   grant read to `ALL APPLICATION PACKAGES`, which an installation under
//!   Program Files does and the installer sets elsewhere; when the
//!   `AppContainer` cannot be made or started, the process is not started
//!   at all (decision 58: security is never downgraded).
//! - **The interface** has the restricted token at low integrity in place
//!   of a container, by design: it needs the desktop.
//!
//! Only the two pipe handles are inherited, by the handle list; the
//! environment is the one the trusted core chose, and nothing else.

#![allow(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]
#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "each call's safety is argued once, at the module's head: every handle and pointer passed is one this module made and still owns"
)]

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::FromRawHandle as _;
use std::os::windows::process::ExitStatusExt as _;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, GetLastError, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree, PSID,
    SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
use windows_sys::Win32::Security::{
    ACL, CreateRestrictedToken, DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE, FreeSid, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
    SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    TokenIntegrityLevel,
};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
    JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicUIRestrictions,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemServices::{
    SE_GROUP_ENABLED, SE_GROUP_INTEGRITY, SE_GROUP_USE_FOR_DENY_ONLY, TIME_ZONE_ID_DAYLIGHT, TIME_ZONE_ID_STANDARD,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetCurrentProcess, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_ID_INVALID, TIME_ZONE_INFORMATION};
use windows_sys::Win32::System::WindowsProgramming::{PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT, PROCESS_CREATION_CHILD_PROCESS_RESTRICTED};

use crate::{Process, Report, Role, Spawn};

/// The process mitigation policies, as `winbase.h` defines them for the
/// `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY` attribute's first 64 bits;
/// `windows-sys` does not carry these macros.
mod mitigation {
    pub const DEP_ENABLE: u64 = 0x1;
    pub const FORCE_RELOCATE_IMAGES_ALWAYS_ON: u64 = 0x1 << 8;
    pub const HEAP_TERMINATE_ALWAYS_ON: u64 = 0x1 << 12;
    pub const BOTTOM_UP_ASLR_ALWAYS_ON: u64 = 0x1 << 16;
    pub const HIGH_ENTROPY_ASLR_ALWAYS_ON: u64 = 0x1 << 20;
    pub const STRICT_HANDLE_CHECKS_ALWAYS_ON: u64 = 0x1 << 24;
    pub const WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON: u64 = 0x1 << 28;
    pub const EXTENSION_POINT_DISABLE_ALWAYS_ON: u64 = 0x1 << 32;
    pub const PROHIBIT_DYNAMIC_CODE_ALWAYS_ON: u64 = 0x1 << 36;
    pub const CONTROL_FLOW_GUARD_ALWAYS_ON: u64 = 0x1 << 40;
    pub const IMAGE_LOAD_NO_REMOTE_ALWAYS_ON: u64 = 0x1 << 52;
    pub const IMAGE_LOAD_NO_LOW_LABEL_ALWAYS_ON: u64 = 0x1 << 56;
}

/// The low integrity level's SID (`SECURITY_MANDATORY_LOW_RID`).
const LOW_INTEGRITY_SID: &str = "S-1-16-4096";
/// The administrators' group, for deny only.
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";
/// The internet client capability (`SECURITY_CAPABILITY_INTERNET_CLIENT`).
const INTERNET_CLIENT_SID: &str = "S-1-15-3-1";
/// The private network client and server capability.
const PRIVATE_NETWORK_SID: &str = "S-1-15-3-3";
/// The `AppContainer` profiles, one a role, registered once for the user.
const CONTAINER_DECODER: &str = "NeoTeletext.Decoder";
const CONTAINER_NETWORK: &str = "NeoTeletext.Fetcher";
const CONTAINER_CORE: &str = "NeoTeletext.Core";
const CONTAINER_SOUND: &str = "NeoTeletext.Sound";
const CONTAINER_UPDATER: &str = "NeoTeletext.Updater";
/// Every container's name, for the uninstaller to remove them.
const CONTAINERS: [&str; 5] = [CONTAINER_DECODER, CONTAINER_NETWORK, CONTAINER_CORE, CONTAINER_SOUND, CONTAINER_UPDATER];

/// Removes the browser's `AppContainer` profiles, which the first start of
/// each helper made for the user (requirement 13.20). A profile that is
/// not there is no error.
pub fn remove_containers() {
    for name in CONTAINERS {
        let name = wide(name);
        // The name is NUL-terminated and outlives the call; its result is
        // not needed, since a profile not there is not an error here.
        let _ = unsafe { DeleteAppContainerProfile(name.as_ptr()) };
    }
}
/// `ALL APPLICATION PACKAGES` (`SECURITY_BUILTIN_PACKAGE_ANY_PACKAGE`).
const ALL_APPLICATION_PACKAGES_SID: &str = "S-1-15-2-1";
/// Read and run, for the program folder (`GENERIC_READ | GENERIC_EXECUTE`).
const GENERIC_READ_EXECUTE: u32 = 0x8000_0000 | 0x2000_0000;
/// Full access, for the core's settings folder (`GENERIC_ALL`).
const GENERIC_ALL: u32 = 0x1000_0000;
/// The grant reaches the folder's files and folders (`SUB_CONTAINERS_AND_OBJECTS_INHERIT`).
const INHERIT_TO_ALL: u32 = 0x3;
/// How many attributes a start carries at most.
const ATTRIBUTES: u32 = 5;

/// A helper's process and its job.
#[derive(Debug)]
pub struct Inner {
    process: HANDLE,
    job: HANDLE,
}

impl Inner {
    /// The process, for duplicating handles into it.
    pub(crate) fn raw_process(&self) -> HANDLE {
        self.process
    }

    pub fn kill(&mut self) -> io::Result<()> {
        if unsafe { TerminateProcess(self.process, 1) } == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if unsafe { WaitForSingleObject(self.process, INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut code = 0_u32;
        if unsafe { GetExitCodeProcess(self.process, &raw mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ExitStatus::from_raw(code))
    }
}

impl Drop for Inner {
    /// Closing the job ends the helper, if it still runs.
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.process);
            CloseHandle(self.job);
        }
    }
}

/// A handle closed when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A SID from its text, freed when dropped.
struct Sid(PSID);

impl Sid {
    fn parse(text: &str) -> io::Result<Self> {
        let mut sid: PSID = ptr::null_mut();
        if unsafe { ConvertStringSidToSidW(wide(text).as_ptr(), &raw mut sid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(sid))
    }
}

impl Drop for Sid {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0.cast()) };
    }
}

/// A SID of an `AppContainer`, freed with `FreeSid`.
struct ContainerSid(PSID);

impl Drop for ContainerSid {
    fn drop(&mut self) {
        unsafe { FreeSid(self.0) };
    }
}

/// `text` as a zero-terminated UTF-16 string.
fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// The last error, as an `io::Error`.
fn last_error() -> io::Error {
    io::Error::from_raw_os_error(i32::try_from(unsafe { GetLastError() }).unwrap_or(0))
}

/// The command line as `CreateProcess` reads it: each argument quoted,
/// with its quotes and the backslashes before them escaped.
fn command_line(program: &OsStr, arguments: &[OsString]) -> Vec<u16> {
    let mut line = String::new();
    for (index, argument) in std::iter::once(program).chain(arguments.iter().map(OsString::as_os_str)).enumerate() {
        if index > 0 {
            line.push(' ');
        }
        line.push('"');
        let text = argument.to_string_lossy();
        let mut backslashes = 0_usize;
        for character in text.chars() {
            match character {
                '\\' => backslashes = backslashes.saturating_add(1),
                '"' => {
                    line.extend(std::iter::repeat_n('\\', backslashes.saturating_mul(2).saturating_add(1)));
                    backslashes = 0;
                    line.push('"');
                    continue;
                }
                _ => {
                    line.extend(std::iter::repeat_n('\\', backslashes));
                    backslashes = 0;
                }
            }
            if character != '\\' {
                line.push(character);
            }
        }
        line.extend(std::iter::repeat_n('\\', backslashes.saturating_mul(2)));
        line.push('"');
    }
    wide(line)
}

/// The environment block: `NAME=value` strings, each zero-terminated,
/// then one more zero.
fn environment_block(environment: &[(OsString, OsString)]) -> Vec<u16> {
    let mut block = Vec::new();
    for (name, value) in environment {
        block.extend(name.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

/// A pipe: the end the helper gets inherits; ours does not.
fn pipe() -> io::Result<(Owned, Owned)> {
    let mut read: HANDLE = 0;
    let mut write: HANDLE = 0;
    let attributes =
        SECURITY_ATTRIBUTES { nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0), lpSecurityDescriptor: ptr::null_mut(), bInheritHandle: 1 };
    if unsafe { CreatePipe(&raw mut read, &raw mut write, &raw const attributes, 0) } == 0 {
        return Err(last_error());
    }
    Ok((Owned(read), Owned(write)))
}

fn keep_ours(handle: HANDLE) -> io::Result<()> {
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 { Err(last_error()) } else { Ok(()) }
}

/// The job the helper runs in.
fn job(role: Role) -> io::Result<Owned> {
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job == 0 {
        return Err(last_error());
    }
    let job = Owned(job);
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            ptr::from_ref(&limits).cast(),
            u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
        )
    } == 0
    {
        return Err(last_error());
    }
    // The interface has windows and reads the clipboard for a paste; the
    // others have no business with the desktop at all.
    let mut restrictions =
        JOB_OBJECT_UILIMIT_EXITWINDOWS | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS | JOB_OBJECT_UILIMIT_GLOBALATOMS;
    if role != Role::Interface {
        restrictions |= JOB_OBJECT_UILIMIT_DESKTOP | JOB_OBJECT_UILIMIT_HANDLES | JOB_OBJECT_UILIMIT_READCLIPBOARD | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;
    }
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS { UIRestrictionsClass: restrictions };
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectBasicUIRestrictions,
            ptr::from_ref(&ui).cast(),
            u32::try_from(size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>()).unwrap_or(0),
        )
    } == 0
    {
        return Err(last_error());
    }
    Ok(job)
}

/// The trusted core's own token, restricted: the administrators' group
/// for deny only, no privileges, and low integrity.
fn restricted_token() -> io::Result<Owned> {
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

/// The `AppContainer`'s SID for a role, its profile made on first use.
fn container(role: Role) -> io::Result<ContainerSid> {
    let name = wide(match role {
        Role::Decoder => CONTAINER_DECODER,
        Role::Core => CONTAINER_CORE,
        Role::Sound => CONTAINER_SOUND,
        Role::Updater { .. } => CONTAINER_UPDATER,
        Role::Network | Role::Interface => CONTAINER_NETWORK,
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

/// The attribute list of a start: inherited handles, mitigation policies,
/// no child processes, and for a container its capabilities.
struct Attributes {
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
    _storage: Vec<u8>,
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

fn attributes(count: u32) -> io::Result<Attributes> {
    let mut size = 0_usize;
    // The first call says how much room the list needs.
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &raw mut size) };
    let mut storage = vec![0_u8; size];
    let list: LPPROC_THREAD_ATTRIBUTE_LIST = storage.as_mut_ptr().cast();
    if unsafe { InitializeProcThreadAttributeList(list, count, 0, &raw mut size) } == 0 {
        return Err(last_error());
    }
    Ok(Attributes { list, _storage: storage })
}

fn add_attribute<T>(attributes: &Attributes, which: usize, value: &T, bytes: usize) -> io::Result<()> {
    add_attribute_at(attributes, which, ptr::from_ref(value).cast::<std::ffi::c_void>(), bytes)
}

fn add_attribute_at(attributes: &Attributes, which: usize, value: *const std::ffi::c_void, bytes: usize) -> io::Result<()> {
    if unsafe { UpdateProcThreadAttribute(attributes.list, 0, which, value.cast_mut(), bytes, ptr::null_mut(), ptr::null()) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// Starts the helper in a container (`capabilities` given) or with the
/// restricted token.
#[allow(clippy::too_many_lines, reason = "one start, each attribute in turn")]
fn start(
    spawn: &Spawn<'_>,
    inherited: &[HANDLE],
    stdin: HANDLE,
    stdout: HANDLE,
    container: Option<(&ContainerSid, &[Sid])>,
) -> io::Result<PROCESS_INFORMATION> {
    let count = if container.is_some() { ATTRIBUTES } else { ATTRIBUTES.saturating_sub(2) };
    let attributes = attributes(count)?;
    // Only the pipes reach the helper, whatever else the core has open.
    add_attribute_at(&attributes, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, inherited.as_ptr().cast(), size_of_val(inherited))?;
    let mut policy = mitigation::DEP_ENABLE
        | mitigation::FORCE_RELOCATE_IMAGES_ALWAYS_ON
        | mitigation::HEAP_TERMINATE_ALWAYS_ON
        | mitigation::BOTTOM_UP_ASLR_ALWAYS_ON
        | mitigation::HIGH_ENTROPY_ASLR_ALWAYS_ON
        | mitigation::STRICT_HANDLE_CHECKS_ALWAYS_ON
        | mitigation::EXTENSION_POINT_DISABLE_ALWAYS_ON
        | mitigation::PROHIBIT_DYNAMIC_CODE_ALWAYS_ON
        | mitigation::CONTROL_FLOW_GUARD_ALWAYS_ON
        | mitigation::IMAGE_LOAD_NO_REMOTE_ALWAYS_ON
        | mitigation::IMAGE_LOAD_NO_LOW_LABEL_ALWAYS_ON;
    // No window system for those without a window. The sound helper keeps
    // it: Core Audio's COM apartments may need it.
    if matches!(spawn.role, Role::Decoder | Role::Network | Role::Core | Role::Updater { .. }) {
        policy |= mitigation::WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON;
    }
    let policy = [policy, 0_u64];
    add_attribute(&attributes, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY as usize, &policy, size_of_val(&policy))?;
    let no_children: u32 = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
    add_attribute(&attributes, PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY as usize, &no_children, size_of::<u32>())?;
    let capabilities: Vec<SID_AND_ATTRIBUTES>;
    let mut security = SECURITY_CAPABILITIES { AppContainerSid: ptr::null_mut(), Capabilities: ptr::null_mut(), CapabilityCount: 0, Reserved: 0 };
    let opt_out: u32 = PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;
    if let Some((sid, capability_sids)) = container {
        // Enabled is what a capability carries.
        let enabled = u32::try_from(SE_GROUP_ENABLED).unwrap_or(0);
        capabilities = capability_sids.iter().map(|sid| SID_AND_ATTRIBUTES { Sid: sid.0, Attributes: enabled }).collect();
        security.AppContainerSid = sid.0;
        security.Capabilities = capabilities.as_ptr().cast_mut();
        security.CapabilityCount = u32::try_from(capabilities.len()).unwrap_or(0);
        add_attribute(&attributes, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize, &security, size_of::<SECURITY_CAPABILITIES>())?;
        if matches!(spawn.role, Role::Decoder | Role::Core) {
            add_attribute(&attributes, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY as usize, &opt_out, size_of::<u32>())?;
        }
    }
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap_or(0);
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin;
    startup.StartupInfo.hStdOutput = stdout;
    startup.StartupInfo.hStdError = inherited.get(2).copied().unwrap_or(0);
    startup.lpAttributeList = attributes.list;
    let program = wide(spawn.program);
    let mut line = command_line(spawn.program.as_os_str(), spawn.arguments);
    let mut environment = environment_block(spawn.environment);
    let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let started = if container.is_some() {
        unsafe {
            CreateProcessW(
                program.as_ptr(),
                line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                flags,
                environment.as_mut_ptr().cast(),
                ptr::null(),
                &raw const startup.StartupInfo,
                &raw mut information,
            )
        }
    } else {
        let token = restricted_token()?;
        unsafe {
            CreateProcessAsUserW(
                token.0,
                program.as_ptr(),
                line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                flags,
                environment.as_mut_ptr().cast(),
                ptr::null(),
                &raw const startup.StartupInfo,
                &raw mut information,
            )
        }
    };
    if started == 0 {
        return Err(last_error());
    }
    Ok(information)
}

/// Grants the container `sid` full access to `folder` and everything in
/// it: the trusted core's settings (decision 56), the updater's staging
/// folder.
fn grant(folder: &Path, sid: &ContainerSid) -> io::Result<()> {
    grant_to(folder, sid.0, GENERIC_ALL)
}

/// Grants `ALL APPLICATION PACKAGES` read and run on `folder` and
/// everything in it (`installation-and-updates.md` section 4.2).
pub fn grant_packages_read(folder: &Path) -> io::Result<()> {
    let packages = Sid::parse(ALL_APPLICATION_PACKAGES_SID)?;
    grant_to(folder, packages.0, GENERIC_READ_EXECUTE)
}

/// Adds to `folder`'s access list an entry granting `sid` `permissions`,
/// inherited by everything in it.
fn grant_to(folder: &Path, sid: PSID, permissions: u32) -> io::Result<()> {
    let path = wide(folder);
    let mut old: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
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
    let merged = unsafe { SetEntriesInAclW(1, &raw const access, old, &raw mut new) };
    let written = if merged == 0 {
        unsafe {
            SetNamedSecurityInfoW(path.as_ptr().cast_mut(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, ptr::null_mut(), ptr::null_mut(), new, ptr::null())
        }
    } else {
        merged
    };
    unsafe {
        LocalFree(new.cast());
        LocalFree(descriptor.cast());
    }
    if written != 0 { Err(io::Error::from_raw_os_error(i32::try_from(written).unwrap_or(0))) } else { Ok(()) }
}

/// Duplicates `file`'s handle into the process `into` and returns its
/// value there, for the broker to send the core (decision 56).
pub(crate) fn duplicate_into(file: &File, into: HANDLE) -> io::Result<u64> {
    use std::os::windows::io::AsRawHandle as _;
    let mut theirs: HANDLE = 0;
    if unsafe { DuplicateHandle(GetCurrentProcess(), file.as_raw_handle() as HANDLE, into, &raw mut theirs, 0, 0, DUPLICATE_SAME_ACCESS) } == 0 {
        return Err(last_error());
    }
    u64::try_from(theirs).map_err(|_| io::ErrorKind::InvalidData.into())
}

/// Seconds in a minute: Windows gives a zone's bias in minutes.
const SECONDS_PER_MINUTE: i32 = 60;

/// The local time zone's offset from UTC now, in seconds: the negated bias
/// `GetTimeZoneInformation` gives, with the daylight or standard bias in
/// force now added to it.
pub fn local_offset() -> io::Result<i32> {
    // SAFETY: an all-zero TIME_ZONE_INFORMATION is a valid value of a plain
    // C struct, and the call writes into the one it is given.
    let mut information: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
    let zone = unsafe { GetTimeZoneInformation(&raw mut information) };
    let bias = match zone {
        TIME_ZONE_ID_INVALID => return Err(last_error()),
        TIME_ZONE_ID_DAYLIGHT => information.Bias.saturating_add(information.DaylightBias),
        TIME_ZONE_ID_STANDARD => information.Bias.saturating_add(information.StandardBias),
        _ => information.Bias,
    };
    Ok(bias.saturating_mul(SECONDS_PER_MINUTE).saturating_neg())
}

/// The file of a handle value the broker duplicated into this process.
pub(crate) fn file_from_handle_value(value: u64) -> io::Result<File> {
    let handle = HANDLE::try_from(value).map_err(|_| io::ErrorKind::InvalidData)?;
    if handle == 0 || handle == INVALID_HANDLE_VALUE {
        return Err(io::ErrorKind::InvalidData.into());
    }
    // The broker made this handle for this process and sent its value
    // once; it is ours alone to own.
    Ok(unsafe { File::from_raw_handle(handle as _) })
}

/// Starts a helper confined as its role says.
pub fn spawn(spawn: &Spawn<'_>) -> io::Result<Process> {
    let (their_stdin, our_stdin) = pipe()?;
    let (our_stdout, their_stdout) = pipe()?;
    keep_ours(our_stdin.0)?;
    keep_ours(our_stdout.0)?;
    let stderr = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    let mut inherited = vec![their_stdin.0, their_stdout.0];
    if stderr != 0 && stderr != INVALID_HANDLE_VALUE {
        inherited.push(stderr);
    }
    let job = job(spawn.role)?;
    let mut report = Report { layers: vec![String::from("job object"), String::from("mitigation policies"), String::from("no child processes")] };
    let information = if spawn.role == Role::Interface {
        // The interface's sandbox by design, not a fallback: it needs the
        // desktop, which a container cannot reach.
        let information = start(spawn, &inherited, their_stdin.0, their_stdout.0, None)?;
        report.layers.push(String::from("restricted token at low integrity"));
        information
    } else {
        // Every other role runs in its container or not at all
        // (decision 58).
        let sid = container(spawn.role).map_err(|error| io::Error::new(error.kind(), format!("cannot make its AppContainer: {error}")))?;
        let mut capabilities = Vec::new();
        // A container cannot be told a port: the updater's host is pinned by
        // TLS (`installation-and-updates.md` section 8.1).
        if matches!(spawn.role, Role::Network | Role::Updater { .. }) {
            capabilities.push(Sid::parse(INTERNET_CLIENT_SID)?);
            if spawn.local_network {
                capabilities.push(Sid::parse(PRIVATE_NETWORK_SID)?);
            }
        }
        if matches!(spawn.role, Role::Core | Role::Updater { .. })
            && let Some(folder) = spawn.writable
        {
            grant(folder, &sid)?;
        }
        let information = start(spawn, &inherited, their_stdin.0, their_stdout.0, Some((&sid, &capabilities)))
            .map_err(|error| io::Error::new(error.kind(), format!("cannot start in its AppContainer: {error}")))?;
        report.layers.push(String::from(if matches!(spawn.role, Role::Decoder | Role::Core) { "less privileged AppContainer" } else { "AppContainer" }));
        information
    };
    let process = Owned(information.hProcess);
    let thread = Owned(information.hThread);
    // Into the job before it runs a single instruction.
    if unsafe { AssignProcessToJobObject(job.0, process.0) } == 0 {
        let error = last_error();
        unsafe { TerminateProcess(process.0, 1) };
        return Err(error);
    }
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        let error = last_error();
        unsafe { TerminateProcess(process.0, 1) };
        return Err(error);
    }
    drop(thread);
    drop(their_stdin);
    drop(their_stdout);
    let stdin = unsafe { File::from_raw_handle(our_stdin.0 as _) };
    let stdout = unsafe { File::from_raw_handle(our_stdout.0 as _) };
    std::mem::forget(our_stdin);
    std::mem::forget(our_stdout);
    let inner = Inner { process: process.0, job: job.0 };
    std::mem::forget(process);
    std::mem::forget(job);
    Ok(Process::new(Some(stdin), Some(stdout), inner, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_quoted_as_the_c_runtime_reads_them() {
        let line = command_line(
            OsStr::new(r"C:\Program Files\NeoTeletext\neoteletext-fetcher.exe"),
            &[OsString::from("http://a.example/x y"), OsString::from(r#"say "hi" \"#)],
        );
        let text = String::from_utf16_lossy(&line[..line.len().saturating_sub(1)]);
        assert_eq!(text, r#""C:\Program Files\NeoTeletext\neoteletext-fetcher.exe" "http://a.example/x y" "say \"hi\" \\""#);
    }

    #[test]
    fn the_environment_block_ends_with_two_zeros() {
        let block = environment_block(&[(OsString::from("A"), OsString::from("1"))]);
        assert_eq!(String::from_utf16_lossy(&block[..3]), "A=1");
        assert_eq!(&block[3..], &[0, 0]);
    }
}
