#!/usr/bin/env bash
# Builds the probe with the project's C floor of warnings.
set -euo pipefail
cc -std=c11 -Wall -Wextra -Werror -Wshadow -Wundef -O2 sandboxed_view.c -o sandboxed_view $(pkg-config --cflags --libs webkit2gtk-4.1)
cc -std=c11 -Wall -Wextra -Werror -Wshadow -Wundef -O2 -static x11_handshake.c -o x11_handshake
mkdir -p extension
cc -std=c11 -Wall -Wextra -Werror -Wshadow -Wundef -O2 -shared -fPIC -DAS_WEB_EXTENSION x11_handshake.c -o extension/libx11_handshake.so $(pkg-config --cflags --libs webkit2gtk-web-extension-4.1 x11)
cc -std=c11 -Wall -Wextra -Werror -Wshadow -Wundef -O2 wayland_probe.c -o wayland_probe $(pkg-config --cflags --libs wayland-client)
mkdir -p wayland-extension
cc -std=c11 -Wall -Wextra -Werror -Wshadow -Wundef -O2 -shared -fPIC -DAS_WEB_EXTENSION wayland_probe.c -o wayland-extension/libwayland_probe.so $(pkg-config --cflags --libs webkit2gtk-web-extension-4.1 wayland-client)
