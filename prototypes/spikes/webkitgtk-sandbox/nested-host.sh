#!/usr/bin/env bash
# Decisions 29 and 30, measured: the confined window host (as in
# confined-host.sh) given a nested display of its own, hosted by a compositor
# that is itself a client of the user's display: cage, a stock compositor, or
# NeoTeletext's own display host (spikes/display-host). The window host
# receives only the nested display's socket, never the user's display. Can it
# still reach the user's other programs, or the trusted strip?
# Usage: nested-host.sh <the sandbox-network spike's directory> x11|wayland cage|<display host program>
set -uo pipefail
spike="$(cd "$1" && pwd)/src-tauri/target/release/sandbox-network-spike"
desktop=$2
compositor=$3
cd "$(dirname "$0")"
here=$PWD
proofs="$HOME/neoteletext-proofs"
ESCAPED="$proofs/escaped"
rm -f "$ESCAPED"

# The user's desktop: an X server, or a Wayland compositor.
if [ "$desktop" = x11 ]; then
    Xvfb :97 -screen 0 1280x900x24 -nolisten tcp -ac >/dev/null 2>&1 &
    DESKTOP=$!
    sleep 2
    export DISPLAY=:97
    nested_backend=x11
else
    unset DISPLAY
    config=$(mktemp)
    echo "xwayland disable" > "$config"
    WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c "$config" >/dev/null 2>&1 &
    DESKTOP=$!
    sleep 2
    WAYLAND_DISPLAY=$(ls -t "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | head -1)
    export WAYLAND_DISPLAY
    nested_backend=wayland
fi
echo "## the user's desktop: $desktop (DISPLAY=${DISPLAY:-none}, WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-none})"
desktop_sockets=$(ls /tmp/.X11-unix 2>/dev/null | sed 's|^|/tmp/.X11-unix/|'; ls "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | sed "s|^|$XDG_RUNTIME_DIR/|")

RULES=(-r /usr -r /etc -r /lib -r /lib64 -r /bin -r /proc -r /sys -r /dev -r /tmp/.X11-unix
       -r "$(dirname "$spike")" -r "$here" -w /dev/shm -w /dev/null -w /dev/dri
       # GTK's Wayland back end writes temporary icon files; this /tmp is the
       # private, empty one from private-mounts.sh.
       -w /tmp)
# Inside cage: forget the user's display, then confine. private-mounts.sh
# passes through only the socket WAYLAND_DISPLAY names, which cage set.
inside=(env -u DISPLAY GDK_BACKEND=wayland unshare --user --map-root-user --net --mount "$here/private-mounts.sh"
        "$here/landlock_host" "${RULES[@]}" --)
if [ "$compositor" = cage ]; then
    nested() { WLR_BACKENDS=$nested_backend WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 timeout 40 cage -- "$@"; }
else
    display_host=$(cd "$(dirname "$compositor")" && pwd)/$(basename "$compositor")
    # The test machine has an old libxkbcommon (0.8.0) in /usr/local that
    # shadows the distribution's, while libxkbcommon-x11 is the
    # distribution's; keymaps made by one and freed by the other crash the
    # display host on exit. The distribution's library is loaded first.
    nested() { LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu timeout 40 "$display_host" -- "$@"; }
fi
echo "## the nested display's compositor: $compositor"

echo "## 1. The probe page, confined, on the nested display"
nested "${inside[@]}" "$spike" 2>&1 | grep -E '^PROBE (own asset|fetch|WebRTC:)|landlock_host' | sed 's/^/  /'

echo "## 2. What the confined process can reach"
# The lists are passed as arguments: the confined process cannot read the home directory.
nested "${inside[@]}" /bin/sh -c "echo \"  the user's display sockets ($(echo $desktop_sockets)) it can reach: \$(/usr/bin/python3 -I $here/socket_reach.py $(echo $desktop_sockets))\";
    echo \"  the nested display's socket \$XDG_RUNTIME_DIR/\$WAYLAND_DISPLAY: \$(/usr/bin/python3 -I $here/socket_reach.py \$XDG_RUNTIME_DIR/\$WAYLAND_DISPLAY)\";
    $here/wayland_probe | sed 's/^host/  the nested compositor/'" 2>/dev/null

echo "## 3. All of the user's listening local sockets it can reach"
listening=$(ss -xlH | awk '{print $5}' | grep '^/' | sort -u)
nested "${inside[@]}" /usr/bin/python3 -I "$here/socket_reach.py" $listening 2>/dev/null | sed "s/^/  of $(echo "$listening" | wc -l): /"

if [ "$desktop" = x11 ]; then
    echo "## 4. Typing into the user's terminal through the user's X display"
    xterm -e bash --norc &
    TERMINAL=$!
    sleep 2
    nested "${inside[@]}" /bin/sh -c "DISPLAY=:97 xdotool search --class xterm windowfocus --sync && DISPLAY=:97 xdotool type 'touch $ESCAPED
'" 2>&1 | grep -v landlock_host | sed 's/^/  /'
    sleep 2
    echo "  command ran in the user's terminal: $([ -e "$ESCAPED" ] && echo YES || echo no)"
    kill "$TERMINAL" 2>/dev/null
fi
echo "## 5. Typing into the window host's own strip through the nested compositor's virtual keyboard"
# strip_standin plays the window host's address field; virtual_type plays a
# compromised web content process, which shares the window host's
# confinement and so holds the nested compositor's socket too.
nested "${inside[@]}" /bin/sh -c "$here/strip_standin & sleep 2; $here/virtual_type attacker.example; wait" 2>&1 \
    | grep -E 'STRIP INPUT|no strip input|virtual_type' | sed 's/^/  /'

if [ "$compositor" != cage ] && [ "$desktop" = x11 ]; then
    # The display host's window is the only one on the user's display, at
    # its top left corner; the strip is its top 40 rows (strip.rs).
    echo "## 6. A window on the nested display asking for full screen at the top left"
    (nested "${inside[@]}" "$here/cover_attempt" > /dev/null 2>&1 &)
    sleep 4
    echo "  on screen in the strip's area (400,20): $(xwd -root -silent | convert xwd:- -format '%[pixel:p{400,20}]' info:)  (the strip is srgb(0,51,153))"
    echo "  on screen below it (400,200): $(xwd -root -silent | convert xwd:- -format '%[pixel:p{400,200}]' info:)"
    sleep 6

    echo "## 7. The user's own keyboard and pointer (on the user's display)"
    nested "${inside[@]}" "$here/strip_standin" > "$proofs/host-output.txt" 2>&1 &
    HOST=$!
    sleep 3
    # The test display has no window manager to give the clicked window the
    # keyboard, as a desktop's does, so it is given directly.
    xdotool search --class display-host-spike windowfocus --sync
    xdotool mousemove 400 20 click 1 type --delay 20 'user.example'
    xdotool key Return
    sleep 1
    xdotool mousemove 400 200 click 1 type --delay 20 'typed by the user'
    xdotool key Return
    wait "$HOST" 2>/dev/null
    grep -E 'STRIP INPUT' "$proofs/host-output.txt" | sed 's/^/  /'
    rm -f "$proofs/host-output.txt"
fi

kill "$DESKTOP" 2>/dev/null
wait 2>/dev/null
rm -f "$ESCAPED" "${config:-}"
