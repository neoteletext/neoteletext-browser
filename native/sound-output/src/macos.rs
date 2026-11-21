//! macOS: Apple's default output audio unit, the output unit that follows
//! the system's chosen output device (`AudioToolbox`, `AUComponent.h`:
//! `kAudioUnitType_Output`, `kAudioUnitSubType_DefaultOutput`), given the
//! stream's format on its input (`kAudioUnitProperty_StreamFormat`, the
//! input scope of element 0), which it converts to the device's own, and a
//! render callback (`kAudioUnitProperty_SetRenderCallback`) that Core
//! Audio's real-time thread calls for each buffer. The buffer's size is
//! the device's I/O buffer for this process (`AudioHardware.h`,
//! `kAudioDevicePropertyBufferFrameSize`, within
//! `kAudioDevicePropertyBufferFrameSizeRange`). A device that goes away
//! (`kAudioDevicePropertyDeviceIsAlive`) is told as trouble, as it was
//! with `cpal`.
//!
//! The declarations below are those of the macOS SDK's headers
//! `AudioToolbox/AudioComponent.h`, `AUComponent.h`,
//! `AudioUnitProperties.h`, `AudioOutputUnit.h`,
//! `CoreAudio/AudioHardware.h`, `AudioHardwareBase.h` and
//! `CoreAudioTypes/CoreAudioBaseTypes.h`; the sizes the headers imply are
//! asserted.
#![allow(unsafe_code, reason = "Core Audio's interface is C functions: each call is a foreign function, with its reason at each block")]

use std::ffi::c_void;
use std::sync::{Mutex, PoisonError};

use crate::{CHANNELS, Described, Error, Fill, Granted, Size, Trouble};

/// `OSStatus`, `OSType`, `AudioObjectID` (`MacTypes.h`, `AudioHardwareBase.h`).
type OsStatus = i32;
type OsType = u32;
type AudioObjectId = u32;

/// `AudioComponent` and `AudioComponentInstance` (an `AudioUnit`): opaque.
#[repr(C)]
struct OpaqueAudioComponent {
    _opaque: [u8; 0],
}
#[repr(C)]
struct ComponentInstanceRecord {
    _opaque: [u8; 0],
}
type AudioComponent = *mut OpaqueAudioComponent;
type AudioUnit = *mut ComponentInstanceRecord;

/// `AudioComponentDescription` (`AudioComponent.h`, packed to 4 bytes).
#[repr(C, packed(4))]
struct AudioComponentDescription {
    kind: OsType,
    sub_kind: OsType,
    manufacturer: OsType,
    flags: u32,
    flags_mask: u32,
}

/// `AudioStreamBasicDescription` (`CoreAudioBaseTypes.h`).
#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

/// `AudioBuffer` and `AudioBufferList`, whose array has
/// `mNumberBuffers` elements (`CoreAudioBaseTypes.h`).
#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}
#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

/// `AURenderCallback` and `AURenderCallbackStruct` (`AUComponent.h`,
/// `AudioUnitProperties.h`). The action flags and the time stamp are not
/// read.
type AuRenderCallback = unsafe extern "C" fn(
    ref_con: *mut c_void,
    action_flags: *mut u32,
    time_stamp: *const c_void,
    bus: u32,
    frames: u32,
    data: *mut AudioBufferList,
) -> OsStatus;
#[repr(C)]
struct AuRenderCallbackStruct {
    input_proc: Option<AuRenderCallback>,
    input_proc_ref_con: *mut c_void,
}

/// `AudioObjectPropertyAddress` and `AudioValueRange`
/// (`AudioHardwareBase.h`, `CoreAudioBaseTypes.h`).
#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}
#[repr(C)]
struct AudioValueRange {
    minimum: f64,
    maximum: f64,
}

/// `AudioObjectPropertyListenerProc` (`AudioHardware.h`).
type AudioObjectPropertyListenerProc =
    unsafe extern "C" fn(object: AudioObjectId, count: u32, addresses: *const AudioObjectPropertyAddress, client_data: *mut c_void) -> OsStatus;

// The sizes the headers' declarations have on every macOS target.
const _: () = assert!(size_of::<AudioComponentDescription>() == 20);
const _: () = assert!(size_of::<AudioStreamBasicDescription>() == 40);
const _: () = assert!(size_of::<AudioBuffer>() == 16);
const _: () = assert!(size_of::<AudioObjectPropertyAddress>() == 12);
const _: () = assert!(size_of::<AudioValueRange>() == 16);

/// A four-character code, as the headers write them (`'auou'`).
const fn four(code: [u8; 4]) -> u32 {
    u32::from_be_bytes(code)
}

/// `kAudioUnitType_Output`, `kAudioUnitSubType_DefaultOutput` and
/// `kAudioUnitManufacturer_Apple` (`AUComponent.h`).
const UNIT_TYPE_OUTPUT: OsType = four(*b"auou");
const UNIT_SUB_TYPE_DEFAULT_OUTPUT: OsType = four(*b"def ");
const UNIT_MANUFACTURER_APPLE: OsType = four(*b"appl");
/// `kAudioUnitScope_Global` and `kAudioUnitScope_Input`
/// (`AudioUnitProperties.h`).
const SCOPE_GLOBAL: u32 = 0;
const SCOPE_INPUT: u32 = 1;
/// The output unit's one element.
const ELEMENT_OUTPUT: u32 = 0;
/// `kAudioUnitProperty_StreamFormat`, `kAudioUnitProperty_SetRenderCallback`
/// and `kAudioOutputUnitProperty_CurrentDevice` (`AudioUnitProperties.h`).
const PROPERTY_STREAM_FORMAT: u32 = 8;
const PROPERTY_SET_RENDER_CALLBACK: u32 = 23;
const PROPERTY_CURRENT_DEVICE: u32 = 2000;
/// `kAudioFormatLinearPCM`, `kAudioFormatFlagIsSignedInteger` and
/// `kAudioFormatFlagIsPacked`; native byte order is no flag on a
/// little-endian Mac (`kAudioFormatFlagsNativeEndian`,
/// `CoreAudioBaseTypes.h`).
const FORMAT_LINEAR_PCM: u32 = four(*b"lpcm");
const FORMAT_FLAG_SIGNED_INTEGER: u32 = 1 << 2;
const FORMAT_FLAG_PACKED: u32 = 1 << 3;
/// `kAudioDevicePropertyBufferFrameSize`,
/// `kAudioDevicePropertyBufferFrameSizeRange`,
/// `kAudioDevicePropertyDeviceIsAlive`,
/// `kAudioDevicePropertyNominalSampleRate`, `kAudioObjectPropertyScopeGlobal`
/// and `kAudioObjectPropertyElementMain` (`AudioHardware.h`,
/// `AudioHardwareBase.h`).
const DEVICE_BUFFER_FRAME_SIZE: u32 = four(*b"fsiz");
const DEVICE_BUFFER_FRAME_SIZE_RANGE: u32 = four(*b"fsz#");
const DEVICE_IS_ALIVE: u32 = four(*b"livn");
const DEVICE_NOMINAL_SAMPLE_RATE: u32 = four(*b"nsrt");
const OBJECT_SCOPE_GLOBAL: u32 = four(*b"glob");
const OBJECT_ELEMENT_MAIN: u32 = 0;
/// `noErr`.
const NO_ERROR: OsStatus = 0;
/// Bytes in a sample, and in a frame of two.
const BYTES_PER_SAMPLE: u32 = 2;
const BITS_PER_SAMPLE: u32 = 16;

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(component: AudioComponent, description: *const AudioComponentDescription) -> AudioComponent;
    fn AudioComponentInstanceNew(component: AudioComponent, instance: *mut AudioUnit) -> OsStatus;
    fn AudioComponentInstanceDispose(instance: AudioUnit) -> OsStatus;
    fn AudioUnitInitialize(unit: AudioUnit) -> OsStatus;
    fn AudioUnitUninitialize(unit: AudioUnit) -> OsStatus;
    fn AudioUnitGetProperty(unit: AudioUnit, id: u32, scope: u32, element: u32, data: *mut c_void, size: *mut u32) -> OsStatus;
    fn AudioUnitSetProperty(unit: AudioUnit, id: u32, scope: u32, element: u32, data: *const c_void, size: u32) -> OsStatus;
    fn AudioOutputUnitStart(unit: AudioUnit) -> OsStatus;
    fn AudioOutputUnitStop(unit: AudioUnit) -> OsStatus;
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
    fn AudioObjectSetPropertyData(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OsStatus;
    fn AudioObjectAddPropertyListener(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;
    fn AudioObjectRemovePropertyListener(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;
}

/// A call's status, as an error naming what was called.
fn checked(status: OsStatus, what: &str) -> Result<(), Error> {
    if status == NO_ERROR { Ok(()) } else { Err(Error::new(format!("{what}: Core Audio status {status}"))) }
}

/// The size of `T` as the calls take it.
fn size_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

/// A global property of the device.
const fn device_property(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { selector, scope: OBJECT_SCOPE_GLOBAL, element: OBJECT_ELEMENT_MAIN }
}

/// One of a device's properties, of type `T`.
fn device_get<T>(device: AudioObjectId, selector: u32, value: &mut T, what: &str) -> Result<(), Error> {
    let address = device_property(selector);
    let mut size = size_u32::<T>();
    // SAFETY: the address and the size live until the call returns, and
    // `value` has room for `size` bytes of the property's type, which the
    // header gives for each selector this is called with.
    checked(unsafe { AudioObjectGetPropertyData(device, &raw const address, 0, std::ptr::null(), &raw mut size, std::ptr::from_mut(value).cast()) }, what)
}

/// A default output unit, disposed of when dropped.
struct Unit(AudioUnit);

// SAFETY: an audio unit's properties may be set and it may be started and
// stopped from any thread (`AudioOutputUnit.h`); the stream's owner alone
// does so, one call at a time.
unsafe impl Send for Unit {}

impl Unit {
    fn new() -> Result<Self, Error> {
        let description = AudioComponentDescription {
            kind: UNIT_TYPE_OUTPUT,
            sub_kind: UNIT_SUB_TYPE_DEFAULT_OUTPUT,
            manufacturer: UNIT_MANUFACTURER_APPLE,
            flags: 0,
            flags_mask: 0,
        };
        // SAFETY: the description lives until the call returns; a null
        // component starts the search at the first.
        let component = unsafe { AudioComponentFindNext(std::ptr::null_mut(), &raw const description) };
        if component.is_null() {
            return Err(Error::new("no default output unit"));
        }
        let mut unit = std::ptr::null_mut();
        // SAFETY: the component was found above; `unit` is a valid place
        // for the instance.
        checked(unsafe { AudioComponentInstanceNew(component, &raw mut unit) }, "the default output unit")?;
        if unit.is_null() {
            return Err(Error::new("the default output unit gave no instance"));
        }
        Ok(Self(unit))
    }

    /// The device the unit plays to now.
    fn device(&self) -> Result<AudioObjectId, Error> {
        let mut device: AudioObjectId = 0;
        let mut size = size_u32::<AudioObjectId>();
        checked(
            // SAFETY: the unit is live; `device` has room for the
            // property's `AudioDeviceID`.
            unsafe {
                AudioUnitGetProperty(self.0, PROPERTY_CURRENT_DEVICE, SCOPE_GLOBAL, ELEMENT_OUTPUT, std::ptr::from_mut(&mut device).cast(), &raw mut size)
            },
            "no audio output device",
        )?;
        Ok(device)
    }

    fn set<T>(&self, id: u32, scope: u32, value: &T, what: &str) -> Result<(), Error> {
        // SAFETY: the unit is live and `value` holds the property's type,
        // which the header gives for each property this is called with,
        // for the length of the call.
        checked(unsafe { AudioUnitSetProperty(self.0, id, scope, ELEMENT_OUTPUT, std::ptr::from_ref(value).cast(), size_u32::<T>()) }, what)
    }
}

impl Drop for Unit {
    fn drop(&mut self) {
        // SAFETY: the unit is live and disposed of once; uninitialising one
        // never initialised does nothing.
        unsafe {
            AudioOutputUnitStop(self.0);
            AudioUnitUninitialize(self.0);
            AudioComponentInstanceDispose(self.0);
        }
    }
}

/// A rate in hertz from the device's floating-point one, refused when it
/// is not a whole number of hertz within the bounds.
fn rate_of(rate: f64) -> Result<u32, Error> {
    let bounds = f64::from(crate::MIN_RATE_HZ)..=f64::from(crate::MAX_RATE_HZ);
    if !bounds.contains(&rate) || rate.fract() != 0.0 {
        return Err(Error::new(format!("the output device's rate of {rate} Hz")));
    }
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a whole number within the bounds, checked just above")]
    Ok(rate as u32)
}

/// A frame count from the device's floating-point range.
fn frames_of(frames: f64) -> u32 {
    if frames.is_nan() || frames <= 0.0 {
        return 0;
    }
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "not negative, and held to the bound")]
    let frames = frames.min(f64::from(crate::MAX_BUFFER_FRAMES)) as u32;
    frames
}

/// Describes the device the default output unit plays to: its nominal
/// rate, and the range of I/O buffer sizes it grants.
pub fn describe() -> Result<Described, Error> {
    let unit = Unit::new()?;
    let device = unit.device()?;
    let mut rate: f64 = 0.0;
    device_get(device, DEVICE_NOMINAL_SAMPLE_RATE, &mut rate, "the output device's rate")?;
    let mut range = AudioValueRange { minimum: 0.0, maximum: 0.0 };
    let granted = match device_get(device, DEVICE_BUFFER_FRAME_SIZE_RANGE, &mut range, "the output device's buffer sizes") {
        Ok(()) => Granted::Range(frames_of(range.minimum), frames_of(range.maximum)),
        Err(_) => Granted::Own,
    };
    Ok(Described { rate_hz: rate_of(rate)?, granted })
}

/// What the callbacks hold, boxed so that its address stays put while
/// the unit and the listener may call them.
struct Callbacks {
    fill: Mutex<Fill>,
    trouble: Mutex<Trouble>,
}

/// The render callback: fills the unit's one interleaved buffer.
unsafe extern "C" fn render(ref_con: *mut c_void, _: *mut u32, _: *const c_void, _: u32, frames: u32, data: *mut AudioBufferList) -> OsStatus {
    if ref_con.is_null() || data.is_null() {
        return NO_ERROR;
    }
    // SAFETY: `ref_con` is the `Callbacks` the stream boxed and gave with
    // the callback, which outlives the unit; `data` is the unit's buffer
    // list, whose first buffer holds the input format given to the unit:
    // `frames` interleaved frames of two 16-bit samples, aligned for them.
    unsafe {
        let callbacks = &*ref_con.cast::<Callbacks>();
        let list = &mut *data;
        if list.number_buffers == 0 || list.buffers[0].data.is_null() || list.buffers[0].data.align_offset(align_of::<i16>()) != 0 {
            return NO_ERROR;
        }
        let buffer = &mut list.buffers[0];
        let held = usize::try_from(buffer.data_byte_size / BYTES_PER_SAMPLE).unwrap_or(0);
        let samples = usize::try_from(frames).unwrap_or(0).saturating_mul(CHANNELS).min(held);
        let samples = samples - samples % CHANNELS;
        let slice = std::slice::from_raw_parts_mut(buffer.data.cast::<i16>(), samples);
        let mut fill = callbacks.fill.lock().unwrap_or_else(PoisonError::into_inner);
        fill(slice);
    }
    NO_ERROR
}

/// The device listener: tells a device that is no longer alive.
unsafe extern "C" fn listen(device: AudioObjectId, _: u32, _: *const AudioObjectPropertyAddress, client_data: *mut c_void) -> OsStatus {
    if client_data.is_null() {
        return NO_ERROR;
    }
    // SAFETY: `client_data` is the stream's boxed `Callbacks`, which
    // outlives the listener's registration.
    let callbacks = unsafe { &*client_data.cast::<Callbacks>() };
    let mut alive: u32 = 1;
    if device_get(device, DEVICE_IS_ALIVE, &mut alive, "whether the device is alive").is_ok() && alive == 0 {
        let mut trouble = callbacks.trouble.lock().unwrap_or_else(PoisonError::into_inner);
        trouble("the audio output device went away");
    }
    NO_ERROR
}

/// The stream: the unit, its device's listener and the callbacks both
/// call.
pub struct Stream {
    unit: Unit,
    device: AudioObjectId,
    callbacks: Box<Callbacks>,
}

impl Stream {
    /// Opens the default output unit with the stream's format at the rate
    /// described and the buffer size asked, stopped.
    pub fn open(described: &Described, size: Size, fill: Fill, trouble: Trouble) -> Result<Self, Error> {
        let unit = Unit::new()?;
        let device = unit.device()?;
        let channels = u32::try_from(CHANNELS).map_err(|_| Error::new("channels"))?;
        let format = AudioStreamBasicDescription {
            sample_rate: f64::from(described.rate_hz),
            format_id: FORMAT_LINEAR_PCM,
            format_flags: FORMAT_FLAG_SIGNED_INTEGER | FORMAT_FLAG_PACKED,
            bytes_per_packet: BYTES_PER_SAMPLE * channels,
            frames_per_packet: 1,
            bytes_per_frame: BYTES_PER_SAMPLE * channels,
            channels_per_frame: channels,
            bits_per_channel: BITS_PER_SAMPLE,
            reserved: 0,
        };
        unit.set(PROPERTY_STREAM_FORMAT, SCOPE_INPUT, &format, "the output's format")?;
        if let Size::Frames(frames) = size {
            let address = device_property(DEVICE_BUFFER_FRAME_SIZE);
            checked(
                // SAFETY: the address and the frame count, the property's
                // `UInt32`, live until the call returns.
                unsafe { AudioObjectSetPropertyData(device, &raw const address, 0, std::ptr::null(), size_u32::<u32>(), std::ptr::from_ref(&frames).cast()) },
                "the output's buffer size",
            )?;
        }
        let callbacks = Box::new(Callbacks { fill: Mutex::new(fill), trouble: Mutex::new(trouble) });
        let context = std::ptr::from_ref::<Callbacks>(&callbacks).cast_mut().cast::<c_void>();
        let callback = AuRenderCallbackStruct { input_proc: Some(render), input_proc_ref_con: context };
        unit.set(PROPERTY_SET_RENDER_CALLBACK, SCOPE_INPUT, &callback, "the output's callback")?;
        // SAFETY: the unit is live and configured.
        checked(unsafe { AudioUnitInitialize(unit.0) }, "cannot open the audio output")?;
        let address = device_property(DEVICE_IS_ALIVE);
        // SAFETY: the address lives until the call returns; `context` is
        // the boxed callbacks, which the stream removes the listener from
        // before it frees them.
        checked(unsafe { AudioObjectAddPropertyListener(device, &raw const address, listen, context) }, "watching the output device")?;
        Ok(Self { unit, device, callbacks })
    }

    /// Starts the unit.
    pub fn play(&mut self) -> Result<(), Error> {
        // SAFETY: the unit is live and initialised.
        checked(unsafe { AudioOutputUnitStart(self.unit.0) }, "starting the output")
    }

    /// Stops it.
    pub fn pause(&mut self) -> Result<(), Error> {
        // SAFETY: as above.
        checked(unsafe { AudioOutputUnitStop(self.unit.0) }, "stopping the output")
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let address = device_property(DEVICE_IS_ALIVE);
        let context = std::ptr::from_ref::<Callbacks>(&self.callbacks).cast_mut().cast::<c_void>();
        // SAFETY: the listener was added with this address, function and
        // context; it is removed before the callbacks it reads are freed.
        // The unit stops and is disposed of when `unit` drops, before
        // `callbacks`, as the fields are declared.
        unsafe { AudioObjectRemovePropertyListener(self.device, &raw const address, listen, context) };
    }
}
