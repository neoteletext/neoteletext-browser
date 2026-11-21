//! Windows: WASAPI in shared mode, event driven, following Microsoft's
//! "Rendering a Stream" and `IAudioClient::Initialize` (Core Audio APIs,
//! Windows SDK): the default render endpoint for the console role
//! (`IMMDeviceEnumerator::GetDefaultAudioEndpoint`), an `IAudioClient`
//! initialised with the endpoint's mix rate and 16-bit stereo PCM, which
//! the audio engine converts (`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM`), and an
//! event the engine signals whenever a period's room is free
//! (`AUDCLNT_STREAMFLAGS_EVENTCALLBACK`). The client lives on a thread of
//! its own, in the multithreaded apartment, which alone touches it: stopped
//! (`IAudioClient::Stop`) the thread waits for the owner; started, it fills
//! the room the engine reports (`GetCurrentPadding`) through
//! `IAudioRenderClient::GetBuffer` and `ReleaseBuffer`.
//!
//! A device that goes away while playing (`AUDCLNT_E_DEVICE_INVALIDATED`)
//! is told, and the stream plays nothing more until the owner asks again;
//! the helper goes on running, as it did with `cpal`.
#![allow(unsafe_code, reason = "WASAPI's interfaces are COM objects, whose every method is a foreign call: each block has its reason")]

use std::sync::Arc;
use std::sync::mpsc;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient,
    IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::control::{Control, Follower, Wanted};
use crate::{CHANNELS, Described, Error, Fill, Granted, Size, Trouble};

/// `REFERENCE_TIME`, in which WASAPI gives durations: hundreds of
/// nanoseconds, ten million to the second.
const REFERENCE_TIME_PER_SECOND: u64 = 10_000_000;
/// Bits in a sample.
const BITS_PER_SAMPLE: u16 = 16;
/// Bytes in a frame: two channels of two bytes.
const BYTES_PER_FRAME: u16 = 4;
const _: () = assert!(BYTES_PER_FRAME as usize == CHANNELS * size_of::<i16>());
/// How long the thread waits for the engine's event before it says the
/// device is stuck, in milliseconds: twenty of the longest periods.
const STUCK_MS: u32 = 2_000;

fn failed(what: &str, error: &windows::core::Error) -> Error {
    Error::new(format!("{what}: {error}"))
}

/// The multithreaded apartment, for the life of one thread.
struct Apartment;

impl Apartment {
    fn enter() -> Result<Self, Error> {
        // SAFETY: called on the thread that uses the apartment, which
        // leaves it once, when this is dropped.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().map_err(|error| failed("COM", &error))?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: entered by `enter` on this thread.
        unsafe { CoUninitialize() };
    }
}

/// The default render endpoint's audio client, and the endpoint's mix
/// rate and default period.
fn client() -> Result<(IAudioClient, u32, u32), Error> {
    // SAFETY: COM is entered on this thread; the enumerator, the device
    // and the client are reference counted by the bindings and released
    // when dropped. The mix format is the engine's own allocation, read
    // once and freed with `CoTaskMemFree` as `GetMixFormat` requires.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|error| failed("the device enumerator", &error))?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|error| failed("no audio output device", &error))?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|error| failed("the audio client", &error))?;
        let mix = client.GetMixFormat().map_err(|error| failed("the mix format", &error))?;
        if mix.is_null() {
            return Err(Error::new("the device gave no mix format"));
        }
        let rate = (*mix).nSamplesPerSec;
        CoTaskMemFree(Some(mix.cast_const().cast()));
        let mut period: i64 = 0;
        client.GetDevicePeriod(Some(&raw mut period), None).map_err(|error| failed("the device period", &error))?;
        let period_frames = frames(u64::try_from(period).unwrap_or(0), rate);
        Ok((client, rate, period_frames))
    }
}

/// A duration in `REFERENCE_TIME` as frames at `rate`.
fn frames(duration: u64, rate: u32) -> u32 {
    u32::try_from(duration.saturating_mul(u64::from(rate)).checked_div(REFERENCE_TIME_PER_SECOND).unwrap_or(0)).unwrap_or(u32::MAX)
}

/// Frames at `rate` as a duration in `REFERENCE_TIME`.
fn duration(frames: u32, rate: u32) -> i64 {
    i64::try_from(u64::from(frames).saturating_mul(REFERENCE_TIME_PER_SECOND).checked_div(u64::from(rate)).unwrap_or(0)).unwrap_or(i64::MAX)
}

/// Describes the default render endpoint: its mix rate, and its period
/// as the one size it grants, as the shared engine runs at it.
pub fn describe() -> Result<Described, Error> {
    // On a thread of its own, so that the caller's apartment is not
    // changed.
    std::thread::Builder::new()
        .name("sound output".into())
        .spawn(|| {
            let _apartment = Apartment::enter()?;
            let (_, rate, period) = client()?;
            Ok(Described { rate_hz: rate, granted: Granted::Range(period, period) })
        })
        .map_err(|error| Error::new(format!("the output's thread: {error}")))?
        .join()
        .map_err(|_| Error::new("the output's thread ended"))?
}

/// The stream: its thread, which owns the client.
pub struct Stream {
    control: Arc<Control>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Stream {
    /// Opens the default render endpoint at the rate described, stopped.
    pub fn open(described: &Described, size: Size, fill: Fill, trouble: Trouble) -> Result<Self, Error> {
        let control = Arc::new(Control::new());
        let thread_control = Arc::clone(&control);
        let (opened, result) = mpsc::sync_channel(1);
        let described = *described;
        let thread = std::thread::Builder::new()
            .name("sound output".into())
            .spawn(move || {
                let started = Apartment::enter().and_then(|apartment| Opened::new(&described, size).map(|stream| (apartment, stream)));
                match started {
                    Ok((_apartment, stream)) => {
                        let _ = opened.send(Ok(()));
                        stream.run(&thread_control, fill, trouble);
                    }
                    Err(error) => {
                        let _ = opened.send(Err(error));
                    }
                }
            })
            .map_err(|error| Error::new(format!("the output's thread: {error}")))?;
        match result.recv() {
            Ok(Ok(())) => Ok(Self { control, thread: Some(thread) }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(Error::new("the output's thread ended")),
        }
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

/// An initialised client, its render service and its event, used by the
/// stream's thread alone.
struct Opened {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: HANDLE,
    buffer_frames: u32,
}

impl Opened {
    fn new(described: &Described, size: Size) -> Result<Self, Error> {
        let (client, rate, _) = client()?;
        if rate != described.rate_hz {
            return Err(Error::new(format!("the output now runs at {rate} Hz, not {} Hz", described.rate_hz)));
        }
        let format = WAVEFORMATEX {
            wFormatTag: u16::try_from(WAVE_FORMAT_PCM).unwrap_or(1),
            nChannels: u16::try_from(CHANNELS).unwrap_or(2),
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate.saturating_mul(u32::from(BYTES_PER_FRAME)),
            nBlockAlign: BYTES_PER_FRAME,
            wBitsPerSample: BITS_PER_SAMPLE,
            cbSize: 0,
        };
        // In shared mode with an event the buffer's duration is the period
        // asked for; none asks for the engine's own.
        let buffer = match size {
            Size::Own => 0,
            Size::Frames(frames) => duration(frames, rate),
        };
        // SAFETY: the client is this thread's; the format lives until the
        // call returns; the event is closed when this is dropped, after the
        // client that signals it.
        unsafe {
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    buffer,
                    0,
                    &raw const format,
                    None,
                )
                .map_err(|error| failed("cannot open the audio output", &error))?;
            let event = CreateEventW(None, false, false, None).map_err(|error| failed("the output's event", &error))?;
            let opened = (|| {
                client.SetEventHandle(event).map_err(|error| failed("the output's event", &error))?;
                let buffer_frames = client.GetBufferSize().map_err(|error| failed("the output's buffer", &error))?;
                let render: IAudioRenderClient = client.GetService().map_err(|error| failed("the render client", &error))?;
                Ok((buffer_frames, render))
            })();
            match opened {
                Ok((buffer_frames, render)) if buffer_frames <= crate::MAX_BUFFER_FRAMES => Ok(Self { client, render, event, buffer_frames }),
                Ok((buffer_frames, _)) => {
                    let _ = CloseHandle(event);
                    Err(Error::new(format!("the output's buffer of {buffer_frames} frames")))
                }
                Err(error) => {
                    let _ = CloseHandle(event);
                    Err(error)
                }
            }
        }
    }

    /// Fills what room the engine has.
    fn fill_room(&self, fill: &mut Fill) -> Result<(), Error> {
        // SAFETY: the client and its render service are this thread's.
        // `GetBuffer` gives room for `room` frames of the format given to
        // `Initialize`, 16-bit stereo, aligned for its samples, which is
        // ours until `ReleaseBuffer` hands it back with all of it written.
        unsafe {
            let padding = self.client.GetCurrentPadding().map_err(|error| failed("the output's room", &error))?;
            let room = self.buffer_frames.saturating_sub(padding);
            if room == 0 {
                return Ok(());
            }
            let data = self.render.GetBuffer(room).map_err(|error| failed("the output's buffer", &error))?;
            if data.is_null() || data.align_offset(align_of::<i16>()) != 0 {
                return Err(Error::new("the output gave no buffer, or one not aligned for its samples"));
            }
            let samples = usize::try_from(room).unwrap_or(0).saturating_mul(CHANNELS);
            #[expect(clippy::cast_ptr_alignment, reason = "the alignment is checked just above")]
            let slice = std::slice::from_raw_parts_mut(data.cast::<i16>(), samples);
            fill(slice);
            self.render.ReleaseBuffer(room, 0).map_err(|error| failed("the output's buffer", &error))
        }
    }

    /// The thread's loop: follows the owner; playing, fills the room at
    /// each of the engine's events.
    fn run(self, control: &Control, mut fill: Fill, mut trouble: Trouble) {
        let mut follower = Follower::new();
        let mut running = false;
        loop {
            match follower.next(control) {
                Wanted::Closing => break,
                Wanted::Paused => {
                    if running {
                        // SAFETY: the client is this thread's.
                        let _ = unsafe { self.client.Stop() };
                        running = false;
                    }
                }
                Wanted::Playing => {
                    if !running {
                        // The buffer is filled before the stream starts, so
                        // its first period is not silence (Microsoft's
                        // "Rendering a Stream").
                        // SAFETY: the client is this thread's.
                        let started =
                            self.fill_room(&mut fill).and_then(|()| unsafe { self.client.Start() }.map_err(|error| failed("starting the output", &error)));
                        if let Err(error) = started {
                            trouble(&error.to_string());
                            follower.broke();
                            continue;
                        }
                        running = true;
                    }
                    // SAFETY: the event is open until this is dropped.
                    let woken = unsafe { WaitForSingleObject(self.event, STUCK_MS) };
                    let result =
                        if woken == WAIT_OBJECT_0 { self.fill_room(&mut fill) } else { Err(Error::new("the output signalled nothing for two seconds")) };
                    if let Err(error) = result {
                        trouble(&error.to_string());
                        // SAFETY: the client is this thread's.
                        let _ = unsafe { self.client.Stop() };
                        running = false;
                        follower.broke();
                    }
                }
            }
        }
        if running {
            // SAFETY: the client is this thread's.
            let _ = unsafe { self.client.Stop() };
        }
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: the event was made by `CreateEventW` and is closed once;
        // the client that signals it is stopped.
        let _ = unsafe { CloseHandle(self.event) };
    }
}
