//! The shell's Common Item Dialog (`shobjidl_core.h`: `IFileOpenDialog`,
//! `IFileSaveDialog`, both `IFileDialog`s, and `IShellItem`), through COM.
//! `windows-sys` gives the functions, constants and class identifiers but
//! not COM's interfaces, so their tables of methods are laid out here in
//! the header's order, every method before the last one used.
//!
//! COM is reached only through foreign calls, so this module is allowed
//! unsafe code; every block says why it is sound.
#![expect(unsafe_code, reason = "the shell's dialogs are reached only through COM's foreign calls (DEC-7168, requirement 10.12)")]

use std::ffi::c_void;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::UI::Shell::{FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, SHCreateItemFromParsingName, SIGDN_FILESYSPATH};
use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};

use crate::{Dialog, Kind, shell_path, windows_pattern};

/// `shobjidl_core.h`: the interfaces' identifiers.
const IID_FILE_OPEN_DIALOG: GUID = GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);
const IID_FILE_SAVE_DIALOG: GUID = GUID::from_u128(0x84bccd23_5fde_4cdb_aea4_af64b83d78ab);
const IID_SHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
/// `winerror.h`: `HRESULT_FROM_WIN32(ERROR_CANCELLED)`, what `Show`
/// returns when the person cancels: the error's low 16 bits, the Win32
/// facility (7) and the failure bit.
const CANCELLED: HRESULT = (0x8007_0000_u32 | (ERROR_CANCELLED & 0xFFFF)).cast_signed();
/// The longest path read back, in UTF-16 units: a verbatim path's limit
/// (Microsoft's "Maximum Path Length Limitation", about 32,767 characters).
const MAX_PATH_UNITS: usize = 32_767;
/// The filter when none is asked for, as the dialog had before.
const ALL_FILES: (&str, &str) = ("All Files", "*.*");

/// `shtypes.h`: `COMDLG_FILTERSPEC`.
#[repr(C)]
struct FilterSpec {
    name: PCWSTR,
    spec: PCWSTR,
}

/// `unknwn.h`: `IUnknown`'s methods.
#[repr(C)]
struct UnknownMethods {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
}

/// `shobjidl_core.h`: `IModalWindow`'s and then `IFileDialog`'s methods,
/// in order, to `SetDefaultExtension`.
#[repr(C)]
struct FileDialogMethods {
    unknown: UnknownMethods,
    show: unsafe extern "system" fn(this: *mut c_void, owner: HWND) -> HRESULT,
    set_file_types: unsafe extern "system" fn(this: *mut c_void, count: u32, specs: *const FilterSpec) -> HRESULT,
    set_file_type_index: usize,
    get_file_type_index: usize,
    advise: usize,
    unadvise: usize,
    set_options: unsafe extern "system" fn(this: *mut c_void, options: u32) -> HRESULT,
    get_options: unsafe extern "system" fn(this: *mut c_void, options: *mut u32) -> HRESULT,
    set_default_folder: usize,
    set_folder: unsafe extern "system" fn(this: *mut c_void, item: *mut c_void) -> HRESULT,
    get_folder: usize,
    get_current_selection: usize,
    set_file_name: unsafe extern "system" fn(this: *mut c_void, name: PCWSTR) -> HRESULT,
    get_file_name: usize,
    set_title: unsafe extern "system" fn(this: *mut c_void, title: PCWSTR) -> HRESULT,
    set_ok_button_label: usize,
    set_file_name_label: usize,
    get_result: unsafe extern "system" fn(this: *mut c_void, item: *mut *mut c_void) -> HRESULT,
    add_place: usize,
    set_default_extension: unsafe extern "system" fn(this: *mut c_void, extension: PCWSTR) -> HRESULT,
}

/// `shobjidl_core.h`: `IShellItem`'s methods, to `GetDisplayName`.
#[repr(C)]
struct ShellItemMethods {
    unknown: UnknownMethods,
    bind_to_handler: usize,
    get_parent: usize,
    get_display_name: unsafe extern "system" fn(this: *mut c_void, form: i32, name: *mut PWSTR) -> HRESULT,
}

/// A COM object this module holds one reference to, released when dropped.
struct Object(*mut c_void);

impl Object {
    /// Its table of methods, as `T` lays it out.
    ///
    /// # Safety
    ///
    /// The object must implement an interface whose table begins as `T`.
    unsafe fn methods<T>(&self) -> &T {
        // SAFETY: a COM object's first field is a pointer to its table of
        // methods, which lives as long as the object; the caller vouches
        // for the layout.
        unsafe { &**self.0.cast::<*const T>() }
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        // SAFETY: the pointer is a live COM object whose one reference this
        // value holds; every interface begins with IUnknown's methods.
        unsafe { (self.methods::<UnknownMethods>().release)(self.0) };
    }
}

fn check(result: HRESULT, what: &str) -> Result<(), String> {
    if result < 0 { Err(format!("{what} failed (HRESULT {result:#010x})")) } else { Ok(()) }
}

/// `text` as a zero-ended UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn choose(dialog: &Dialog<'_>) -> Result<Option<PathBuf>, String> {
    // SAFETY: COM is started on this thread, single-threaded as the shell's
    // dialogs need, and ended below on the same thread once every object
    // made here has been released.
    check(unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).cast_unsigned()) }, "starting COM")?;
    let chosen = show(dialog);
    // SAFETY: as above; `show` has dropped every object it made.
    unsafe { CoUninitialize() };
    chosen
}

fn show(dialog: &Dialog<'_>) -> Result<Option<PathBuf>, String> {
    let (class, interface) = match dialog.kind {
        Kind::Save(_) => (&FileSaveDialog, &IID_FILE_SAVE_DIALOG),
        Kind::File | Kind::Folder => (&FileOpenDialog, &IID_FILE_OPEN_DIALOG),
    };
    let mut made: *mut c_void = std::ptr::null_mut();
    // SAFETY: the class and interface identifiers are the shell's, `made`
    // a place for one pointer, written on success with an object holding
    // one reference for this function.
    check(unsafe { CoCreateInstance(class, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, interface, &raw mut made) }, "making the dialog")?;
    if made.is_null() {
        return Err(String::from("the shell made no dialog"));
    }
    let object = Object(made);
    // SAFETY: both dialogs' interfaces begin with IFileDialog's table.
    let methods = unsafe { object.methods::<FileDialogMethods>() };
    let this = object.0;
    let title = wide(dialog.title);
    // SAFETY: `title` is a zero-ended UTF-16 string alive for the call.
    check(unsafe { (methods.set_title)(this, title.as_ptr()) }, "setting the title")?;
    if dialog.kind == Kind::Folder {
        let mut options = 0;
        // SAFETY: `options` is a place for one u32, alive for both calls.
        unsafe {
            check((methods.get_options)(this, &raw mut options), "reading the options")?;
            check((methods.set_options)(this, options | FOS_PICKFOLDERS), "choosing folders")?;
        }
    } else {
        let (name, pattern) = match dialog.filter {
            Some((name, endings)) => (name.to_owned(), windows_pattern(endings)),
            None => (ALL_FILES.0.to_owned(), ALL_FILES.1.to_owned()),
        };
        if let Some(first) = dialog.filter.and_then(|(_, endings)| endings.first()) {
            let extension = wide(first);
            // SAFETY: `extension` is a zero-ended UTF-16 string alive for
            // the call.
            check(unsafe { (methods.set_default_extension)(this, extension.as_ptr()) }, "setting the default extension")?;
        }
        let (name, pattern) = (wide(&name), wide(&pattern));
        let spec = FilterSpec { name: name.as_ptr(), spec: pattern.as_ptr() };
        // SAFETY: one filter whose strings are zero-ended and alive for the
        // call, which copies them.
        check(unsafe { (methods.set_file_types)(this, 1, &raw const spec) }, "setting the filter")?;
    }
    if let Some(folder) = dialog.folder.and_then(|folder| folder.to_str()) {
        let path = wide(shell_path(folder));
        let mut item: *mut c_void = std::ptr::null_mut();
        // SAFETY: `path` is zero-ended and alive for the call, `item` a
        // place for one pointer, written on success with a shell item
        // holding one reference, which `Object` releases. A folder the
        // shell cannot name is left out, as the dialog starts elsewhere.
        if unsafe { SHCreateItemFromParsingName(path.as_ptr(), std::ptr::null_mut(), &IID_SHELL_ITEM, &raw mut item) } >= 0 && !item.is_null() {
            let item = Object(item);
            // SAFETY: `item` is a live shell item for the call, which takes
            // its own reference if it keeps it.
            check(unsafe { (methods.set_folder)(this, item.0) }, "setting the folder")?;
        }
    }
    if let Kind::Save(name) = dialog.kind {
        let name = wide(name);
        // SAFETY: `name` is a zero-ended UTF-16 string alive for the call.
        check(unsafe { (methods.set_file_name)(this, name.as_ptr()) }, "setting the file name")?;
    }
    // SAFETY: the owner is a window handle or none (0); a handle that
    // names no window is refused by the system, and the call returns when
    // the person closes the dialog.
    let shown = unsafe { (methods.show)(this, dialog.owner.unwrap_or(0)) };
    if shown == CANCELLED {
        return Ok(None);
    }
    check(shown, "showing the dialog")?;
    let mut item: *mut c_void = std::ptr::null_mut();
    // SAFETY: `item` is a place for one pointer, written on success with a
    // shell item holding one reference, which `Object` releases.
    check(unsafe { (methods.get_result)(this, &raw mut item) }, "reading the choice")?;
    if item.is_null() {
        return Err(String::from("the dialog chose nothing"));
    }
    let item = Object(item);
    path_of(&item).map(Some)
}

/// The file system path of a shell item.
fn path_of(item: &Object) -> Result<PathBuf, String> {
    // SAFETY: the object is a shell item, whose table begins as
    // `ShellItemMethods`.
    let methods = unsafe { item.methods::<ShellItemMethods>() };
    let mut name: PWSTR = std::ptr::null_mut();
    // SAFETY: `name` is a place for one pointer, written on success with a
    // zero-ended string the caller frees with `CoTaskMemFree`, as below.
    check(unsafe { (methods.get_display_name)(item.0, SIGDN_FILESYSPATH, &raw mut name) }, "reading the path")?;
    if name.is_null() {
        return Err(String::from("the dialog's choice has no path"));
    }
    // SAFETY: `name` is a zero-ended UTF-16 string the shell allocated;
    // it is read no further than its zero or the bound, then freed once.
    let units = unsafe {
        let mut length = 0;
        while length < MAX_PATH_UNITS && *name.add(length) != 0 {
            length = length.saturating_add(1);
        }
        let units = std::slice::from_raw_parts(name, length).to_vec();
        CoTaskMemFree(name.cast_const().cast());
        units
    };
    if units.len() >= MAX_PATH_UNITS {
        return Err(String::from("the dialog's choice has a path too long"));
    }
    String::from_utf16(&units).map(PathBuf::from).map_err(|_| String::from("the dialog's choice is not a path the browser can name"))
}
