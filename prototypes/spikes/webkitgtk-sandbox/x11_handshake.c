/*
 * Performs the X11 connection handshake (X Window System Protocol, chapter
 * "Connection Setup") over a display's Unix socket, authenticating with the
 * MIT-MAGIC-COOKIE-1 entry from $XAUTHORITY. Prints whether the server
 * accepted the connection.
 * Built two ways: as a program (usage: x11_handshake <socket path>), and with
 * AS_WEB_EXTENSION as a WebKitGTK web process extension, so that the attempt
 * runs inside the sandboxed web content process itself, under its seccomp
 * filter and namespaces, as code that had taken over that process would.
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#define COOKIE_NAME "MIT-MAGIC-COOKIE-1"
#define MAX_FIELD 1024
#define SETUP_ACCEPTED 1

static int read_counted(FILE *file, unsigned char *out, size_t *length) {
    int high = fgetc(file), low = fgetc(file);
    if (high == EOF || low == EOF) return -1;
    *length = (size_t)((high << 8) | low);
    if (*length > MAX_FIELD) return -1;
    return fread(out, 1, *length, file) == *length ? 0 : -1;
}

/* Reads the first MIT-MAGIC-COOKIE-1 entry of an Xauthority file. */
static int read_cookie(const char *path, unsigned char *cookie, size_t *cookie_length) {
    FILE *file = fopen(path, "rb");
    if (!file) return -1;
    unsigned char field[MAX_FIELD];
    size_t length;
    while (fgetc(file) != EOF && fgetc(file) != EOF) { /* family */
        unsigned char name[MAX_FIELD];
        size_t name_length;
        if (read_counted(file, field, &length) || read_counted(file, field, &length) ||
            read_counted(file, name, &name_length) || read_counted(file, cookie, cookie_length)) break;
        if (name_length == strlen(COOKIE_NAME) && memcmp(name, COOKIE_NAME, name_length) == 0) {
            fclose(file);
            return 0;
        }
    }
    fclose(file);
    return -1;
}

static size_t pad4(size_t length) { return (4 - length % 4) % 4; }

static int x11_handshake(const char *socket_path) {
    const char *authority = getenv("XAUTHORITY");
    unsigned char cookie[MAX_FIELD];
    size_t cookie_length = 0;
    if (!authority || read_cookie(authority, cookie, &cookie_length)) {
        printf("no usable cookie\n");
        return 1;
    }
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un address = {.sun_family = AF_UNIX};
    strncpy(address.sun_path, socket_path, sizeof address.sun_path - 1);
    if (fd < 0 || connect(fd, (struct sockaddr *)&address, sizeof address) != 0) {
        printf("connect to %s failed: %s\n", socket_path, strerror(errno));
        return 1;
    }
    /* 'l' (little-endian), version 11.0, then authorisation name and data. */
    unsigned char setup[12 + MAX_FIELD * 2] = {'l', 0, 11, 0, 0, 0};
    size_t name_length = strlen(COOKIE_NAME);
    setup[6] = (unsigned char)name_length;
    setup[8] = (unsigned char)cookie_length;
    size_t offset = 12;
    memcpy(setup + offset, COOKIE_NAME, name_length);
    offset += name_length + pad4(name_length);
    memcpy(setup + offset, cookie, cookie_length);
    offset += cookie_length + pad4(cookie_length);
    unsigned char reply = 0;
    if (write(fd, setup, offset) != (ssize_t)offset || read(fd, &reply, 1) != 1) {
        printf("handshake failed\n");
        return 1;
    }
    printf(reply == SETUP_ACCEPTED ? "X server ACCEPTED the connection\n" : "X server refused the connection\n");
    close(fd);
    return reply == SETUP_ACCEPTED ? 0 : 1;
}

#ifdef AS_WEB_EXTENSION
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <webkit2/webkit-web-extension.h>

#define SOCKET_DIRECTORY "/tmp/.X11-unix/X"
/* Where and how large a browser's trusted strip might be. */
#define FAKE_STRIP_WIDTH 800
#define FAKE_STRIP_HEIGHT 40
#define FAKE_STRIP_TITLE "fake trusted strip"

/* The host maps its own window after the web process starts, so the fake
 * strip raises itself again once the host's window is up. */
#define RAISE_AFTER_SECONDS 1

/* Kept open for the life of the process, so the window stays mapped. */
static Display *fake_strip_display;
static Window fake_strip_window;

static gboolean raise_fake_strip(gpointer unused) {
    (void)unused;
    XRaiseWindow(fake_strip_display, fake_strip_window);
    XSync(fake_strip_display, False);
    printf("inside the web process: raised the fake strip above every other window\n");
    fflush(stdout);
    return G_SOURCE_REMOVE;
}

/* An override-redirect window is placed and stacked by the client alone; the
 * window manager has no say (Inter-Client Communication Conventions Manual,
 * section 4.1.10), so nothing stops it covering another program's window. */
static void open_fake_strip(void) {
    fake_strip_display = XOpenDisplay(NULL);
    if (!fake_strip_display) {
        printf("inside the web process: XOpenDisplay failed\n");
        return;
    }
    XSetWindowAttributes attributes = {.override_redirect = True,
        .background_pixel = WhitePixel(fake_strip_display, DefaultScreen(fake_strip_display))};
    Window window = XCreateWindow(fake_strip_display, DefaultRootWindow(fake_strip_display), 0, 0,
        FAKE_STRIP_WIDTH, FAKE_STRIP_HEIGHT, 0, CopyFromParent, InputOutput, CopyFromParent,
        CWOverrideRedirect | CWBackPixel, &attributes);
    fake_strip_window = window;
    XStoreName(fake_strip_display, window, FAKE_STRIP_TITLE);
    XMapRaised(fake_strip_display, window);
    XSync(fake_strip_display, False);
    printf("inside the web process: mapped an override-redirect window \"" FAKE_STRIP_TITLE "\" at 0,0\n");
    g_timeout_add_seconds(RAISE_AFTER_SECONDS, raise_fake_strip, NULL);
}

/* WebKitGTK calls this in every web process it starts. */
G_MODULE_EXPORT void webkit_web_extension_initialize(WebKitWebExtension *extension);
G_MODULE_EXPORT void webkit_web_extension_initialize(WebKitWebExtension *extension) {
    (void)extension;
    const char *display = getenv("DISPLAY");
    char socket_path[sizeof ((struct sockaddr_un *)0)->sun_path];
    snprintf(socket_path, sizeof socket_path, SOCKET_DIRECTORY "%s", display && display[0] == ':' ? display + 1 : "");
    printf("inside the web process %d: ", (int)getpid());
    x11_handshake(socket_path);
    open_fake_strip();
    fflush(stdout);
}
#else
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    return x11_handshake(argv[1]);
}
#endif
