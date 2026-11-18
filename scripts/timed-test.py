#!/usr/bin/env python3
"""The timed test between the browser's real processes (neoteletext-common's
page-rules.md RULE-66; REQ-6535): presses keys on the music player's page
101 through the X test extension (XTEST) and reads, from the browser's own
marks, when each process did its part.

Run by scripts/timed-test.sh, which builds the browser with the feature
`timed-test` (the marks and the test's settings, in no shipped build),
starts a private X display, a private PipeWire with a null sink, the server
and the music player, and runs this once for each setting:

  timed-test.py DISPLAY BROWSER_LOG LABEL KEYS KEY_RECT METER_RECT [--check]

Each process of the browser writes a line `neoteletext-timed PROCESS WHAT
MICROSECONDS DETAIL` on the machine's monotonic clock (CLOCK_MONOTONIC,
which Python's time.monotonic also reads):

  interface key       the interface read the key from the X server
  decoder local-midi  the decoder wrote the note's LocalMidi to the core
  sound sample        the sound helper handed the output device the buffer
                      holding the note's first sample that is not silence;
                      the detail is the buffer's frames
  interface composed  the interface presented a frame; the detail is its
                      damaged rectangles, x0,y0,x1,y1;...
  interface phase     the interface told its frames; decoder phase, the
                      decoder read them: both "PERIOD NEXT_IN"

and the hops between them (DEC-6659 item 23): the interface's key-sent;
the core's input-arrived (its reader), input-told (written for the
decoder), regis-read and local-midi-read; the decoder's input-read, paint
and regis (a repaint written); the sound helper's local-midi-read; the
interface's regis-taken (on the window's thread) and presented. The
"composed" time is the test display's composition that shows the frame:
the first at or after its present.

From them it prints, for the key's LocalMidi, its first sample, the frame
that shows the key's changed rectangle (KEY_RECT, in the display's pixels)
and the frame that shows the meter's lowest segment (METER_RECT), the
least, median and greatest interval from the key, with the device buffer
granted and the display's refresh; and the FramePhase's error, the time
the phase spent in the pipes from the interface through the trusted core
to the decoder. With --check it fails when a bound of RULE-64 or RULE-65 is
passed: given for the browser with every change of DEC-6580 and DEC-6635
on, so a run with one switched off is printed and not held to them.
"""
import ctypes
import random
import statistics
import sys
import time

# RULE-66: at least 200 keys, at random phases of the tick and of the
# device buffer.
LEAST_KEYS = 200
# How long a key is held, and the silence after it, so that the next note
# begins from silence and its first sample is the note's own, and the
# meter's lowest segment is dark again: its last change came 537
# milliseconds after the key in the median and 590 at most (19 November
# 2026, 200 keys), so a rest of 450 after a hold of 100 often left it
# falling when the next key came, and the key then found no frame that
# lit it. 800 leaves 310 milliseconds beyond the longest fall seen.
HOLD_SECONDS = 0.1
REST_SECONDS = 0.8
# The random part of the gap: longer than a tick and a device buffer.
JITTER_SECONDS = 0.05
# How long after the key a mark may come and still be the key's.
WINDOW_US = 400_000
# The keys pressed before the timed ones.
WARM_UP_KEYS = 1
# The music player plays C4 on the computer key A (its RULE-92).
KEY = b"a"
NOTE_ON = 0x90
STATUS_KIND = 0xF0
MICROSECONDS_PER_MILLISECOND = 1000.0
# The test display's refresh period, R (interface/src/app/test_display.rs).
REFRESH_MS = 16.667
# RULE-64's bounds, most and median, by the device buffer granted: the
# frames at 48,000 a second and the row above which a size falls to the
# next row down.
SOUND_BOUNDS = [(128, 10.0, 5.0), (256, 12.0, 6.0), (480, 17.0, 9.0)]
# RULE-64: LocalMidi leaves the decoder within 2 ms of the key.
MIDI_MOST_MS = 2.0
# RULE-65: 6 + R at most and 4.5 + R / 2 in the median to the changed
# rectangle; 57 and 30 to the meter's lowest segment.
PICTURE_MOST_MS = 6.0 + REFRESH_MS
PICTURE_MEDIAN_MS = 4.5 + REFRESH_MS / 2
METER_MOST_MS = 57.0
METER_MEDIAN_MS = 30.0


def marks(path):
    """Every mark of the log, as (microseconds, process, what, detail)."""
    found = []
    for line in open(path, errors="replace"):
        parts = line.split()
        if len(parts) < 4 or parts[0] != "neoteletext-timed" or not parts[3].isdigit():
            continue
        found.append((int(parts[3]), parts[1], parts[2], " ".join(parts[4:])))
    found.sort()
    return found


def boxes(detail):
    for box in detail.split(";"):
        if box:
            yield tuple(int(value) for value in box.split(","))


def meets(detail, rect):
    x0, y0, x1, y1 = rect
    return any(a < x1 and x0 < c and b < y1 and y0 < d for a, b, c, d in boxes(detail))


def figures(name, values):
    if not values:
        return f"{name}: none"
    return f"{name}: n {len(values)}, least {min(values):.1f} ms, median {statistics.median(values):.1f} ms, greatest {max(values):.1f} ms"


# The hops of a key's way to the sound helper and to the picture, in
# order.
MIDI_HOPS = [("interface", "key-sent"), ("core", "input-arrived"), ("core", "input-told"), ("decoder", "input-read"), ("decoder", "local-midi"),
             ("core", "local-midi-read"), ("sound", "local-midi-read")]
PICTURE_HOPS = [("decoder", "regis"), ("core", "regis-read"), ("interface", "regis-taken"), ("interface", "presented")]


def hops(found, keys, picture_at):
    """Each hop's least, median and greatest from the one before it, in
    milliseconds: the sound's way for every key, and the picture's from
    the repaint written last before the frame that showed the key."""
    lines = []
    for name, chain, frames in (("LocalMidi", MIDI_HOPS, None), ("picture", PICTURE_HOPS, picture_at)):
        gaps = {hop: [] for hop in chain}
        for key in keys:
            if frames is not None and key not in frames:
                continue
            until = key + WINDOW_US if frames is None else frames[key]
            after = [mark for mark in found if key < mark[0] < until]
            if frames is not None:
                written = [mark[0] for mark in after if mark[1:3] == chain[0]]
                if not written:
                    continue
                at = written[-1]
                hop_chain = chain[1:]
                gaps[chain[0]].append((at - key) / MICROSECONDS_PER_MILLISECOND)
            else:
                at, hop_chain = key, chain
            # Each hop is the first of its kind after the start: a process
            # may mark what it read before the one that wrote it has marked
            # its writing, so the order of two marks a few microseconds
            # apart is not the order of the hops.
            start = at
            for hop in hop_chain:
                nxt = next((mark[0] for mark in after if mark[0] >= start and mark[1:3] == hop), None)
                if nxt is None:
                    break
                gaps[hop].append((nxt - at) / MICROSECONDS_PER_MILLISECOND)
                at = nxt
            if frames is not None:
                gaps.setdefault(("interface", "composed"), []).append((frames[key] - at) / MICROSECONDS_PER_MILLISECOND)
        before = "key"
        for hop, values in gaps.items():
            lines.append("    " + figures(f"{name} hop to {hop[0]} {hop[1]} (from {before})", values))
            before = f"{hop[0]} {hop[1]}"
    return lines


def press_keys(display, count):
    x11, xtst = ctypes.CDLL("libX11.so.6"), ctypes.CDLL("libXtst.so.6")
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XStringToKeysym.restype = ctypes.c_ulong
    dpy = ctypes.c_void_p(x11.XOpenDisplay(display.encode()))
    if not dpy:
        sys.exit(f"cannot open the display {display}")
    code = x11.XKeysymToKeycode(dpy, ctypes.c_ulong(x11.XStringToKeysym(KEY)))
    random.seed(66)
    # One key first, not timed: the first frame after the window takes the
    # focus is drawn whole.
    for _ in range(WARM_UP_KEYS):
        xtst.XTestFakeKeyEvent(dpy, code, 1, 0)
        x11.XFlush(dpy)
        time.sleep(HOLD_SECONDS)
        xtst.XTestFakeKeyEvent(dpy, code, 0, 0)
        x11.XFlush(dpy)
    time.sleep(REST_SECONDS)
    started = int(time.monotonic() * 1_000_000)
    for _ in range(count):
        time.sleep(REST_SECONDS + random.uniform(0, JITTER_SECONDS))
        xtst.XTestFakeKeyEvent(dpy, code, 1, 0)
        x11.XFlush(dpy)
        time.sleep(HOLD_SECONDS)
        xtst.XTestFakeKeyEvent(dpy, code, 0, 0)
        x11.XFlush(dpy)
    time.sleep(REST_SECONDS)
    return started


def main():
    if len(sys.argv) < 7:
        sys.exit(__doc__)
    display, log, label, count = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    key_rect, meter_rect = (tuple(int(value) for value in argument.split(",")) for argument in sys.argv[5:7])
    check = "--check" in sys.argv[7:]
    if count < LEAST_KEYS and check:
        sys.exit(f"RULE-66 asks for at least {LEAST_KEYS} keys")
    started = press_keys(display, count)
    found = [mark for mark in marks(log) if mark[0] >= started]
    keys = [at for at, process, what, _ in found if (process, what) == ("interface", "key")]
    midi, sound, picture, meter, buffers = [], [], [], [], set()
    picture_at = {}
    for index, key in enumerate(keys):
        until = min(keys[index + 1] if index + 1 < len(keys) else key + WINDOW_US, key + WINDOW_US)
        after = [mark for mark in found if key < mark[0] < until]
        for at, process, what, detail in after:
            if (process, what) == ("decoder", "local-midi") and int(detail.split()[0], 16) & STATUS_KIND == NOTE_ON:
                midi.append((at - key) / MICROSECONDS_PER_MILLISECOND)
                break
        for at, process, what, detail in after:
            if (process, what) == ("sound", "sample"):
                sound.append((at - key) / MICROSECONDS_PER_MILLISECOND)
                buffers.add(int(detail))
                break
        for at, process, what, detail in after:
            if (process, what) == ("interface", "composed") and meets(detail, key_rect):
                picture.append((at - key) / MICROSECONDS_PER_MILLISECOND)
                picture_at[key] = at
                break
        for at, process, what, detail in after:
            if (process, what) == ("interface", "composed") and meets(detail, meter_rect):
                meter.append((at - key) / MICROSECONDS_PER_MILLISECOND)
                break
    # The phase's error: each decoder reading against the interface's
    # telling before it, both as the time of the next composition.
    errors = []
    told = None
    for at, process, what, detail in found:
        if what != "phase":
            continue
        period, next_in = (int(value) for value in detail.split())
        composition = at + next_in
        if process == "interface":
            told = (composition, period)
        elif told is not None:
            offset = (composition - told[0]) % told[1]
            errors.append((offset if offset * 2 <= told[1] else offset - told[1]) / MICROSECONDS_PER_MILLISECOND)
    print(f"== {label}: {len(keys)} keys read of {count} pressed; device buffer {sorted(buffers) or 'unknown'} frames; display {1000 / REFRESH_MS:.2f} hertz (the test display)")
    print("  " + figures("key to LocalMidi written by the decoder", midi))
    print("  " + figures("key to the note's first sample handed to the device", sound))
    print("  " + figures("key to the frame composed with the key's changed rectangle", picture))
    print("  " + figures("key to the frame composed with the meter's lowest segment", meter))
    print("  " + figures("FramePhase's error through the pipes, interface to decoder", errors))
    print("  the hops:")
    for line in hops(found, keys, picture_at):
        print(line)
    if not check:
        return
    failures = []
    if len(keys) < count:
        failures.append(f"the interface read {len(keys)} of {count} keys")
    for name, values in (("LocalMidi", midi), ("sound", sound), ("picture", picture), ("meter", meter)):
        if len(values) < len(keys):
            failures.append(f"{name}: {len(values)} of {len(keys)} keys seen")
    if midi and max(midi) > MIDI_MOST_MS:
        failures.append(f"LocalMidi at most {MIDI_MOST_MS} ms (RULE-64): {max(midi):.1f}")
    granted = max(buffers) if buffers else None
    bound = next(((most, median) for frames, most, median in SOUND_BOUNDS if granted is not None and granted <= frames), None)
    if sound and bound:
        if max(sound) > bound[0] or statistics.median(sound) > bound[1]:
            failures.append(f"sound at most {bound[0]} and {bound[1]} in the median at {granted} frames (RULE-64): {max(sound):.1f}, {statistics.median(sound):.1f}")
    elif sound:
        print(f"  RULE-64 gives no bound for a device buffer of {granted} frames: printed only")
    if picture and (max(picture) > PICTURE_MOST_MS or statistics.median(picture) > PICTURE_MEDIAN_MS):
        failures.append(f"picture at most {PICTURE_MOST_MS:.1f} and {PICTURE_MEDIAN_MS:.1f} in the median (RULE-65): {max(picture):.1f}, {statistics.median(picture):.1f}")
    if meter and (max(meter) > METER_MOST_MS or statistics.median(meter) > METER_MEDIAN_MS):
        failures.append(f"meter at most {METER_MOST_MS} and {METER_MEDIAN_MS} in the median (RULE-65): {max(meter):.1f}, {statistics.median(meter):.1f}")
    for failure in failures:
        print(f"  FAILED: {failure}")
    if failures:
        sys.exit(1)


if __name__ == "__main__":
    main()
