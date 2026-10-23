//! A confined start: the attribute list (inherited handles, mitigation
//! policies, no child processes, the container and its capabilities) and
//! the `CreateProcess` call that takes it.

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES};
use windows_sys::Win32::System::SystemServices::SE_GROUP_ENABLED;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
    PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
};
use windows_sys::Win32::System::WindowsProgramming::{PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT, PROCESS_CREATION_CHILD_PROCESS_RESTRICTED};

use super::handles::{ContainerSid, Sid, last_error, wide};
use super::process::{command_line, environment_block};
use crate::{Role, Spawn};

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

/// How many attributes a start carries at most.
const ATTRIBUTES: u32 = 5;

/// The attribute list of a start: inherited handles, mitigation policies,
/// no child processes, the container and its capabilities, and for a less
/// privileged container its opt-out of `ALL APPLICATION PACKAGES`.
struct Attributes {
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
    /// The list's room, in 64-bit words, so it is aligned for the pointers
    /// and sizes it holds (finding F-59): bytes have no alignment.
    _storage: Vec<u64>,
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: the list was initialised by `attributes` in storage this value still owns.
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

fn attributes(count: u32) -> io::Result<Attributes> {
    let mut size = 0_usize;
    // The first call says how much room the list needs.
    // SAFETY: a null list asks only for the size, written to a live local; the call fails
    // by design, so its result is not read.
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &raw mut size) };
    let mut storage = vec![0_u64; size.div_ceil(size_of::<u64>())];
    let list: LPPROC_THREAD_ATTRIBUTE_LIST = storage.as_mut_ptr().cast();
    // SAFETY: `list` points at storage of at least `size` bytes, aligned for its fields,
    // which outlives it in `Attributes`.
    if unsafe { InitializeProcThreadAttributeList(list, count, 0, &raw mut size) } == 0 {
        return Err(last_error());
    }
    Ok(Attributes { list, _storage: storage })
}

fn add_attribute<T>(attributes: &Attributes, which: usize, value: &T, bytes: usize) -> io::Result<()> {
    add_attribute_at(attributes, which, ptr::from_ref(value).cast::<std::ffi::c_void>(), bytes)
}

fn add_attribute_at(attributes: &Attributes, which: usize, value: *const std::ffi::c_void, bytes: usize) -> io::Result<()> {
    // SAFETY: the list is initialised; `value` points at `bytes` bytes the caller keeps
    // alive until CreateProcessW has read them (locals of `start`).
    if unsafe { UpdateProcThreadAttribute(attributes.list, 0, which, value.cast_mut(), bytes, ptr::null_mut(), ptr::null()) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// Starts the helper in its container `sid`, with `capabilities`.
pub(super) fn start(
    spawn: &Spawn<'_>,
    inherited: &[HANDLE],
    stdin: HANDLE,
    stdout: HANDLE,
    sid: &ContainerSid,
    capability_sids: &[Sid],
) -> io::Result<PROCESS_INFORMATION> {
    let attributes = attributes(ATTRIBUTES)?;
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
    // Enabled is what a capability carries.
    let enabled = u32::try_from(SE_GROUP_ENABLED).unwrap_or(0);
    let capabilities: Vec<SID_AND_ATTRIBUTES> = capability_sids.iter().map(|sid| SID_AND_ATTRIBUTES { Sid: sid.0, Attributes: enabled }).collect();
    let security = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: capabilities.as_ptr().cast_mut(),
        CapabilityCount: u32::try_from(capabilities.len()).unwrap_or(0),
        Reserved: 0,
    };
    add_attribute(&attributes, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize, &security, size_of::<SECURITY_CAPABILITIES>())?;
    let opt_out: u32 = PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;
    if matches!(spawn.role, Role::Decoder | Role::Core) {
        add_attribute(&attributes, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY as usize, &opt_out, size_of::<u32>())?;
    }
    // SAFETY: an all-zero STARTUPINFOEXW is a valid value of a plain C struct.
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
    // SAFETY: likewise PROCESS_INFORMATION, which the call writes.
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer is to a live local of this function (the program and the
    // mutable command line and environment block, NUL-terminated) or to the
    // startup information and its attribute list, all alive for the call.
    let started = unsafe {
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
    };
    if started == 0 {
        return Err(last_error());
    }
    Ok(information)
}
