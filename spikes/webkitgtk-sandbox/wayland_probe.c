/*
 * Connects to the Wayland compositor named by $WAYLAND_DISPLAY and lists the
 * globals it advertises that would let a client reach beyond its own
 * windows: draw above every window, type or point into other programs, read
 * the screen, or read and set the clipboard.
 * Built two ways: as a program, and with AS_WEB_EXTENSION as a WebKitGTK web
 * process extension, so that the attempt runs inside the sandboxed web
 * content process itself.
 */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>

/* Protocol interface names, from each protocol's XML definition. */
static const char *const REACHING_PROTOCOLS[] = {
    "zwlr_layer_shell_v1",                 /* surfaces above every window */
    "zwp_virtual_keyboard_manager_v1",     /* key presses into the focused program */
    "zwlr_virtual_pointer_manager_v1",     /* pointer movement and clicks */
    "zwlr_screencopy_manager_v1",          /* copies of the screen */
    "ext_image_copy_capture_manager_v1",   /* copies of the screen or of windows */
    "zwlr_data_control_manager_v1",        /* the clipboard, without focus */
    "ext_data_control_manager_v1",         /* the same, standardised */
};
#define REACHING_COUNT (sizeof REACHING_PROTOCOLS / sizeof REACHING_PROTOCOLS[0])

static int globals_seen;
static int reaching_seen[REACHING_COUNT];

static void on_global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data; (void)registry; (void)name; (void)version;
    globals_seen++;
    for (size_t index = 0; index < REACHING_COUNT; index++)
        if (strcmp(interface, REACHING_PROTOCOLS[index]) == 0) reaching_seen[index] = 1;
}

static void on_global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}

static const struct wl_registry_listener registry_listener = {on_global, on_global_remove};

static int wayland_probe(const char *where) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) {
        printf("%s: cannot connect to the Wayland compositor\n", where);
        return 1;
    }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);
    printf("%s: CONNECTED to the Wayland compositor; %d globals; offered:", where, globals_seen);
    for (size_t index = 0; index < REACHING_COUNT; index++)
        if (reaching_seen[index]) printf(" %s", REACHING_PROTOCOLS[index]);
    printf("\n");
    wl_registry_destroy(registry);
    wl_display_disconnect(display);
    return 0;
}

#ifdef AS_WEB_EXTENSION
#include <webkit2/webkit-web-extension.h>

G_MODULE_EXPORT void webkit_web_extension_initialize(WebKitWebExtension *extension);
G_MODULE_EXPORT void webkit_web_extension_initialize(WebKitWebExtension *extension) {
    (void)extension;
    char where[64];
    snprintf(where, sizeof where, "inside the web process %d", (int)getpid());
    wayland_probe(where);
    fflush(stdout);
}
#else
int main(void) {
    return wayland_probe("host");
}
#endif
