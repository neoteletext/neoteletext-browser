/*
 * Connects to the Wayland compositor named by $WAYLAND_DISPLAY and lists the
 * globals it advertises, noting the layer shell protocol
 * (wlr-layer-shell-unstable-v1), which lets any client that holds it place a
 * surface above every window, as a trusted strip's imitation would need.
 * Built two ways: as a program, and with AS_WEB_EXTENSION as a WebKitGTK web
 * process extension, so that the attempt runs inside the sandboxed web
 * content process itself.
 */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>

#define LAYER_SHELL "zwlr_layer_shell_v1"

static int globals_seen;
static int layer_shell_seen;

static void on_global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data; (void)registry; (void)name; (void)version;
    globals_seen++;
    if (strcmp(interface, LAYER_SHELL) == 0) layer_shell_seen = 1;
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
    printf("%s: CONNECTED to the Wayland compositor; %d globals; " LAYER_SHELL " %s\n", where, globals_seen,
        layer_shell_seen ? "OFFERED" : "not offered");
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
