/*
 * The module's own memcpy, memmove and memset (script-api.md section 3.2
 * rule 5(a); the security review's finding S-04, TODO-7270, and S-18 item
 * 1, TODO-7285).
 *
 * The C library's versions are compiled to WebAssembly's bulk-memory
 * instructions, one of which can copy the whole 64 MiB memory while the
 * interpreter charges it almost nothing and cannot be stopped inside it.
 * These are plain loops, compiled without bulk memory, and after every
 * COPY_CHECK_BYTES they ask the host to read the clock (the import
 * `nt.check`), which traps when the callback has passed its time limit.
 * No copy or fill of any size can therefore outrun the limit by more than
 * the time one such piece takes.
 *
 * Growing the memory is the one other instruction whose work depends on a
 * number the module gives: the interpreter zeroes every page it adds. The
 * C library's sbrk is therefore replaced here too, by one that grows by
 * at most GROW_CHECK_BYTES at a time with a clock check between.
 *
 * This file is compiled with -fno-builtin so that the compiler does not
 * turn these loops back into calls to themselves.
 */
#include <errno.h>
#include <stddef.h>
#include <stdint.h>

/* The bytes copied or filled between two clock checks: the host's
 * HOST_COPY_CHECK_BYTES (native/script/src/limits.rs), which the container
 * compares with this at its start through nt_limit(). */
#define COPY_CHECK_BYTES 65536

/* The most the memory grows by in one instruction: 16 pages, a mebibyte. */
#define GROW_CHECK_BYTES (1024 * 1024)
/* A WebAssembly page (WebAssembly core specification, section 4.2.8). */
#define PAGE_BYTES 65536

__attribute__((import_module("nt"), import_name("check"))) void nt_check(void);

/* The C library's allocator asks for more memory here. Grows by `increment`
 * bytes, a whole number of pages, a piece at a time; the old end of
 * memory, or -1 with errno set when the container's memory is full (what
 * was added before that stays, since memory cannot shrink, and the
 * allocator finds it by asking where memory ends). */
void *sbrk(intptr_t increment) {
    if (increment == 0) return (void *)(__builtin_wasm_memory_size(0) * PAGE_BYTES);
    if (increment < 0 || increment % PAGE_BYTES != 0) {
        errno = EINVAL;
        return (void *)-1;
    }
    uintptr_t first = SIZE_MAX;
    uintptr_t left = (uintptr_t)increment;
    while (left > 0) {
        uintptr_t piece = left > GROW_CHECK_BYTES ? GROW_CHECK_BYTES : left;
        uintptr_t old = __builtin_wasm_memory_grow(0, piece / PAGE_BYTES);
        if (old == SIZE_MAX) {
            errno = ENOMEM;
            return (void *)-1;
        }
        if (first == SIZE_MAX) first = old;
        left -= piece;
        if (left > 0) nt_check();
    }
    return (void *)(first * PAGE_BYTES);
}

int32_t nt_copy_check_bytes(void) { return COPY_CHECK_BYTES; }

typedef uint64_t __attribute__((may_alias, aligned(1))) word;

static void forward(unsigned char *to, const unsigned char *from, size_t count) {
    for (; count >= sizeof(word); count -= sizeof(word), to += sizeof(word), from += sizeof(word)) *(word *)to = *(const word *)from;
    for (; count; count--) *to++ = *from++;
}

static void backward(unsigned char *to, const unsigned char *from, size_t count) {
    to += count;
    from += count;
    for (; count >= sizeof(word); count -= sizeof(word)) {
        to -= sizeof(word);
        from -= sizeof(word);
        *(word *)to = *(const word *)from;
    }
    for (; count; count--) *--to = *--from;
}

void *memcpy(void *restrict target, const void *restrict source, size_t count) {
    unsigned char *to = target;
    const unsigned char *from = source;
    while (count > COPY_CHECK_BYTES) {
        forward(to, from, COPY_CHECK_BYTES);
        to += COPY_CHECK_BYTES;
        from += COPY_CHECK_BYTES;
        count -= COPY_CHECK_BYTES;
        nt_check();
    }
    forward(to, from, count);
    return target;
}

void *memmove(void *target, const void *source, size_t count) {
    unsigned char *to = target;
    const unsigned char *from = source;
    if (to == from) return target;
    if ((uintptr_t)to - (uintptr_t)from >= count) {
        /* The target starts before the source or past its end: forwards. */
        while (count > COPY_CHECK_BYTES) {
            forward(to, from, COPY_CHECK_BYTES);
            to += COPY_CHECK_BYTES;
            from += COPY_CHECK_BYTES;
            count -= COPY_CHECK_BYTES;
            nt_check();
        }
        forward(to, from, count);
    } else {
        /* The target starts inside the source: backwards, a piece at a
         * time from the end. A piece's own copy is backwards too, since
         * the two may be closer than a word. */
        while (count > COPY_CHECK_BYTES) {
            count -= COPY_CHECK_BYTES;
            backward(to + count, from + count, COPY_CHECK_BYTES);
            nt_check();
        }
        backward(to, from, count);
    }
    return target;
}

static void fill(unsigned char *to, word pattern, unsigned char byte, size_t count) {
    for (; count >= sizeof(word); count -= sizeof(word), to += sizeof(word)) *(word *)to = pattern;
    for (; count; count--) *to++ = byte;
}

void *memset(void *target, int value, size_t count) {
    unsigned char *to = target;
    unsigned char byte = (unsigned char)value;
    /* The byte in each of a word's eight places. */
    word pattern = (word)byte * UINT64_C(0x0101010101010101);
    while (count > COPY_CHECK_BYTES) {
        fill(to, pattern, byte, COPY_CHECK_BYTES);
        to += COPY_CHECK_BYTES;
        count -= COPY_CHECK_BYTES;
        nt_check();
    }
    fill(to, pattern, byte, count);
    return target;
}
