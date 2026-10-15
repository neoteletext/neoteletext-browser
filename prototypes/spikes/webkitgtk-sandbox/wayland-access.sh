#!/usr/bin/env bash
# Proof B on Linux under Wayland: can WebKitGTK's sandboxed web process reach
# the compositor, and is it offered the layer shell that lets a client cover
# every window? Runs sway with no screen or input devices.
set -uo pipefail
cd "$(dirname "$0")"
unset DISPLAY
config=$(mktemp)
echo "xwayland disable" > "$config"
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c "$config" >/dev/null 2>&1 &
SWAY=$!
sleep 2
WAYLAND_DISPLAY=$(ls -t "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | head -1)
export WAYLAND_DISPLAY
echo "compositor socket: $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
./wayland_probe

GDK_BACKEND=wayland ./sandboxed_view on "$PWD/wayland-extension" > view-wayland.log 2>&1 &
VIEW=$!
sleep 3
for process in $(pgrep -f '^/usr/lib/.*/WebKitWebProcess '); do
    parent=$(ps -o ppid= -p "$process" | tr -d ' ')
    [ "$(ps -o comm= -p "$parent")" = bwrap ] || continue
    echo "sandboxed web process $process"
    echo "  WAYLAND_DISPLAY in its environment: $(tr '\0' '\n' < "/proc/$process/environ" | grep '^WAYLAND_DISPLAY=' | cut -d= -f2)"
    echo "  compositor socket mounted into its sandbox: $(awk '{print $5}' "/proc/$process/mountinfo" | grep -E 'wayland' | tr '\n' ' ')"
    break
done
kill "$VIEW" "$SWAY" 2>/dev/null
wait 2>/dev/null
rm -f "$config"
grep 'web process' view-wayland.log | sed 's/^/  /'
