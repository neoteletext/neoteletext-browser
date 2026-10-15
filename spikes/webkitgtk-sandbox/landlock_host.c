/*
 * Starts a program under a Landlock rule set, as the window host's launcher
 * would. Landlock domains are inherited by every child (landlock(7),
 * "Inheritance"), so WebKitGTK's network and content processes are confined
 * by the same rules.
 * Usage: landlock_host [-r path]... [-w path]... -- program [arguments]
 *   -r  read, list and execute beneath path
 *   -w  every file system right beneath path
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/landlock.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#define READ_RIGHTS (LANDLOCK_ACCESS_FS_EXECUTE | LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR)
/* landlock(7): only these rights may be granted on a path that is not a directory. */
#define FILE_RIGHTS (LANDLOCK_ACCESS_FS_EXECUTE | LANDLOCK_ACCESS_FS_WRITE_FILE | LANDLOCK_ACCESS_FS_READ_FILE | \
                     LANDLOCK_ACCESS_FS_TRUNCATE | LANDLOCK_ACCESS_FS_IOCTL_DEV)
/* Every file system right of each ABI version (landlock(7), "Access rights"). */
#define ABI_1_RIGHTS ((LANDLOCK_ACCESS_FS_MAKE_SYM << 1) - 1)
#define MINIMUM_ABI 1

static uint64_t rights_for_abi(long abi) {
    uint64_t rights = ABI_1_RIGHTS;
    if (abi >= 2) rights |= LANDLOCK_ACCESS_FS_REFER;
    if (abi >= 3) rights |= LANDLOCK_ACCESS_FS_TRUNCATE;
    if (abi >= 5) rights |= LANDLOCK_ACCESS_FS_IOCTL_DEV;
    return rights;
}

static int allow(int ruleset, const char *path, uint64_t rights) {
    int fd = open(path, O_PATH | O_CLOEXEC);
    if (fd < 0) {
        fprintf(stderr, "landlock_host: skipping %s: %s\n", path, strerror(errno));
        return 0;
    }
    struct stat status;
    if (fstat(fd, &status) == 0 && !S_ISDIR(status.st_mode)) rights &= FILE_RIGHTS;
    struct landlock_path_beneath_attr rule = {.allowed_access = rights, .parent_fd = fd};
    int result = (int)syscall(SYS_landlock_add_rule, ruleset, LANDLOCK_RULE_PATH_BENEATH, &rule, 0);
    if (result != 0) fprintf(stderr, "landlock_host: rule for %s: %s\n", path, strerror(errno));
    close(fd);
    return result;
}

int main(int argc, char **argv) {
    long abi = syscall(SYS_landlock_create_ruleset, NULL, 0, LANDLOCK_CREATE_RULESET_VERSION);
    if (abi < MINIMUM_ABI) {
        fprintf(stderr, "landlock_host: Landlock unavailable: %s\n", strerror(errno));
        return 1;
    }
    uint64_t all_rights = rights_for_abi(abi);
    struct landlock_ruleset_attr attributes = {.handled_access_fs = all_rights};
    int ruleset = (int)syscall(SYS_landlock_create_ruleset, &attributes, sizeof attributes, 0);
    if (ruleset < 0) {
        perror("landlock_host: create ruleset");
        return 1;
    }
    int index = 1;
    for (; index + 1 < argc && strcmp(argv[index], "--") != 0; index += 2) {
        uint64_t rights = strcmp(argv[index], "-w") == 0 ? all_rights : READ_RIGHTS & all_rights;
        if (allow(ruleset, argv[index + 1], rights) != 0) return 1;
    }
    if (index + 1 >= argc) {
        fprintf(stderr, "usage: landlock_host [-r path]... [-w path]... -- program [arguments]\n");
        return 2;
    }
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 || syscall(SYS_landlock_restrict_self, ruleset, 0) != 0) {
        perror("landlock_host: restrict");
        return 1;
    }
    close(ruleset);
    fprintf(stderr, "landlock_host: Landlock ABI %ld enforced\n", abi);
    execvp(argv[index + 1], argv + index + 1);
    perror("landlock_host: exec");
    return 1;
}
