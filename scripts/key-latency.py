#!/usr/bin/env python3
"""Times a key from the X server to the pixels, on a private X display.

Presses keys through the X test extension (XTEST) on an Xvfb display and
watches a region of the display's framebuffer file for its pixels to
change: the time between is what the browser, the server and the channel
take between them to answer a key. A real keyboard, a real display's
refresh and a desktop's compositor are outside it.

Used for REQ-6399 to REQ-6401 (DEC-6398): run it before and after a change
to the path a keystroke takes, on the same machine, and compare the
quickest keys and the medians. A busy machine slows every hop at once, so
compare runs made one after the other.

Setting up (DEVELOPMENT.md section 15: never the owner's desktop):
  Xvfb :97 -screen 0 1600x1000x24 -fbdir DIR -nolisten tcp
  a server and a browser on DISPLAY=:97, the browser's window focused
  (xdotool windowfocus), on a page that answers every key, such as the
  ReGIS test channel's typing page; the browser built optimised, under
  native/target/ so that its sandbox may start.

  key-latency.py DIR/Xvfb_screen0 X0 Y0 X1 Y1 COUNT [GAP_MS] [DISPLAY]

The region X0 Y0 X1 Y1, in the display's pixels, must change at every key
and at nothing else: the line a ReGIS page's commit count is written on
does, where a blinking cursor would not do. The keys alternate a letter
and Backspace, so that a line never fills. With NEOTELETEXT_FRAME_TRACE
set for the browser, its interface also reports every frame it draws.
"""
import ctypes
import mmap
import struct
import sys
import time

# The XWD header's fields, as Xvfb writes its framebuffer file (XWDFile.h):
# 32-bit big-endian words, the header's size first.
HEADER_SIZE_AT, WIDTH_AT, BYTES_PER_LINE_AT, COLOURS_AT = 0, 16, 48, 76
COLOUR_ENTRY_BYTES = 12
BYTES_PER_PIXEL = 4
# How long a key may go unanswered before it is counted as missed.
GIVE_UP_SECONDS = 2.0
DEFAULT_GAP_MS = 300.0


def main() -> None:
    if len(sys.argv) < 7:
        sys.exit(__doc__)
    path, x0, y0, x1, y1, count = sys.argv[1], *map(int, sys.argv[2:7])
    gap = float(sys.argv[7]) / 1000 if len(sys.argv) > 7 else DEFAULT_GAP_MS / 1000
    display = sys.argv[8].encode() if len(sys.argv) > 8 else b":97"
    x11, xtst = ctypes.CDLL("libX11.so.6"), ctypes.CDLL("libXtst.so.6")
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XStringToKeysym.restype = ctypes.c_ulong
    dpy = ctypes.c_void_p(x11.XOpenDisplay(display))
    with open(path, "rb") as file:
        frame = mmap.mmap(file.fileno(), 0, prot=mmap.PROT_READ)

    def word(at: int) -> int:
        return struct.unpack(">I", frame[at : at + 4])[0]

    pixels_at = word(HEADER_SIZE_AT) + word(COLOURS_AT) * COLOUR_ENTRY_BYTES
    line = word(BYTES_PER_LINE_AT)

    def region() -> bytes:
        return b"".join(frame[pixels_at + y * line + x0 * BYTES_PER_PIXEL : pixels_at + y * line + x1 * BYTES_PER_PIXEL] for y in range(y0, y1, 2))

    keys = [b"a", b"BackSpace"]
    taken = []
    for index in range(count):
        code = x11.XKeysymToKeycode(dpy, ctypes.c_ulong(x11.XStringToKeysym(keys[index % len(keys)])))
        before = region()
        pressed = time.perf_counter()
        xtst.XTestFakeKeyEvent(dpy, code, 1, 0)
        x11.XFlush(dpy)
        changed = None
        while time.perf_counter() - pressed < GIVE_UP_SECONDS:
            if region() != before:
                changed = time.perf_counter() - pressed
                break
        xtst.XTestFakeKeyEvent(dpy, code, 0, 0)
        x11.XFlush(dpy)
        taken.append(changed)
        time.sleep(gap)
    answered = sorted(seconds * 1000 for seconds in taken if seconds is not None)
    print(f"keys {count}, answered {len(answered)}, missed {count - len(answered)}")
    if answered:
        nine_tenths = answered[max(int(len(answered) * 0.9) - 1, 0)]
        print(f"quickest {answered[0]:.1f}  median {answered[len(answered) // 2]:.1f}  nine in ten within {nine_tenths:.1f}  slowest {answered[-1]:.1f} ms")
        print(" ".join(f"{seconds * 1000:.0f}" for seconds in taken if seconds is not None))


if __name__ == "__main__":
    main()
