/*
 * Types a line into whatever has keyboard focus, through the compositor's
 * virtual keyboard protocol (virtual-keyboard-unstable-v1), as a compromised
 * web content process holding the compositor's socket could.
 * Usage: virtual_type <text>   (letters, digits, '.', '/', ':' and space; Enter follows)
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include "virtual-keyboard-unstable-v1-client-protocol.h"

/* Linux evdev key codes are X key codes less 8 (xkbcommon, "keycodes"). */
#define EVDEV_OFFSET 8
#define KEY_DELAY_NANOSECONDS 20000000L

static struct wl_seat *seat;
static struct zwp_virtual_keyboard_manager_v1 *manager;

static void on_global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data; (void)version;
    if (strcmp(interface, wl_seat_interface.name) == 0)
        seat = wl_registry_bind(registry, name, &wl_seat_interface, 1);
    else if (strcmp(interface, zwp_virtual_keyboard_manager_v1_interface.name) == 0)
        manager = wl_registry_bind(registry, name, &zwp_virtual_keyboard_manager_v1_interface, 1);
}
static void on_global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}
static const struct wl_registry_listener registry_listener = {on_global, on_global_remove};

/* Finds the key that produces a key symbol at shift level 0. */
static xkb_keycode_t key_for(struct xkb_keymap *keymap, xkb_keysym_t wanted) {
    for (xkb_keycode_t key = xkb_keymap_min_keycode(keymap); key <= xkb_keymap_max_keycode(keymap); key++) {
        const xkb_keysym_t *symbols;
        if (xkb_keymap_key_get_syms_by_level(keymap, key, 0, 0, &symbols) > 0 && symbols[0] == wanted) return key;
    }
    return 0;
}

static void pause_briefly(void) {
    struct timespec delay = {0, KEY_DELAY_NANOSECONDS};
    nanosleep(&delay, NULL);
}

static void press(struct zwp_virtual_keyboard_v1 *keyboard, struct wl_display *display, xkb_keycode_t key, uint32_t *time) {
    zwp_virtual_keyboard_v1_key(keyboard, (*time)++, key - EVDEV_OFFSET, WL_KEYBOARD_KEY_STATE_PRESSED);
    zwp_virtual_keyboard_v1_key(keyboard, (*time)++, key - EVDEV_OFFSET, WL_KEYBOARD_KEY_STATE_RELEASED);
    wl_display_flush(display);
    pause_briefly();
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) { printf("virtual_type: cannot connect to the compositor\n"); return 1; }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);
    if (!seat || !manager) { printf("virtual_type: no virtual keyboard offered\n"); return 1; }

    struct xkb_context *context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    struct xkb_rule_names names = {0};
    struct xkb_keymap *keymap = xkb_keymap_new_from_names(context, &names, XKB_KEYMAP_COMPILE_NO_FLAGS);
    char *text = xkb_keymap_get_as_string(keymap, XKB_KEYMAP_FORMAT_TEXT_V1);
    size_t size = strlen(text) + 1;
    int fd = memfd_create("keymap", MFD_CLOEXEC);
    if (fd < 0 || write(fd, text, size) != (ssize_t)size) return 1;

    struct zwp_virtual_keyboard_v1 *keyboard = zwp_virtual_keyboard_manager_v1_create_virtual_keyboard(manager, seat);
    zwp_virtual_keyboard_v1_keymap(keyboard, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, fd, (uint32_t)size);
    wl_display_roundtrip(display);
    pause_briefly();

    uint32_t time = 0;
    for (const char *character = argv[1]; *character; character++) {
        xkb_keycode_t key = key_for(keymap, xkb_utf32_to_keysym((uint32_t)(unsigned char)*character));
        if (key) press(keyboard, display, key, &time);
    }
    press(keyboard, display, key_for(keymap, XKB_KEY_Return), &time);
    wl_display_roundtrip(display);
    printf("virtual_type: typed \"%s\" and Enter\n", argv[1]);
    close(fd);
    wl_display_disconnect(display);
    return 0;
}
