#!/usr/bin/env bash
# The timed test between the browser's real processes (neoteletext-common's
# page-rules.md RULE-66; REQ-6535): the interface, the trusted core, a
# confined decoder and the sound helper as four processes on their real
# pipes, with the music player's page 101 open, keys pressed through XTEST,
# and six sets of figures: each change of DEC-6580 and DEC-6635 switched
# off, then on, one after the other on the same machine.
#
# It never touches the person's desktop or profile (DEVELOPMENT.md section
# 15): a private Xvfb display, a private PipeWire whose only output is a
# null sink, and a scratch HOME with every XDG folder in it, all under the
# run folder given, which must be a short path outside /tmp (the sound
# server's and the broker's sockets) and is emptied first. Every process is
# stopped by its own PID.
#
#   scripts/timed-test.sh RUN_FOLDER SERVER_BINARY MUSIC_PLAYER_FOLDER [KEYS]
#
# SERVER_BINARY is neoteletext-app-server's teletext-server at program
# protocol 15; MUSIC_PLAYER_FOLDER a checkout of
# neoteletext-server-channel-musicplayer at the second form, its node
# modules installed. The browser is built here with the feature
# `timed-test`, into CARGO_TARGET_DIR, which must lie under
# ~/devel/neoteletext-browser/**/native/target for the development
# AppArmor profile to let its helpers confine themselves
# (scripts/apparmor-development.sh). Linux only: the marks read the
# monotonic clock from the standard library's form of an Instant there.
#
# KEY_RECT and METER_RECT, in the display's pixels, are where page 101
# draws the key A's pressed look and the meter's lowest segment in the
# window at the display's top left; the defaults are those of the window
# this script opens.
set -euo pipefail

run=${1:?run folder}
server=${2:?the server binary}
player=${3:?the folder of the music player}
keys=${4:-200}
here=$(cd "$(dirname "$0")/.." && pwd)
display=${TIMED_DISPLAY:-:91}
port=${TIMED_PORT:-8391}
key_rect=${KEY_RECT:-585,560,635,625}
meter_rect=${METER_RECT:-1163,633,1210,655}
node_bin=${NODE_BIN:-$(dirname "$(command -v node)")}
target=${CARGO_TARGET_DIR:-$here/native/target}

for tool in Xvfb pipewire wireplumber pw-cli wpctl xdotool python3; do
    command -v "$tool" >/dev/null || { echo "timed-test: $tool is needed and is not installed" >&2; exit 1; }
done

echo "== building the browser with the timed test's marks and settings"
(cd "$here/native" && cargo build --release --bins -p neoteletext -p neoteletext-decoder -p neoteletext-interface -p neoteletext-sound -p neoteletext-fetcher -p neoteletext-keyboard \
    --features neoteletext/timed-test,neoteletext-decoder/timed-test,neoteletext-interface/timed-test,neoteletext-sound/timed-test)
browser="$target/release/neoteletext"

rm -rf "$run"
mkdir -p "$run"/{h,r,fb,log,state}
chmod 700 "$run/r"
pids=()
stop() {
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
}
trap stop EXIT
clean_env=(env -i PATH=/usr/bin:/bin:"$node_bin" HOME="$run/h" XDG_CONFIG_HOME="$run/h/.config" XDG_DATA_HOME="$run/h/.local/share" XDG_RUNTIME_DIR="$run/r")

Xvfb "$display" -screen 0 1600x1000x24 -fbdir "$run/fb" -nolisten tcp >"$run/log/xvfb.log" 2>&1 &
pids+=($!)
"${clean_env[@]}" pipewire >"$run/log/pipewire.log" 2>&1 &
pids+=($!)
until [ -S "$run/r/pipewire-0" ]; do sleep 0.1; done
"${clean_env[@]}" wireplumber >"$run/log/wireplumber.log" 2>&1 &
pids+=($!)
until "${clean_env[@]}" wpctl status >/dev/null 2>&1; do sleep 0.2; done
# The one output: a null sink, which plays nothing anywhere.
"${clean_env[@]}" pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=timed node.description=timed media.class=Audio/Sink object.linger=true audio.position=[FL FR] audio.rate=48000 }' >/dev/null
until "${clean_env[@]}" wpctl status 2>/dev/null | grep -q '\* *[0-9]*\. timed'; do sleep 0.2; done

cat >"$run/channels.json" <<EOF
{"channels": [{"folder": "musicplayer", "name": "Music player", "command": ["node", "src/main.ts"], "directory": "$player"}]}
EOF
"${clean_env[@]}" "$server" --listen 127.0.0.1:$port --origin http://127.0.0.1:$port --channels "$run/channels.json" --state "$run/state" --log-format text >"$run/log/server.log" 2>&1 &
pids+=($!)
until grep -q . "$run/log/server.log" 2>/dev/null && (echo >/dev/tcp/127.0.0.1/$port) 2>/dev/null; do sleep 0.2; done

# One run of the browser with the test's settings `$1` (comma separated,
# none for every change on), labelled `$2`.
measure() {
    local settings=$1 label=$2 check=$3 log="$run/log/browser-$2.log"
    "${clean_env[@]}" DISPLAY="$display" NEOTELETEXT_TIMED_TEST="$settings" "$browser" --no-keychain --allow-local 127.0.0.1 "http://127.0.0.1:$port/musicplayer/101" >"$log" 2>&1 &
    local pid=$!
    # The window opens, is given the focus (Xvfb has no window manager),
    # and the page and its table arrive.
    local window=""
    until [ -n "$window" ]; do sleep 0.5; window=$(DISPLAY="$display" xdotool search --onlyvisible --class neoteletext 2>/dev/null | head -1 || true); done
    DISPLAY="$display" xdotool windowmove "$window" 0 0 windowsize "$window" 1280 900 windowfocus "$window" mousemove 1500 950
    sleep 8
    python3 "$here/scripts/timed-test.py" "$display" "$log" "$label" "$keys" "$key_rect" "$meter_rect" $check || status=1
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    rm -rf "$run/h/.config/neoteletext" "$run/h/.local/share/neoteletext"
}

status=0
date
uname -n
"${clean_env[@]}" wpctl status | grep -A1 'Sinks:' | tail -1
for setting in events-at-the-tick usual-device-buffer unlocked; do
    measure "$setting" "$setting-off" ""
    measure "" "$setting-on" --check
done
exit $status
