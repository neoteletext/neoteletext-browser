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
//! - **An `AppContainer`** for the decoder (no capabilities, and opted out
//!   of the all-application-packages permission: a less privileged
//!   `AppContainer`) and the fetcher (the internet client capability, and
//!   the private network one when the user allowed local addresses).
//!   The helper's program must grant read to `ALL APPLICATION PACKAGES`,
//!   which an installation under Program Files does and a folder of the
//!   user's does not; when the `AppContainer` start fails, the helper is
//!   started with the restricted token alone and the report says so.
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
use std::process::ExitStatus;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree, PSID, SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, FreeSid, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES, SetTokenInformation,
    TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
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
use windows_sys::Win32::System::SystemServices::{SE_GROUP_INTEGRITY, SE_GROUP_USE_FOR_DENY_ONLY};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetCurrentProcess, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
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
/// How many attributes a start carries at most.
const ATTRIBUTES: u32 = 5;

/// A helper's process and its job.
#[derive(Debug)]
pub struct Inner {
    process: HANDLE,
    job: HANDLE,
}

impl Inner {
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
    let name = wide(if role == Role::Decoder { CONTAINER_DECODER } else { CONTAINER_NETWORK });
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
    if spawn.role != Role::Interface {
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
        // SE_GROUP_ENABLED is what a capability carries.
        capabilities = capability_sids.iter().map(|sid| SID_AND_ATTRIBUTES { Sid: sid.0, Attributes: 0x4 }).collect();
        security.AppContainerSid = sid.0;
        security.Capabilities = capabilities.as_ptr().cast_mut();
        security.CapabilityCount = u32::try_from(capabilities.len()).unwrap_or(0);
        add_attribute(&attributes, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize, &security, size_of::<SECURITY_CAPABILITIES>())?;
        if spawn.role == Role::Decoder {
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
    let mut report =
        Report { layers: vec![String::from("job object"), String::from("mitigation policies"), String::from("no child processes")], missing: Vec::new() };
    let mut started = None;
    // NEOTELETEXT_CONFINE_NO_CONTAINER makes a helper start as it would on
    // a folder the container may not read, for checking that path.
    let no_container = std::env::var_os("NEOTELETEXT_CONFINE_NO_CONTAINER").is_some();
    if spawn.role != Role::Interface && !no_container {
        match container(spawn.role) {
            Ok(sid) => {
                let mut capabilities = Vec::new();
                if spawn.role == Role::Network {
                    capabilities.push(Sid::parse(INTERNET_CLIENT_SID)?);
                    if spawn.local_network {
                        capabilities.push(Sid::parse(PRIVATE_NETWORK_SID)?);
                    }
                }
                match start(spawn, &inherited, their_stdin.0, their_stdout.0, Some((&sid, &capabilities))) {
                    Ok(information) => {
                        report.layers.push(String::from(if spawn.role == Role::Decoder { "less privileged AppContainer" } else { "AppContainer" }));
                        started = Some(information);
                    }
                    Err(error) => report.missing.push(format!("an AppContainer ({error}; the program folder must grant read to ALL APPLICATION PACKAGES)")),
                }
            }
            Err(error) => report.missing.push(format!("an AppContainer ({error})")),
        }
    }
    let information = if let Some(information) = started {
        information
    } else {
        let information = start(spawn, &inherited, their_stdin.0, their_stdout.0, None)?;
        report.layers.push(String::from("restricted token at low integrity"));
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
    Ok(Process { stdin: Some(stdin), stdout: Some(stdout), inner, report })
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
