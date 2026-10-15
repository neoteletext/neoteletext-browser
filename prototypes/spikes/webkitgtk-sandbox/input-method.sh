#!/usr/bin/env bash
# Decision 32, input methods: the user's input method talks to the display
# host only; the confined window host receives composed text through the
# display host's text-input protocol, and has no input method service of its
# own to reach. Uses fcitx5 with its Hangul engine, in which the keys g, k, s
# compose 한 (two-set layout) whatever the dictionary, on an X11 desktop.
# Usage: input-method.sh <display host program>
set -uo pipefail
display_host=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
cd "$(dirname "$0")"
here=$PWD
proofs="$HOME/t42-proofs"
output="$proofs/input-method-output.txt"

Xvfb :97 -screen 0 1280x900x24 -nolisten tcp -ac >/dev/null 2>&1 &
DESKTOP=$!
sleep 2
export DISPLAY=:97

# The user's input method, with a private configuration: Hangul active by default.
config=$(mktemp -d)
mkdir -p "$config/fcitx5"
cat > "$config/fcitx5/profile" <<'PROFILE'
[Groups/0]
Name=Default
Default Layout=us
DefaultIM=hangul

[Groups/0/Items/0]
Name=hangul
Layout=

[GroupOrder]
0=Default
PROFILE
printf '[Behavior]\nActiveByDefault=True\n' > "$config/fcitx5/config"
# fcitx5 also crashes with the machine's old libxkbcommon (see nested-host.sh).
LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu XDG_CONFIG_HOME=$config fcitx5 >/dev/null 2>&1 &
INPUT_METHOD=$!
# Ready once its X input method server is registered and the engine loaded.
for _ in $(seq 30); do
    xprop -root XIM_SERVERS 2>/dev/null | grep -q fcitx && break
    sleep 0.5
done
sleep 2
echo "## the user's input method: $(xprop -root XIM_SERVERS)"
export XMODIFIERS=@im=fcitx

RULES=(-r /usr -r /etc -r /lib -r /lib64 -r /bin -r /proc -r /sys -r /dev -r /tmp/.X11-unix
       -r "$here" -w /dev/shm -w /dev/null -w /dev/dri -w /tmp)
# GTK's own Wayland input method module uses the compositor's text-input
# protocol; no input method service is reachable inside the confinement.
inside=(env -u DISPLAY -u XMODIFIERS GDK_BACKEND=wayland GTK_IM_MODULE=wayland
        unshare --user --map-root-user --net --mount "$here/private-mounts.sh" "$here/landlock_host" "${RULES[@]}" --)
# See nested-host.sh: the distribution's libxkbcommon is loaded first.
LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu timeout 30 "$display_host" -- "${inside[@]}" "$here/strip_standin" > "$output" 2>&1 &
HOST=$!
sleep 4
xdotool search --class display-host-spike windowfocus --sync
echo "## typing g k s, then Enter, in the strip"
xdotool mousemove 400 20 click 1
sleep 0.5
xdotool type --delay 80 'gks'
sleep 0.5
xdotool key Return
sleep 1
echo "## typing g k s, then Enter, in the confined window host's text field"
xdotool mousemove 400 200 click 1
sleep 0.5
xdotool type --delay 80 'gks'
sleep 0.5
xdotool key Return
wait "$HOST" 2>/dev/null
grep -E 'STRIP INPUT' "$output" | sed 's/^/  /'
kill "$INPUT_METHOD" "$DESKTOP" 2>/dev/null
wait 2>/dev/null
rm -rf "$config" "$output"
