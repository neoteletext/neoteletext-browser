# Credits

- **Smithay** (MIT licence), `src/backend/winit/mod.rs` in version 0.7.0: the order of steps that sets up OpenGL ES on a winit window under X11 and Wayland, followed by the display host's own window layer (`spikes/display-host/src/window.rs`). The display host needs its own because Smithay's winit back end does not pass on input method events.
