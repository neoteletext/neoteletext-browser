/*
 * Proof B on Linux: does WebKitGTK's own web process sandbox, when enabled,
 * cut the web content process off from the display, the network and files?
 * Usage: sandboxed_view on|off [web extension directory]
 * (the window closes itself after a few seconds)
 */
#include <string.h>
#include <gtk/gtk.h>
#include <webkit2/webkit2.h>

#define WINDOW_WIDTH 800
#define WINDOW_HEIGHT 400
#define LIFETIME_SECONDS 8

static gboolean quit_soon(gpointer unused) {
    (void)unused;
    gtk_main_quit();
    return G_SOURCE_REMOVE;
}

int main(int argc, char **argv) {
    gboolean sandbox = argc > 1 && strcmp(argv[1], "on") == 0;
    gtk_init(&argc, &argv);

    WebKitWebContext *context = webkit_web_context_new();
    /* Must be set before the context starts any web process. */
    webkit_web_context_set_sandbox_enabled(context, sandbox);
    if (argc > 2) webkit_web_context_set_web_extensions_directory(context, argv[2]);

    GtkWidget *window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_default_size(GTK_WINDOW(window), WINDOW_WIDTH, WINDOW_HEIGHT);
    GtkWidget *view = webkit_web_view_new_with_context(context);
    gtk_container_add(GTK_CONTAINER(window), view);
    webkit_web_view_load_html(WEBKIT_WEB_VIEW(view),
        "<!doctype html><body style='background:#222;color:#fff'>sandbox test</body>", NULL);
    gtk_widget_show_all(window);

    g_print("sandbox %s\n", sandbox ? "on" : "off");
    g_timeout_add_seconds(LIFETIME_SECONDS, quit_soon, NULL);
    gtk_main();
    return 0;
}
