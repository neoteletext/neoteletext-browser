/*
 * Tries to cover everything, the trusted strip included, as a compromised
 * process on the nested display might: a window that asks for full screen
 * and asks to be placed at the top left corner.
 */
#include <gtk/gtk.h>

#define LIFETIME_SECONDS 8

static gboolean quit_soon(gpointer unused) {
    (void)unused;
    gtk_main_quit();
    return G_SOURCE_REMOVE;
}

int main(int argc, char **argv) {
    gtk_init(&argc, &argv);
    GtkWidget *window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_move(GTK_WINDOW(window), 0, 0);
    gtk_window_fullscreen(GTK_WINDOW(window));
    gtk_widget_show_all(window);
    g_print("cover_attempt: asked for full screen at 0,0\n");
    g_timeout_add_seconds(LIFETIME_SECONDS, quit_soon, NULL);
    gtk_main();
    return 0;
}
