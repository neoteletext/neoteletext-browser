/*
 * Stands in for the trusted strip's address field: one window with one text
 * field. Whatever is entered there, followed by Enter, is printed as strip
 * input would be sent to the trusted core.
 * Usage: strip_standin [seconds to stay open]
 */
#include <stdlib.h>
#include <gtk/gtk.h>

#define WINDOW_WIDTH 600
#define WINDOW_HEIGHT 60
#define LIFETIME_SECONDS 15
#define MAX_LIFETIME_SECONDS 600

static void on_activate(GtkEntry *entry, gpointer unused) {
    (void)unused;
    g_print("STRIP INPUT received: \"%s\"\n", gtk_entry_get_text(entry));
    gtk_main_quit();
}

static gboolean quit_soon(gpointer unused) {
    (void)unused;
    g_print("no strip input arrived\n");
    gtk_main_quit();
    return G_SOURCE_REMOVE;
}

int main(int argc, char **argv) {
    gtk_init(&argc, &argv);
    long lifetime = argc > 1 ? strtol(argv[1], NULL, 10) : LIFETIME_SECONDS;
    if (lifetime <= 0 || lifetime > MAX_LIFETIME_SECONDS) lifetime = LIFETIME_SECONDS;
    GtkWidget *window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_default_size(GTK_WINDOW(window), WINDOW_WIDTH, WINDOW_HEIGHT);
    GtkWidget *entry = gtk_entry_new();
    g_signal_connect(entry, "activate", G_CALLBACK(on_activate), NULL);
    gtk_container_add(GTK_CONTAINER(window), entry);
    gtk_widget_show_all(window);
    gtk_widget_grab_focus(entry);
    g_timeout_add_seconds((guint)lifetime, quit_soon, NULL);
    gtk_main();
    return 0;
}
