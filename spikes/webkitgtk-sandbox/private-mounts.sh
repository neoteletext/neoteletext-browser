#!/usr/bin/env bash
# Runs a command with /run and /tmp replaced by empty private directories
# that hold only the socket of the display in use (X11 by DISPLAY, Wayland by
# WAYLAND_DISPLAY), so that no other local service's Unix socket can be
# reached by path. Landlock does not govern connecting to a socket.
# Must run as root of a new user and mount namespace:
#   unshare --user --map-root-user --mount [--net] private-mounts.sh command...
set -euo pipefail
runtime_directory=${XDG_RUNTIME_DIR:-/run/user/$(stat -c %u "$HOME")}
x11_socket=""
if [[ ${DISPLAY:-} =~ ^:([0-9]+)$ ]]; then x11_socket=/tmp/.X11-unix/X${BASH_REMATCH[1]}; fi
wayland_socket=""
if [ -n "${WAYLAND_DISPLAY:-}" ]; then wayland_socket=$runtime_directory/$WAYLAND_DISPLAY; fi

mount --make-rprivate /
# The sockets are held while /run and /tmp are covered, then moved into
# place. (mount(8) resolves a /proc/self/fd path to its name, so an open
# descriptor cannot serve as the source.)
mount -t tmpfs -o mode=0700 held-sockets /mnt
for socket in $x11_socket $wayland_socket; do
    touch "/mnt/$(basename "$socket")"
    mount --bind "$socket" "/mnt/$(basename "$socket")"
done
mount -t tmpfs -o mode=0755 private-run /run
mkdir -p "$runtime_directory"
chmod 0700 "$runtime_directory"
mount -t tmpfs -o mode=1777 private-tmp /tmp
for socket in $x11_socket $wayland_socket; do
    mkdir -p "$(dirname "$socket")"
    touch "$socket"
    mount --move "/mnt/$(basename "$socket")" "$socket"
done
umount /mnt
unset DBUS_SESSION_BUS_ADDRESS SSH_AUTH_SOCK
exec "$@"
