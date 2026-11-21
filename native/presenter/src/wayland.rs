//! Wayland: two buffers of shared memory (`wl_shm`, format `xrgb8888`, the
//! pixel as the interface draws it), attached to the window's surface in
//! turn, each present marking only the boxes that changed
//! (`wl_surface.damage_buffer`, from version 4 of `wl_surface`; before it
//! the whole surface). A buffer the compositor still holds is not drawn
//! into: the next frame waits for its `wl_buffer.release` (the Wayland
//! protocol, `wl_buffer` and `wl_surface.attach`). winit owns the
//! connection and the surface and is told before each present; this
//! keeps a queue of its own for its buffers' events, on the same
//! connection.
#![allow(unsafe_code, reason = "winit's Wayland connection and surface are given as raw pointers: two blocks, each with its reason")]

use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use raw_window_handle::{WaylandDisplayHandle, WaylandWindowHandle};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

use crate::shared_memory::{Kind, Mapping};
use crate::{BYTES_PER_PIXEL, Error, Rect, pixel_count};

/// The version of `wl_surface` from which damage is given in the buffer's
/// own pixels (`wl_surface.damage_buffer`).
const DAMAGE_BUFFER_VERSION: u32 = 4;
/// The version of `wl_shm` bound: the first, which has all this uses.
const SHM_VERSION: u32 = 1;

fn failed(what: &str, error: impl std::fmt::Display) -> Error {
    Error::new(format!("{what}: {error}"))
}

/// The queue's state: nothing, since each buffer's release is kept with
/// the buffer.
struct State;

/// One buffer: its memory, its pool and the buffer made from it.
struct Buffer {
    memory: Mapping,
    pool: wl_shm_pool::WlShmPool,
    proxy: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
    /// The compositor has given it back, or never had it.
    released: Arc<AtomicBool>,
    /// How many presents ago its pixels were presented; 0 for none.
    age: u8,
}

impl Buffer {
    fn new(shm: &wl_shm::WlShm, queue: &QueueHandle<State>, width: u32, height: u32) -> Result<Self, Error> {
        let memory = Mapping::new(Kind::Sealed, pixel_count(width, height))?;
        let size = i32::try_from(memory.bytes()).map_err(|_| Error::new("a buffer beyond Wayland's sizes"))?;
        let pool = shm.create_pool(memory.as_fd(), size, queue, ());
        let released = Arc::new(AtomicBool::new(true));
        let proxy = make_buffer(&pool, queue, width, height, &released)?;
        Ok(Self { memory, pool, proxy, width, height, released, age: 0 })
    }

    /// Makes it `width` by `height`, growing its memory and its pool when
    /// it must; what it held is no longer a frame.
    fn resize(&mut self, queue: &QueueHandle<State>, width: u32, height: u32) -> Result<(), Error> {
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        self.proxy.destroy();
        let pixels = pixel_count(width, height);
        if pixels.saturating_mul(BYTES_PER_PIXEL) > self.memory.bytes() {
            self.memory.grow(pixels)?;
            self.pool.resize(i32::try_from(self.memory.bytes()).map_err(|_| Error::new("a buffer beyond Wayland's sizes"))?);
        }
        self.proxy = make_buffer(&self.pool, queue, width, height, &self.released)?;
        self.width = width;
        self.height = height;
        self.age = 0;
        Ok(())
    }
}

fn make_buffer(
    pool: &wl_shm_pool::WlShmPool,
    queue: &QueueHandle<State>,
    width: u32,
    height: u32,
    released: &Arc<AtomicBool>,
) -> Result<wl_buffer::WlBuffer, Error> {
    let side = |value: u32| i32::try_from(value).map_err(|_| Error::new("a buffer beyond Wayland's sizes"));
    let stride = side(width.saturating_mul(u32::try_from(BYTES_PER_PIXEL).unwrap_or(u32::MAX)))?;
    Ok(pool.create_buffer(0, side(width)?, side(height)?, stride, wl_shm::Format::Xrgb8888, queue, Arc::clone(released)))
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.proxy.destroy();
        self.pool.destroy();
    }
}

/// Presenting to one Wayland surface.
pub struct Surface {
    connection: Connection,
    queue: EventQueue<State>,
    handle: QueueHandle<State>,
    shm: wl_shm::WlShm,
    window: wl_surface::WlSurface,
    /// The buffer drawn into next, and the one presented last.
    buffers: Option<(Buffer, Buffer)>,
    width: u32,
    height: u32,
}

impl Surface {
    pub fn new(display: WaylandDisplayHandle, window: WaylandWindowHandle) -> Result<Self, Error> {
        // SAFETY: the display is winit's live `wl_display`, which outlives
        // its windows and so this surface; the backend made from it does
        // not close it.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&connection).map_err(|error| failed("the Wayland registry", error))?;
        let handle = queue.handle();
        let shm: wl_shm::WlShm = globals.bind(&handle, SHM_VERSION..=SHM_VERSION, ()).map_err(|error| failed("Wayland's shared memory", error))?;
        // SAFETY: the surface is winit's live `wl_surface` of this window,
        // which outlives this surface, on the connection above.
        let id = unsafe { ObjectId::from_ptr(wl_surface::WlSurface::interface(), window.surface.as_ptr().cast()) }
            .map_err(|error| failed("the window's surface", error))?;
        let surface = wl_surface::WlSurface::from_id(&connection, id).map_err(|error| failed("the window's surface", error))?;
        Ok(Self { connection, queue, handle, shm, window: surface, buffers: None, width: 0, height: 0 })
    }

    pub const fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn prepare(&mut self) -> Result<u8, Error> {
        let (width, height) = (self.width, self.height);
        match &mut self.buffers {
            None => self.buffers = Some((Buffer::new(&self.shm, &self.handle, width, height)?, Buffer::new(&self.shm, &self.handle, width, height)?)),
            Some((next, _)) => {
                while !next.released.load(Ordering::Acquire) {
                    self.queue.blocking_dispatch(&mut State).map_err(|error| failed("waiting for the compositor", error))?;
                }
                next.resize(&self.handle, width, height)?;
            }
        }
        Ok(self.buffers.as_ref().map_or(0, |(next, _)| next.age))
    }

    pub fn pixels(&self) -> &[u32] {
        self.buffers.as_ref().map_or(&[], |(next, _)| next.memory.pixels(pixel_count(next.width, next.height)))
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        match &mut self.buffers {
            Some((next, _)) => next.memory.pixels_mut(pixel_count(next.width, next.height)),
            None => &mut [],
        }
    }

    pub fn present(&mut self, damage: &[Rect]) -> Result<(), Error> {
        let _ = self.queue.dispatch_pending(&mut State);
        let Some((next, last)) = &mut self.buffers else { return Err(Error::new("the surface has no buffer")) };
        next.released.store(false, Ordering::Release);
        self.window.attach(Some(&next.proxy), 0, 0);
        if self.window.version() < DAMAGE_BUFFER_VERSION {
            self.window.damage(0, 0, i32::MAX, i32::MAX);
        } else {
            for rect in damage {
                let side = |value: u32| i32::try_from(value).map_err(|_| Error::new("a box beyond Wayland's coordinates"));
                self.window.damage_buffer(side(rect.x)?, side(rect.y)?, side(rect.width.get())?, side(rect.height.get())?);
            }
        }
        self.window.commit();
        // The one presented is a frame old; the other, which is drawn into
        // next, one frame older than it was.
        next.age = 1;
        if last.age != 0 {
            last.age = last.age.saturating_add(1);
        }
        std::mem::swap(next, last);
        self.connection.flush().map_err(|error| failed("the compositor", error))
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm::WlShm, ()> for State {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, (): &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, (): &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_buffer::WlBuffer, Arc<AtomicBool>> for State {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, event: wl_buffer::Event, released: &Arc<AtomicBool>, _: &Connection, _: &QueueHandle<Self>) {
        if matches!(event, wl_buffer::Event::Release) {
            released.store(true, Ordering::Release);
        }
    }
}
