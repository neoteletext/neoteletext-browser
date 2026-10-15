#!/usr/bin/env bash
# Proof B on Linux: can WebKitGTK's sandboxed web process reach an X display
# that requires an authorisation cookie, as a real desktop's does?
set -uo pipefail
cd "$(dirname "$0")"
DISPLAY_NUMBER=:98
export XAUTHORITY="$HOME/.Xauthority-t42"
rm -f "$XAUTHORITY"; touch "$XAUTHORITY"
xauth -f "$XAUTHORITY" add "$DISPLAY_NUMBER" . "$(mcookie)"
Xvfb "$DISPLAY_NUMBER" -screen 0 1280x900x24 -nolisten tcp -auth "$XAUTHORITY" >/dev/null 2>&1 &
XVFB=$!
sleep 2
echo "host, with the cookie:    $(DISPLAY=$DISPLAY_NUMBER xdpyinfo 2>&1 | head -1)"
echo "host, without the cookie: $(DISPLAY=$DISPLAY_NUMBER XAUTHORITY=/nonexistent xdpyinfo 2>&1 | head -1)"

# The extension runs the handshake inside each web process it is loaded into.
DISPLAY=$DISPLAY_NUMBER ./sandboxed_view on "$PWD/extension" > view.log 2>&1 &
VIEW=$!
sleep 3
for process in $(pgrep -f '^/usr/lib/.*/WebKitWebProcess '); do
    parent=$(ps -o ppid= -p "$process" | tr -d ' ')
    [ "$(ps -o comm= -p "$parent")" = bwrap ] || continue
    cookie=$(tr '\0' '\n' < "/proc/$process/environ" | grep '^XAUTHORITY=' | cut -d= -f2)
    echo "sandboxed web process $process"
    echo "  XAUTHORITY in its environment: ${cookie:-none}"
    echo "  cookie file visible inside its sandbox: $(ls "/proc/$process/root${cookie:-/nonexistent}" 2>&1 | head -1)"
    echo "  X sockets visible inside: $(ls "/proc/$process/root/tmp/.X11-unix" 2>&1 | tr '\n' ' ')"
    echo "  sandbox mounts: $(awk '{print $5}' "/proc/$process/mountinfo" | tr '\n' ' ')"
    # What is on screen decides it: the host's page is dark grey (#222222)
    # everywhere; the fake strip is white and covers its top 40 rows.
    echo "  pixel on screen inside the strip area (400,20): $(DISPLAY=$DISPLAY_NUMBER xwd -root -silent | convert xwd:- -format '%[pixel:p{400,20}]' info:)"
    echo "  pixel on screen below it, the host's page (400,200): $(DISPLAY=$DISPLAY_NUMBER xwd -root -silent | convert xwd:- -format '%[pixel:p{400,200}]' info:)"
    echo "  the sandboxed process's window, as the X server reports it:"
    DISPLAY=$DISPLAY_NUMBER xwininfo -name "fake trusted strip" 2>&1 | grep -E 'Absolute|Width|Height|Map State|Override' | sed 's/^ */    /'
    break
done
kill "$VIEW" "$XVFB" 2>/dev/null
wait 2>/dev/null
rm -f "$XAUTHORITY"
echo "connecting to the display from inside the sandboxed web process:"
grep 'inside the web process' view.log | sed 's/^/  /'
