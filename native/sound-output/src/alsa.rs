//! Linux: ALSA's PCM interface, through the system's `libasound`, linked
//! dynamically as before (decision 40: the owner accepted its LGPL licence;
//! only the sound helper links it, so a Linux without it runs silent,
//! decision 55). The calls and constants are those of `alsa/pcm.h` and
//! `alsa/error.h` (alsa-lib 1.2), and the order of the configuration is
//! the one its PCM interface documentation gives: hardware parameters
//! (access, format, channels, rate, period, buffer), then software
//! parameters (start threshold, the least space a wait returns for).
//!
//! The stream is the device `default`, which on a desktop is the sound
//! server's plug-in, with a buffer of two
//! periods, the period being the size the helper asks for; it starts once
//! both are written, and is written with `snd_pcm_writei` from a thread of
//! its own. Stopped, it is dropped (`snd_pcm_drop`) and the thread waits
//! for the owner; started again, it is prepared (`snd_pcm_prepare`) and
//! filled anew.
#![allow(unsafe_code, reason = "ALSA's PCM interface is a C library's: each call is a foreign function, with its reason at each block")]

use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::Arc;

use crate::control::{Control, Follower, Wanted};
use crate::{CHANNELS, Described, Error, Fill, Granted, Size, Trouble};

/// `snd_pcm_t`, `snd_pcm_hw_params_t` and `snd_pcm_sw_params_t`: opaque
/// to their users (`alsa/pcm.h`).
#[repr(C)]
struct SndPcm {
    _opaque: [u8; 0],
}
#[repr(C)]
struct SndPcmHwParams {
    _opaque: [u8; 0],
}
#[repr(C)]
struct SndPcmSwParams {
    _opaque: [u8; 0],
}

/// `snd_pcm_uframes_t` and `snd_pcm_sframes_t` (`alsa/pcm.h`).
type Uframes = c_ulong;
type Sframes = c_long;

/// `SND_PCM_STREAM_PLAYBACK` (`alsa/pcm.h`, `snd_pcm_stream_t`).
const STREAM_PLAYBACK: c_int = 0;
/// `SND_PCM_ACCESS_RW_INTERLEAVED` (`alsa/pcm.h`, `snd_pcm_access_t`):
/// the fourth access type, written with `snd_pcm_writei`.
const ACCESS_RW_INTERLEAVED: c_int = 3;
/// `SND_PCM_FORMAT_S16_LE` and `SND_PCM_FORMAT_S16_BE` (`alsa/pcm.h`,
/// `snd_pcm_format_t`): the samples in the machine's own byte order, as
/// `SND_PCM_FORMAT_S16` is defined.
#[cfg(target_endian = "little")]
const FORMAT_S16: c_int = 2;
#[cfg(target_endian = "big")]
const FORMAT_S16: c_int = 3;

#[link(name = "asound")]
unsafe extern "C" {
    fn snd_pcm_open(pcm: *mut *mut SndPcm, name: *const c_char, stream: c_int, mode: c_int) -> c_int;
    fn snd_pcm_close(pcm: *mut SndPcm) -> c_int;
    fn snd_pcm_hw_params(pcm: *mut SndPcm, params: *mut SndPcmHwParams) -> c_int;
    fn snd_pcm_sw_params_current(pcm: *mut SndPcm, params: *mut SndPcmSwParams) -> c_int;
    fn snd_pcm_sw_params(pcm: *mut SndPcm, params: *mut SndPcmSwParams) -> c_int;
    fn snd_pcm_prepare(pcm: *mut SndPcm) -> c_int;
    fn snd_pcm_drop(pcm: *mut SndPcm) -> c_int;
    fn snd_pcm_writei(pcm: *mut SndPcm, buffer: *const c_void, size: Uframes) -> Sframes;
    fn snd_pcm_wait(pcm: *mut SndPcm, timeout: c_int) -> c_int;
    fn snd_pcm_recover(pcm: *mut SndPcm, err: c_int, silent: c_int) -> c_int;
    fn snd_pcm_hw_params_malloc(ptr: *mut *mut SndPcmHwParams) -> c_int;
    fn snd_pcm_hw_params_free(obj: *mut SndPcmHwParams);
    fn snd_pcm_hw_params_any(pcm: *mut SndPcm, params: *mut SndPcmHwParams) -> c_int;
    fn snd_pcm_hw_params_set_access(pcm: *mut SndPcm, params: *mut SndPcmHwParams, access: c_int) -> c_int;
    fn snd_pcm_hw_params_set_format(pcm: *mut SndPcm, params: *mut SndPcmHwParams, format: c_int) -> c_int;
    fn snd_pcm_hw_params_set_channels(pcm: *mut SndPcm, params: *mut SndPcmHwParams, val: c_uint) -> c_int;
    fn snd_pcm_hw_params_set_rate_near(pcm: *mut SndPcm, params: *mut SndPcmHwParams, val: *mut c_uint, dir: *mut c_int) -> c_int;
    fn snd_pcm_hw_params_get_period_size(params: *const SndPcmHwParams, frames: *mut Uframes, dir: *mut c_int) -> c_int;
    fn snd_pcm_hw_params_get_period_size_min(params: *const SndPcmHwParams, frames: *mut Uframes, dir: *mut c_int) -> c_int;
    fn snd_pcm_hw_params_get_period_size_max(params: *const SndPcmHwParams, frames: *mut Uframes, dir: *mut c_int) -> c_int;
    fn snd_pcm_hw_params_get_buffer_size_max(params: *const SndPcmHwParams, val: *mut Uframes) -> c_int;
    fn snd_pcm_hw_params_set_period_size_near(pcm: *mut SndPcm, params: *mut SndPcmHwParams, val: *mut Uframes, dir: *mut c_int) -> c_int;
    fn snd_pcm_hw_params_set_buffer_size_near(pcm: *mut SndPcm, params: *mut SndPcmHwParams, val: *mut Uframes) -> c_int;
    fn snd_pcm_sw_params_malloc(ptr: *mut *mut SndPcmSwParams) -> c_int;
    fn snd_pcm_sw_params_free(obj: *mut SndPcmSwParams);
    fn snd_pcm_sw_params_set_start_threshold(pcm: *mut SndPcm, params: *mut SndPcmSwParams, val: Uframes) -> c_int;
    fn snd_pcm_sw_params_set_avail_min(pcm: *mut SndPcm, params: *mut SndPcmSwParams, val: Uframes) -> c_int;
    fn snd_strerror(errnum: c_int) -> *const c_char;
}

/// The device every stream is opened on: the system's default, the sound
/// server's plug-in on a desktop.
const DEVICE: &CStr = c"default";
/// The rate asked for; the device's nearest is taken.
const ASKED_RATE_HZ: c_uint = 48_000;
/// The periods in the device's buffer: two, so one is played while the
/// other is written. The stream starts once both are written.
const PERIODS: Uframes = 2;
/// How long the thread waits for room in the buffer before it says the
/// device is stuck and looks at the owner's wish again, in periods.
const STUCK_PERIODS: u32 = 20;
/// The least wait for room, in milliseconds: a period of a few frames is
/// still given time for the server's scheduling.
const LEAST_WAIT_MS: u32 = 100;
/// Milliseconds in a second.
const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// The text of an ALSA error number.
fn text(code: c_int) -> String {
    // SAFETY: `snd_strerror` returns a pointer to a static, zero-terminated
    // string for any number (`alsa/error.h`), never null.
    let text = unsafe { snd_strerror(code) };
    if text.is_null() {
        return format!("ALSA error {code}");
    }
    // SAFETY: not null, and zero-terminated and static as above.
    unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
}

/// A call's result, as an error naming what was called.
fn checked(code: c_int, what: &str) -> Result<(), Error> {
    if code < 0 { Err(Error::new(format!("{what}: {}", text(code)))) } else { Ok(()) }
}

/// An open PCM, closed when dropped.
struct Pcm(*mut SndPcm);

// SAFETY: an ALSA PCM handle may be used from any thread, one at a time
// (alsa-lib's PCM interface has no thread affinity); it is used by the
// thread that opens it until it is handed to the stream's thread, which
// alone uses it from then on.
unsafe impl Send for Pcm {}

impl Pcm {
    fn open(device: &CStr) -> Result<Self, Error> {
        let mut pcm = std::ptr::null_mut();
        // SAFETY: `pcm` is a valid place for the handle, the name is
        // zero-terminated and outlives the call, and a mode of 0 is
        // blocking.
        checked(unsafe { snd_pcm_open(&raw mut pcm, device.as_ptr(), STREAM_PLAYBACK, 0) }, "opening the default output")?;
        if pcm.is_null() {
            return Err(Error::new("opening the default output gave no handle"));
        }
        Ok(Self(pcm))
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        // SAFETY: the handle is open, and nothing uses it after this.
        unsafe { snd_pcm_close(self.0) };
    }
}

/// Hardware parameters, freed when dropped.
struct HwParams(*mut SndPcmHwParams);

impl HwParams {
    /// The parameters of the stream's format on `pcm`: interleaved signed
    /// 16-bit samples, two channels, and the rate nearest `rate`, which is
    /// returned.
    fn of_format(pcm: &Pcm, rate: c_uint) -> Result<(Self, c_uint), Error> {
        let mut params = std::ptr::null_mut();
        // SAFETY: `params` is a valid place for the allocation's pointer.
        checked(unsafe { snd_pcm_hw_params_malloc(&raw mut params) }, "allocating the parameters")?;
        if params.is_null() {
            return Err(Error::new("allocating the parameters gave nothing"));
        }
        let params = Self(params);
        let channels = c_uint::try_from(CHANNELS).map_err(|_| Error::new("channels"))?;
        let mut rate = rate;
        let mut direction: c_int = 0;
        // SAFETY: the handle is open and the parameters allocated; each
        // call narrows the configuration space it is given, in the order
        // the PCM interface documents.
        unsafe {
            checked(snd_pcm_hw_params_any(pcm.0, params.0), "the output's configurations")?;
            checked(snd_pcm_hw_params_set_access(pcm.0, params.0, ACCESS_RW_INTERLEAVED), "interleaved writing")?;
            checked(snd_pcm_hw_params_set_format(pcm.0, params.0, FORMAT_S16), "16-bit samples")?;
            checked(snd_pcm_hw_params_set_channels(pcm.0, params.0, channels), "two channels")?;
            checked(snd_pcm_hw_params_set_rate_near(pcm.0, params.0, &raw mut rate, &raw mut direction), "the rate")?;
        }
        Ok((params, rate))
    }

    /// The sizes of period the configuration space allows, the most no
    /// more than a buffer of [`PERIODS`] can hold.
    fn period_range(&self) -> Option<(Uframes, Uframes)> {
        let (mut least, mut most, mut buffer_most): (Uframes, Uframes, Uframes) = (0, 0, 0);
        let mut direction: c_int = 0;
        // SAFETY: the parameters are allocated and set; each call writes
        // one value to the place given.
        let read = unsafe {
            snd_pcm_hw_params_get_period_size_min(self.0, &raw mut least, &raw mut direction) >= 0
                && snd_pcm_hw_params_get_period_size_max(self.0, &raw mut most, &raw mut direction) >= 0
                && snd_pcm_hw_params_get_buffer_size_max(self.0, &raw mut buffer_most) >= 0
        };
        // A least of 0 is no lower bound, as PipeWire's plug-in reports.
        let least = least.max(1);
        let most = if buffer_most > 0 { most.min(buffer_most / PERIODS) } else { most };
        (read && most >= least).then_some((least, most))
    }

    /// Sets a period of `frames` and a buffer of [`PERIODS`] of them.
    fn set_period(&self, pcm: &Pcm, frames: Uframes) -> Result<(), Error> {
        let mut period = frames;
        let mut buffer = frames.saturating_mul(PERIODS);
        let mut direction: c_int = 0;
        // SAFETY: the handle is open and the parameters allocated.
        unsafe {
            checked(snd_pcm_hw_params_set_buffer_size_near(pcm.0, self.0, &raw mut buffer), "the buffer's size")?;
            checked(snd_pcm_hw_params_set_period_size_near(pcm.0, self.0, &raw mut period, &raw mut direction), "the period's size")?;
        }
        Ok(())
    }

    /// Applies the parameters to the device, and returns its period.
    fn apply(&self, pcm: &Pcm) -> Result<Uframes, Error> {
        let mut period: Uframes = 0;
        let mut direction: c_int = 0;
        // SAFETY: the handle is open and the parameters allocated; after
        // `snd_pcm_hw_params` they hold the configuration chosen.
        unsafe {
            checked(snd_pcm_hw_params(pcm.0, self.0), "configuring the output")?;
            checked(snd_pcm_hw_params_get_period_size(self.0, &raw mut period, &raw mut direction), "the output's period")?;
        }
        if period == 0 { Err(Error::new("the output chose no period")) } else { Ok(period) }
    }
}

impl Drop for HwParams {
    fn drop(&mut self) {
        // SAFETY: allocated by `snd_pcm_hw_params_malloc` and not freed.
        unsafe { snd_pcm_hw_params_free(self.0) };
    }
}

/// Sets the software parameters: the stream starts when its buffer is
/// full, and a wait returns when a period has room.
fn set_software(pcm: &Pcm, period: Uframes) -> Result<(), Error> {
    let mut params = std::ptr::null_mut();
    // SAFETY: `params` is a valid place for the allocation's pointer.
    checked(unsafe { snd_pcm_sw_params_malloc(&raw mut params) }, "allocating the software parameters")?;
    if params.is_null() {
        return Err(Error::new("allocating the software parameters gave nothing"));
    }
    // SAFETY: the handle is configured and the parameters allocated; they
    // are freed once, after the calls.
    let result = unsafe {
        checked(snd_pcm_sw_params_current(pcm.0, params), "the software parameters")
            .and_then(|()| checked(snd_pcm_sw_params_set_start_threshold(pcm.0, params, period.saturating_mul(PERIODS)), "the start"))
            .and_then(|()| checked(snd_pcm_sw_params_set_avail_min(pcm.0, params, period), "the wait"))
            .and_then(|()| checked(snd_pcm_sw_params(pcm.0, params), "configuring the software parameters"))
    };
    // SAFETY: allocated above and not freed.
    unsafe { snd_pcm_sw_params_free(params) };
    result
}

/// Describes the default output: the rate nearest 48 kHz it grants, and
/// the periods it grants, held to what a buffer of two can hold.
pub fn describe() -> Result<Described, Error> {
    describe_on(DEVICE)
}

fn describe_on(device: &CStr) -> Result<Described, Error> {
    let pcm = Pcm::open(device)?;
    let (params, rate) = HwParams::of_format(&pcm, ASKED_RATE_HZ)?;
    let granted = match params.period_range() {
        Some((least, most)) => Granted::Range(u32::try_from(least).unwrap_or(u32::MAX), u32::try_from(most).unwrap_or(u32::MAX)),
        None => Granted::Own,
    };
    Ok(Described { rate_hz: rate, granted })
}

/// The stream: its thread, which owns the device.
pub struct Stream {
    control: Arc<Control>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Stream {
    /// Opens the default output at the rate described and the size asked,
    /// stopped.
    pub fn open(described: &Described, size: Size, fill: Fill, trouble: Trouble) -> Result<Self, Error> {
        Self::open_on(DEVICE, described, size, fill, trouble)
    }

    fn open_on(device: &CStr, described: &Described, size: Size, fill: Fill, trouble: Trouble) -> Result<Self, Error> {
        let pcm = Pcm::open(device)?;
        let (params, rate) = HwParams::of_format(&pcm, described.rate_hz)?;
        if rate != described.rate_hz {
            return Err(Error::new(format!("the output now runs at {rate} Hz, not {} Hz", described.rate_hz)));
        }
        let period = match size {
            Size::Frames(frames) => {
                let frames = Uframes::from(frames);
                // The size asked for must be one the device grants: ALSA
                // would otherwise round it without saying (RULE-97 item 1
                // asks only for sizes in the granted range).
                if let Some((least, most)) = params.period_range()
                    && !(least..=most).contains(&frames)
                {
                    return Err(Error::new(format!("a period of {frames} frames is outside the granted {least} to {most}")));
                }
                params.set_period(&pcm, frames)?;
                params.apply(&pcm)?
            }
            Size::Own => {
                // The device's own period, then a buffer of two of them: a
                // sound server's plug-in pairs its period with many more,
                // a far longer wait for every sound.
                let own = params.apply(&pcm)?;
                drop(params);
                let (params, _) = HwParams::of_format(&pcm, rate)?;
                params.set_period(&pcm, own)?;
                params.apply(&pcm)?
            }
        };
        set_software(&pcm, period)?;
        let period = usize::try_from(period).map_err(|_| Error::new("the period"))?;
        if period > usize::try_from(crate::MAX_BUFFER_FRAMES).unwrap_or(usize::MAX) {
            return Err(Error::new(format!("the output chose a period of {period} frames")));
        }
        let wait_ms = u64::try_from(period)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(STUCK_PERIODS))
            .saturating_mul(MILLISECONDS_PER_SECOND)
            .checked_div(u64::from(rate))
            .unwrap_or(0)
            .max(u64::from(LEAST_WAIT_MS));
        let wait_ms = c_int::try_from(wait_ms).unwrap_or(c_int::MAX);
        let control = Arc::new(Control::new());
        let thread_control = Arc::clone(&control);
        let thread = std::thread::Builder::new()
            .name("sound output".into())
            .spawn(move || write(&pcm, period, wait_ms, &thread_control, fill, trouble))
            .map_err(|error| Error::new(format!("the output's thread: {error}")))?;
        Ok(Self { control, thread: Some(thread) })
    }

    /// Starts the stream.
    #[expect(clippy::unnecessary_wraps, reason = "one signature on every platform: starting an audio unit on macOS can fail")]
    pub fn play(&mut self) -> Result<(), Error> {
        self.control.set(Wanted::Playing);
        Ok(())
    }

    /// Stops it.
    #[expect(clippy::unnecessary_wraps, reason = "one signature on every platform: stopping an audio unit on macOS can fail")]
    pub fn pause(&mut self) -> Result<(), Error> {
        self.control.set(Wanted::Paused);
        Ok(())
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.control.set(Wanted::Closing);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The stream's thread: while the owner wants it playing, fills a period
/// and writes it; stopped, drops what the device holds and waits.
fn write(pcm: &Pcm, period: usize, wait_ms: c_int, control: &Control, mut fill: Fill, mut trouble: Trouble) {
    let mut samples = vec![0_i16; period.saturating_mul(CHANNELS)];
    let mut follower = Follower::new();
    let mut running = false;
    loop {
        match follower.next(control) {
            Wanted::Closing => break,
            Wanted::Paused => {
                if running {
                    // SAFETY: the handle is open and used by this thread
                    // alone.
                    unsafe { snd_pcm_drop(pcm.0) };
                    running = false;
                }
            }
            Wanted::Playing => {
                if !running {
                    // SAFETY: as above.
                    if let Err(error) = checked(unsafe { snd_pcm_prepare(pcm.0) }, "starting the output") {
                        trouble(&error.to_string());
                        follower.broke();
                        continue;
                    }
                    running = true;
                }
                fill(&mut samples);
                if let Err(error) = write_period(pcm, &samples, wait_ms, &mut trouble) {
                    trouble(&error.to_string());
                    // SAFETY: as above.
                    unsafe { snd_pcm_drop(pcm.0) };
                    running = false;
                    follower.broke();
                }
            }
        }
    }
    if running {
        // SAFETY: as above; the handle is closed when `pcm` is dropped.
        unsafe { snd_pcm_drop(pcm.0) };
    }
}

/// Writes one period, waiting for room. An underrun or a suspend is told
/// and recovered from (`snd_pcm_recover`), and the rest written; any other
/// failure, or a device with no room for [`STUCK_PERIODS`], is returned.
fn write_period(pcm: &Pcm, samples: &[i16], wait_ms: c_int, trouble: &mut Trouble) -> Result<(), Error> {
    let mut rest = samples;
    while !rest.is_empty() {
        // SAFETY: the handle is open and used by this thread alone.
        let ready = unsafe { snd_pcm_wait(pcm.0, wait_ms) };
        if ready == 0 {
            return Err(Error::new("the output took no samples for twenty periods"));
        }
        let frames = Uframes::try_from(rest.len() / CHANNELS).unwrap_or(0);
        let written = if ready < 0 {
            Sframes::from(ready)
        } else {
            // SAFETY: `rest` holds `frames` whole frames of interleaved
            // 16-bit samples, the format the device was configured with,
            // and lives until the call returns.
            unsafe { snd_pcm_writei(pcm.0, rest.as_ptr().cast(), frames) }
        };
        if written < 0 {
            let code = c_int::try_from(written).unwrap_or(c_int::MIN);
            trouble(&text(code));
            // SAFETY: as above; silent, as the trouble was just told.
            checked(unsafe { snd_pcm_recover(pcm.0, code, 1) }, "recovering the output")?;
            continue;
        }
        let taken = usize::try_from(written).unwrap_or(0).saturating_mul(CHANNELS);
        rest = rest.get(taken..).unwrap_or(&[]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// ALSA's `null` device, which takes every sample at once and plays
    /// nothing: the stream's thread, its configuration and its following of
    /// the owner, through the real library, with nothing heard.
    const NULL_DEVICE: &CStr = c"null";
    /// Long enough for the thread to have written many periods, or none.
    const SETTLE: Duration = Duration::from_millis(100);

    fn counted() -> (Arc<AtomicUsize>, Arc<AtomicUsize>, Fill) {
        let (calls, samples) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (counted_calls, counted_samples) = (Arc::clone(&calls), Arc::clone(&samples));
        let fill: Fill = Box::new(move |data: &mut [i16]| {
            counted_calls.fetch_add(1, Ordering::Relaxed);
            counted_samples.store(data.len(), Ordering::Relaxed);
            data.fill(0);
        });
        (calls, samples, fill)
    }

    /// Opened stopped, the stream fills nothing; played, it fills periods
    /// of the size asked for, two channels each; paused, it stops filling;
    /// dropped, its thread ends.
    #[test]
    fn the_stream_fills_only_while_it_plays() {
        let described = describe_on(NULL_DEVICE).unwrap();
        assert_eq!(described.rate_hz, ASKED_RATE_HZ, "the null device takes any rate");
        let Granted::Range(least, most) = crate::checked(described).unwrap().granted else { panic!("the null device grants a range") };
        let frames = 480_u32.clamp(least, most);
        let (calls, samples, fill) = counted();
        let mut stream = Stream::open_on(NULL_DEVICE, &described, Size::Frames(frames), fill, Box::new(|_| {})).unwrap();
        std::thread::sleep(SETTLE);
        assert_eq!(calls.load(Ordering::Relaxed), 0, "stopped, nothing is filled");
        stream.play().unwrap();
        std::thread::sleep(SETTLE);
        stream.pause().unwrap();
        std::thread::sleep(SETTLE);
        let filled = calls.load(Ordering::Relaxed);
        assert!(filled > 0, "playing, periods are filled");
        assert_eq!(samples.load(Ordering::Relaxed), usize::try_from(frames).unwrap() * CHANNELS, "a period of the size asked, in two channels");
        std::thread::sleep(SETTLE);
        assert_eq!(calls.load(Ordering::Relaxed), filled, "paused, nothing more is filled");
        stream.play().unwrap();
        std::thread::sleep(SETTLE);
        assert!(calls.load(Ordering::Relaxed) > filled, "played again, it fills again");
        let started = Instant::now();
        drop(stream);
        assert!(started.elapsed() < SETTLE, "dropped, the thread ends at once");
    }

    /// The device's own period is taken when no size is asked for.
    #[test]
    fn the_devices_own_period_is_taken_when_none_is_asked() {
        let described = describe_on(NULL_DEVICE).unwrap();
        let (calls, samples, fill) = counted();
        let mut stream = Stream::open_on(NULL_DEVICE, &described, Size::Own, fill, Box::new(|_| {})).unwrap();
        stream.play().unwrap();
        std::thread::sleep(SETTLE);
        drop(stream);
        assert!(calls.load(Ordering::Relaxed) > 0);
        assert!(samples.load(Ordering::Relaxed) > 0 && samples.load(Ordering::Relaxed) % CHANNELS == 0, "whole frames");
    }

    /// A size outside what the device grants is refused, never rounded.
    #[test]
    fn a_size_outside_the_granted_range_is_refused() {
        let described = describe_on(NULL_DEVICE).unwrap();
        let Granted::Range(_, most) = described.granted else { panic!("the null device grants a range") };
        let (_, _, fill) = counted();
        if let Some(beyond) = most.checked_add(1) {
            assert!(Stream::open_on(NULL_DEVICE, &described, Size::Frames(beyond), fill, Box::new(|_| {})).is_err());
        }
    }
}
