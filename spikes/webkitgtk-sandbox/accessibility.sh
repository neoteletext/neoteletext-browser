#!/usr/bin/env bash
# Decision 32, screen readers: the display host publishes the window's
# accessibility tree itself, outside the window host's confinement, from
# state the trusted core holds; the confined window host is never on the
# accessibility bus. Read here as a screen reader reads it.
# Usage: accessibility.sh <display host program>   (run inside dbus-run-session)
set -uo pipefail
display_host=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
cd "$(dirname "$0")"
here=$PWD
proofs="$HOME/t42-proofs"
output="$proofs/accessibility-output.txt"
PAGE_TEXT="P100 T42 NEWS  Headlines: teletext returns to the web"

/usr/libexec/at-spi-bus-launcher --launch-immediately >/dev/null 2>&1 &
BUS=$!
sleep 2
# A screen reader announces itself this way when it starts (Orca does), and
# applications publish their trees only then.
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status ScreenReaderEnabled b true
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true
Xvfb :97 -screen 0 1280x900x24 -nolisten tcp -ac >/dev/null 2>&1 &
DESKTOP=$!
sleep 2
export DISPLAY=:97

RULES=(-r /usr -r /etc -r /lib -r /lib64 -r /bin -r /proc -r /sys -r /dev -r /tmp/.X11-unix
       -r "$here" -w /dev/shm -w /dev/null -w /dev/dri -w /tmp)
inside=(env -u DISPLAY GDK_BACKEND=wayland
        unshare --user --map-root-user --net --mount "$here/private-mounts.sh" "$here/landlock_host" "${RULES[@]}" --)
# See nested-host.sh: the distribution's libxkbcommon is loaded first.
LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu timeout 90 "$display_host" --page-text "$PAGE_TEXT" -- "${inside[@]}" "$here/strip_standin" 80 > "$output" 2>&1 &
HOST=$!
sleep 4
xdotool search --class display-host-spike windowfocus --sync

echo "## 1. The accessibility bus, read from outside"
python3 -I accessibility_probe.py
echo "## 2. The user types into the strip; read again"
xdotool mousemove 400 20 click 1 type --delay 20 'teletext.example'
sleep 1
python3 -I accessibility_probe.py | grep -E "application|text_input|Address"
echo "## 3. The user moves to the page; the screen reader asks for the strip"
xdotool mousemove 400 200 click 1
sleep 1
python3 -I accessibility_probe.py --focus Address | grep -E "asking|Address"
sleep 1
grep -E "ACCESSIBILITY ACTION" "$output" | sed 's/^/  display host: /'
python3 -I accessibility_probe.py | grep -E "Address"
echo "## 4. Orca, the screen reader itself: what it would say as the user moves"
orca_config=$(mktemp -d)
orca_log="$proofs/orca.log"
# Orca also crashes with the machine's old libxkbcommon (see nested-host.sh);
# its settings go to a private directory.
LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu XDG_CONFIG_HOME=$orca_config timeout 20 orca --replace --debug-file "$orca_log" >/dev/null 2>&1 &
ORCA=$!
# Orca is ready once it has said so in its log.
for _ in $(seq 40); do
    grep -a -q "Screen reader on" "$orca_log" 2>/dev/null && break
    sleep 0.5
done
# A second listener alongside Orca, for the same moves.
python3 -I accessibility_probe.py --listen 9 | sed 's/^/  (alongside Orca)/' &
ALONGSIDE=$!
sleep 2
xdotool mousemove 400 200 click 1
sleep 3
xdotool mousemove 400 20 click 1
sleep 3
wait "$ALONGSIDE"
kill "$ORCA" 2>/dev/null
grep -a -o -E "SPEECH OUTPUT: .[^{]*" "$orca_log" | sed "s/^/  /" | head -20
cp "$orca_log" "$proofs/orca-last.log"
rm -rf "$orca_config" "$orca_log"
echo "## 4b. Focus events, as a screen reader receives them, while the user moves"
python3 -I accessibility_probe.py --listen 6 &
LISTENER=$!
sleep 2
xdotool mousemove 400 200 click 1
sleep 2
xdotool mousemove 400 20 click 1
wait "$LISTENER"
echo "## 5. Is the confined window host on the accessibility bus?"
echo "  confined program's process: $(pgrep -x strip_standin | tr '\n' ' ')"
python3 -I accessibility_probe.py | grep -E "^application"
echo "  display host still running: $(kill -0 "$HOST" 2>/dev/null && echo yes || echo no)"
grep -v -E 'STRIP|ACCESSIBILITY' "$output" | grep -i -E 'panic|error|fault|abort' | head -5 | sed 's/^/  display host: /'
kill "$HOST" "$DESKTOP" "$BUS" 2>/dev/null
wait 2>/dev/null
rm -f "$output"
