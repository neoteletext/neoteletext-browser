#!/usr/bin/env bash
# Option 1 of docs/proofs.md section 6.7, measured: the Tauri window host
# confined from outside, with
#   - an empty network namespace,
#   - private /run and /tmp holding only the display's socket (private-mounts.sh),
#   - a Landlock rule set granting no home directory and no data directory.
# Does the webview still work, are WebKitGTK's processes confined with it,
# and what can a confined process still reach?
# Usage: confined-host.sh <the sandbox-network spike's directory>
set -uo pipefail
spike="$(cd "$1" && pwd)/src-tauri/target/release/sandbox-network-spike"
cd "$(dirname "$0")"
here=$PWD
DISPLAY_NUMBER=:97
proofs="$HOME/t42-proofs"
SECRET="$proofs/secret.txt"
ESCAPED="$proofs/escaped"
echo "a file the window host must not read" > "$SECRET"
rm -f "$ESCAPED"
Xvfb "$DISPLAY_NUMBER" -screen 0 1280x900x24 -nolisten tcp -ac >/dev/null 2>&1 &
XVFB=$!
sleep 2
export DISPLAY=$DISPLAY_NUMBER
# Read-only system directories, the display socket and the programs run;
# writable only shared memory, the null device and the graphics devices.
RULES=(-r /usr -r /etc -r /lib -r /lib64 -r /bin -r /proc -r /sys -r /dev -r /tmp/.X11-unix
       -r "$(dirname "$spike")" -r "$here" -w /dev/shm -w /dev/null -w /dev/dri)
landlock=("$here/landlock_host" "${RULES[@]}" --)
network_only=(unshare --user --map-root-user --net)
# Unconfined runs get a private session bus: GTK waits for ever on a session
# bus that does not answer, and confined runs have none to wait on.
private_bus=(dbus-run-session --)
full=(unshare --user --map-root-user --net --mount "$here/private-mounts.sh")

echo "## 1. The probe page, fully confined"
timeout 40 "${full[@]}" "${landlock[@]}" "$spike" 2>&1 | grep -E '^PROBE|landlock_host' | sed 's/^/  /'

echo "## 2. open() of a home file, called inside the web content process (descriptor, errno; 13 is EACCES)"
for mode in "network namespace only" "fully confined"; do
    if [ "$mode" = "fully confined" ]; then wrapper=("${full[@]}" "${landlock[@]}"); else wrapper=("${network_only[@]}" "${private_bus[@]}"); fi
    (timeout 30 "${wrapper[@]}" "$spike" >/dev/null 2>&1 &)
    sleep 1.5
    process=$(pgrep -f '^/usr/lib/.*/WebKitWebProcess ' | head -1)
    result=$(sudo gdb -batch -p "$process" -ex "print (int)open(\"$SECRET\", 0)" -ex 'print *(int*)__errno_location()' 2>/dev/null \
        | grep -E '^\$' | cut -d' ' -f3 | tr '\n' ' ')
    echo "  $mode: ${result% }"
    pkill -x sandbox-network
    sleep 1
done

echo "## 3. Local services a confined process can reach by socket path"
ss -xlH | awk '{print $5}' | grep '^/' | sort -u > "$proofs/sockets.txt"
for mode in "Landlock and network namespace" "fully confined"; do
    if [ "$mode" = "fully confined" ]; then wrapper=("${full[@]}"); else wrapper=("${network_only[@]}"); fi
    echo "  $mode, of $(wc -l < "$proofs/sockets.txt") listening: $("${wrapper[@]}" "${landlock[@]}" /usr/bin/python3 -I socket_reach.py < "$proofs/sockets.txt" 2>/dev/null | cut -c1-160)"
    "${wrapper[@]}" "${landlock[@]}" /usr/bin/systemd-run --user --wait --quiet /usr/bin/touch "$ESCAPED" >/dev/null 2>&1
    echo "    asking the user's systemd to run a program outside: $([ -e "$ESCAPED" ] && echo SUCCEEDED || echo failed)"
    rm -f "$ESCAPED"
done

echo "## 4. Typing into another program's window through the display (X11)"
# A terminal of the user's, outside every confinement, as on a desktop.
xterm -e bash --norc &
TERMINAL=$!
sleep 2
# Focus is set directly (there is no window manager), then the keys are sent
# through the XTEST extension, as a real keyboard's would be.
"${full[@]}" "${landlock[@]}" /bin/sh -c "xdotool search --class xterm windowfocus --sync && xdotool type --delay 20 'touch $ESCAPED
'" 2>/dev/null
sleep 2
echo "  fully confined process typed a command into the terminal; command ran: $([ -e "$ESCAPED" ] && echo YES || echo no)"
kill "$TERMINAL" "$XVFB" 2>/dev/null
wait 2>/dev/null
rm -f "$SECRET" "$ESCAPED"
