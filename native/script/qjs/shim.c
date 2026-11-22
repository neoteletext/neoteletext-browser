/*
 * The host boundary of a channel's scripts (script-api.md sections 3.1,
 * 10.1 and 10.3; DEC-7246, DEC-7256, DEC-7287 item 3): QuickJS compiled to
 * WebAssembly, holding one runtime with at most two contexts (the channel
 * context and the page context), with two ways out, the imports `nt.call`
 * (the one host call) and `nt.check` (a clock check, mem.c), and the entry
 * points below.
 *
 * The boundary is flat: a script's `__host(op, ...)` takes whole numbers
 * and at most one payload that is already a string, a Uint8Array or an
 * Int32Array. No property of an object is read, nothing is converted with
 * `toString` or `valueOf`, no function is called and no reference is kept,
 * so no script code runs inside a host call. Every check of what the
 * numbers and bytes mean is the host's, in Rust.
 *
 * No value of one context is ever given to the other: each entry point
 * names its context, each context has its own dispatch function, and
 * nothing here stores a value of one where the other could read it.
 */
#include <malloc.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>
#include <time.h>

#include "quickjs.h"

#define NT_IMPORT(name) __attribute__((import_module("nt"), import_name(name)))
#define NT_EXPORT(name) __attribute__((export_name(name)))

/* The capacities below are the host's constants of the same meaning
 * (native/script/src/limits.rs). The container reads them through
 * nt_limit() at its start and refuses a module whose values differ. */

/* The most whole numbers one host call takes, the operation's included:
 * MAX_HOST_INTS + 1. */
#define MAX_INTS 24
/* The longest result a host call returns: MAX_RESULT_BYTES. */
#define OUT_CAPACITY (1024 * 1024)
/* The longest error text kept for the host: MAX_ERROR_BYTES. */
#define ERROR_CAPACITY 2048
/* The contexts: CONTEXT_CHANNEL and CONTEXT_PAGE. */
#define CONTEXT_CHANNEL 1
#define CONTEXT_PAGE 2
#define CONTEXTS 2
/* The most promise rejections nobody handled that are kept for one entry;
 * more are counted: MAX_KEPT_REJECTIONS. */
#define MAX_KEPT_REJECTIONS 8

int32_t nt_copy_check_bytes(void);

/* The host's one call: the context it comes from, the operation, its whole
 * numbers, its payload and what the payload is, and where to put the
 * result words and a string or byte result. */
NT_IMPORT("call")
void nt_host_call(int32_t context, int32_t op, const int32_t *ints, int32_t n_ints, const uint8_t *payload, int32_t n_payload, int32_t payload_kind,
                  int32_t *result, uint8_t *out, int32_t out_capacity);

/* What a payload is, to the host and from it. */
enum { PAYLOAD_NONE = 0, PAYLOAD_STRING = 1, PAYLOAD_BYTES = 2, PAYLOAD_INTS = 3 };
/* A host call's result words: status, what the result is, a whole-number
 * result, the length of a string or byte result. */
enum { STATUS_OK = 0, STATUS_RANGE = 1, STATUS_TYPE = 2, STATUS_ERROR = 3 };
enum { RESULT_STATUS = 0, RESULT_KIND = 1, RESULT_VALUE = 2, RESULT_LENGTH = 3, RESULT_WORDS = 4 };
enum { RESULT_UNDEFINED = 0, RESULT_INT = 1, RESULT_STRING = 2, RESULT_BYTES = 3 };
/* Bits of what nt_eval and nt_event return above the dispatch function's
 * own low bits: something was thrown and not caught; a promise was
 * rejected and nobody handled it. */
#define ENTRY_THREW (1 << 8)
#define ENTRY_REJECTED (1 << 9)
/* QuickJS's heap was full at some point in the entry. */
#define ENTRY_MEMORY (1 << 10)
/* The dispatch function's own bits, which pass through unchanged. */
#define ENTRY_DISPATCH_BITS 0xFF
/* nt_limit()'s questions. */
enum { LIMIT_INTS = 1, LIMIT_OUT = 2, LIMIT_ERROR = 3, LIMIT_COPY_CHECK = 4, LIMIT_CONTEXTS = 5 };

static uint8_t out_buffer[OUT_CAPACITY];
static char error_text[ERROR_CAPACITY];
static int32_t error_length;
static JSRuntime *runtime;
static JSContext *contexts[CONTEXTS];
static JSValue dispatch_functions[CONTEXTS];
/* Each context's function for a rejection nobody handled (the prelude's):
 * it raises `unhandledrejection` and says whether that was cancelled. */
static JSValue rejection_functions[CONTEXTS];
/* Each context's `stack` getter of Error.prototype as QuickJS made it,
 * taken before any script ran: native code that reads an error's stack
 * from where QuickJS keeps it and runs nothing of the page's. */
static JSValue stack_getters[CONTEXTS];
/* The seed QuickJS's next context takes for Math.random and its hash
 * seed, and whether one waits to be taken (gettimeofday below). */
static int64_t pending_seed;
static bool seed_waiting;
/* Minutes east of UTC the page's local time is (operation ZONE). */
static int32_t zone_minutes;
/* Rejections nobody has handled yet, as their promises, each with the
 * context it belongs to. */
static JSValue rejected_promises[MAX_KEPT_REJECTIONS];
static JSValue rejected_reasons[MAX_KEPT_REJECTIONS];
static JSContext *rejected_contexts[MAX_KEPT_REJECTIONS];
static int32_t rejected_count;
/* The bytes QuickJS has allocated and not freed, as the C library's
 * allocator sizes them: the budget report's heap figure, kept as it
 * changes because asking QuickJS for it walks the whole heap. */
static size_t heap_bytes;
/* The most it may be (QUICKJS_HEAP_BYTES), and whether an allocation was
 * refused for passing it since the last entry ended: the memory limit
 * was passed, which the host answers by stopping the container. */
static size_t heap_limit;
static bool memory_refused;

/* Whether `size` more bytes fit under the limit; notes a refusal. */
static bool fits(size_t size) {
    if (size > heap_limit || heap_bytes > heap_limit - size) {
        memory_refused = true;
        return false;
    }
    return true;
}

static void *counted_calloc(void *opaque, size_t count, size_t size) {
    (void)opaque;
    if (size && count > SIZE_MAX / size) {
        memory_refused = true;
        return NULL;
    }
    if (!fits(count * size)) return NULL;
    void *pointer = calloc(count, size);
    if (pointer) heap_bytes += malloc_usable_size(pointer);
    else memory_refused = true;
    return pointer;
}

static void *counted_malloc(void *opaque, size_t size) {
    (void)opaque;
    if (!fits(size)) return NULL;
    void *pointer = malloc(size);
    if (pointer) heap_bytes += malloc_usable_size(pointer);
    else memory_refused = true;
    return pointer;
}

static void counted_free(void *opaque, void *pointer) {
    (void)opaque;
    if (!pointer) return;
    heap_bytes -= malloc_usable_size(pointer);
    free(pointer);
}

static void *counted_realloc(void *opaque, void *pointer, size_t size) {
    (void)opaque;
    size_t before = pointer ? malloc_usable_size(pointer) : 0;
    if (size > before && !fits(size - before)) return NULL;
    void *moved = realloc(pointer, size);
    if (moved) heap_bytes = heap_bytes - before + malloc_usable_size(moved);
    else if (size) memory_refused = true;
    return moved;
}

static size_t counted_usable_size(const void *pointer) { return malloc_usable_size((void *)pointer); }

static const JSMallocFunctions counted_allocator = { counted_calloc, counted_malloc, counted_free, counted_realloc, counted_usable_size };

/*
 * QuickJS seeds a new context's Math.random and its property hash from the
 * time of day in microseconds (quickjs-ng 0.18.0 quickjs.c, JS_NewContextRaw).
 * The time a page is given is coarse and guessable, so every context
 * started in one second had one Math.random sequence (the security
 * review's finding S-13, TODO-7279). This gettimeofday, which replaces the
 * C library's for the whole module, gives the host's seed to that one read
 * and the page's coarse clock to every other.
 */
int gettimeofday(struct timeval *restrict tv, void *restrict tz) {
    (void)tz;
    if (!tv) return 0;
    if (seed_waiting) {
        seed_waiting = false;
        tv->tv_sec = (time_t)(pending_seed / 1000000);
        tv->tv_usec = (suseconds_t)(pending_seed % 1000000);
        pending_seed = 0;
        return 0;
    }
    struct timespec now;
    clock_gettime(CLOCK_REALTIME, &now);
    tv->tv_sec = now.tv_sec;
    tv->tv_usec = (suseconds_t)(now.tv_nsec / 1000);
    return 0;
}

/* Local time is UTC moved by the zone the host gave (operation ZONE): the
 * C library here has no time zone data, and QuickJS's Date reads the zone
 * through this alone. */
struct tm *localtime_r(const time_t *restrict when, struct tm *restrict out) {
    time_t moved = *when + (time_t)zone_minutes * 60;
    if (!gmtime_r(&moved, out)) return NULL;
    out->tm_gmtoff = (long)zone_minutes * 60;
    out->tm_isdst = 0;
    return out;
}

static void keep_error(const char *text, size_t length) {
    if (length > ERROR_CAPACITY) length = ERROR_CAPACITY;
    memcpy(error_text, text, length);
    error_length = (int32_t)length;
}

/* Appends `text` to `joined` at `*used`, as far as it fits. */
static void append(char *joined, size_t *used, const char *text, size_t length) {
    size_t room = ERROR_CAPACITY - *used;
    size_t take = length < room ? length : room;
    memcpy(joined + *used, text, take);
    *used += take;
}

/* Appends the own data property `name` of `object` when it is a string,
 * after `before`. Reads the property's descriptor and never calls a getter
 * (the security review's finding S-09 item 4, TODO-7275). The caller has
 * checked that `object` is an ordinary object, not a proxy. */
static void append_own_string(JSContext *ctx, JSValueConst object, const char *name, const char *before, char *joined, size_t *used) {
    JSAtom atom = JS_NewAtom(ctx, name);
    if (atom == JS_ATOM_NULL) return;
    JSPropertyDescriptor descriptor;
    int found = JS_GetOwnProperty(ctx, &descriptor, object, atom);
    JS_FreeAtom(ctx, atom);
    if (found <= 0) return;
    if (!(descriptor.flags & JS_PROP_GETSET) && JS_IsString(descriptor.value)) {
        size_t length = 0;
        /* A string converts with no script code. */
        const char *text = JS_ToCStringLen(ctx, &length, descriptor.value);
        if (text) {
            append(joined, used, before, strlen(before));
            append(joined, used, text, length);
            JS_FreeCString(ctx, text);
        }
    }
    JS_FreeValue(ctx, descriptor.value);
    JS_FreeValue(ctx, descriptor.getter);
    JS_FreeValue(ctx, descriptor.setter);
}

/* Appends an error's stack after a new line: its own `stack` when that is
 * a plain string property; QuickJS's own, read by the getter QuickJS
 * made, when the error has no `stack` of its own; and nothing when the
 * page has given it a getter. */
static void append_stack(JSContext *ctx, JSValueConst thrown, char *joined, size_t *used) {
    JSAtom atom = JS_NewAtom(ctx, "stack");
    if (atom == JS_ATOM_NULL) return;
    int found = JS_GetOwnProperty(ctx, NULL, thrown, atom);
    JS_FreeAtom(ctx, atom);
    if (found < 0) return;
    if (found > 0) {
        append_own_string(ctx, thrown, "stack", "\n", joined, used);
        return;
    }
    int32_t number = (int32_t)(intptr_t)JS_GetContextOpaque(ctx);
    if (number < CONTEXT_CHANNEL || number > CONTEXT_PAGE || !JS_IsFunction(ctx, stack_getters[number - 1])) return;
    JSValue stack = JS_Call(ctx, stack_getters[number - 1], thrown, 0, NULL);
    if (JS_IsException(stack)) {
        JS_FreeValue(ctx, JS_GetException(ctx));
        return;
    }
    if (JS_IsString(stack)) {
        size_t length = 0;
        const char *text = JS_ToCStringLen(ctx, &length, stack);
        if (text) {
            append(joined, used, "\n", 1);
            append(joined, used, text, length);
            JS_FreeCString(ctx, text);
        }
    }
    JS_FreeValue(ctx, stack);
}

/* Describes a thrown value into the kept error without running any script
 * code: the callback has ended, and a `toString` or a `stack` getter of
 * the page's would run after it (TODO-7275). An Error's own `message` and
 * `stack` are taken when they are plain string properties; a string is
 * taken as it is; anything else is named by its type. */
static void keep_thrown(JSContext *ctx, JSValueConst thrown) {
    char joined[ERROR_CAPACITY];
    size_t used = 0;
    if (JS_IsError(thrown) && !JS_IsProxy(thrown)) {
        /* The constructor's name, from the prototype when that is an
         * ordinary object holding it as a plain string. */
        JSValue prototype = JS_GetPrototype(ctx, thrown);
        if (JS_IsObject(prototype) && !JS_IsProxy(prototype)) append_own_string(ctx, prototype, "name", "", joined, &used);
        JS_FreeValue(ctx, prototype);
        if (used == 0) append(joined, &used, "Error", 5);
        append_own_string(ctx, thrown, "message", ": ", joined, &used);
        append_stack(ctx, thrown, joined, &used);
    } else if (JS_IsString(thrown)) {
        size_t length = 0;
        const char *text = JS_ToCStringLen(ctx, &length, thrown);
        static const char before[] = "a string was thrown: ";
        append(joined, &used, before, sizeof before - 1);
        if (text) {
            append(joined, &used, text, length);
            JS_FreeCString(ctx, text);
        }
    } else {
        const char *type = "an object";
        if (JS_IsUndefined(thrown)) type = "undefined";
        else if (JS_IsNull(thrown)) type = "null";
        else if (JS_IsBool(thrown)) type = "a boolean";
        else if (JS_IsNumber(thrown)) type = "a number";
        else if (JS_IsBigInt(thrown)) type = "a bigint";
        else if (JS_IsSymbol(thrown)) type = "a symbol";
        else if (JS_IsFunction(ctx, thrown)) type = "a function";
        static const char before[] = "a value was thrown: ";
        append(joined, &used, before, sizeof before - 1);
        append(joined, &used, type, strlen(type));
    }
    keep_error(joined, used);
}

/* Takes the pending exception of `ctx` and keeps its description, leaving
 * none pending. */
static void keep_exception(JSContext *ctx) {
    JSValue exception = JS_GetException(ctx);
    keep_thrown(ctx, exception);
    JS_FreeValue(ctx, exception);
}

/* The context numbered `number`, or NULL. */
static JSContext *context_of(int32_t number) {
    if (number < CONTEXT_CHANNEL || number > CONTEXT_PAGE) return NULL;
    return contexts[number - 1];
}

/* `__host(op, ...)`: whole numbers and at most one payload. */
static JSValue js_host(JSContext *ctx, JSValueConst this_value, int argc, JSValueConst *argv) {
    (void)this_value;
    int32_t ints[MAX_INTS];
    int32_t n_ints = 0;
    const uint8_t *payload = NULL;
    size_t n_payload = 0;
    int32_t payload_kind = PAYLOAD_NONE;
    const char *string = NULL;
    JSValue buffer = JS_UNDEFINED;
    JSValue failure = JS_UNDEFINED;
    bool failed = false;
    for (int index = 0; index < argc && !failed; index++) {
        JSValueConst value = argv[index];
        if (JS_IsNumber(value)) {
            double number = 0;
            /* A number converts with no script code. */
            JS_ToFloat64(ctx, &number, value);
            if (!isfinite(number) || number != trunc(number) || number < INT32_MIN || number > INT32_MAX) {
                failure = JS_ThrowTypeError(ctx, "argument %d is not a whole number between -2147483648 and 2147483647", index);
                failed = true;
            } else if (n_ints >= MAX_INTS) {
                failure = JS_ThrowRangeError(ctx, "MAX_HOST_INTS: more than %d whole numbers after the operation in one host call", MAX_INTS - 1);
                failed = true;
            } else {
                ints[n_ints++] = (int32_t)number;
            }
        } else if (payload_kind == PAYLOAD_NONE && JS_IsString(value)) {
            string = JS_ToCStringLen(ctx, &n_payload, value);
            if (!string) {
                failure = JS_EXCEPTION;
                failed = true;
            } else {
                payload = (const uint8_t *)string;
                payload_kind = PAYLOAD_STRING;
            }
        } else if (payload_kind == PAYLOAD_NONE && (JS_GetTypedArrayType(value) == JS_TYPED_ARRAY_UINT8 || JS_GetTypedArrayType(value) == JS_TYPED_ARRAY_INT32)) {
            size_t offset = 0, length = 0, element = 0;
            buffer = JS_GetTypedArrayBuffer(ctx, value, &offset, &length, &element);
            if (JS_IsException(buffer)) {
                buffer = JS_UNDEFINED;
                failure = JS_EXCEPTION;
                failed = true;
            } else {
                size_t size = 0;
                uint8_t *start = JS_GetArrayBuffer(ctx, &size, buffer);
                if (!start && size == 0 && length == 0) {
                    /* An empty array has no bytes to point at. */
                    payload = out_buffer;
                    n_payload = 0;
                    payload_kind = JS_GetTypedArrayType(value) == JS_TYPED_ARRAY_UINT8 ? PAYLOAD_BYTES : PAYLOAD_INTS;
                } else if (!start || offset > size || length > size - offset) {
                    failure = JS_ThrowTypeError(ctx, "argument %d is a detached or short typed array", index);
                    failed = true;
                } else {
                    payload = start + offset;
                    n_payload = length;
                    payload_kind = JS_GetTypedArrayType(value) == JS_TYPED_ARRAY_UINT8 ? PAYLOAD_BYTES : PAYLOAD_INTS;
                }
            }
        } else {
            failure = JS_ThrowTypeError(ctx, "argument %d is not a whole number, a string, a Uint8Array or an Int32Array, or is a second payload", index);
            failed = true;
        }
    }
    if (!failed && n_ints < 1) {
        failure = JS_ThrowTypeError(ctx, "a host call names its operation first");
        failed = true;
    }
    if (!failed && n_payload > (size_t)INT32_MAX) {
        failure = JS_ThrowRangeError(ctx, "MAX_HOST_BYTES: the payload is too long");
        failed = true;
    }
    if (failed) {
        if (string) JS_FreeCString(ctx, string);
        JS_FreeValue(ctx, buffer);
        return failure;
    }
    int32_t result[RESULT_WORDS] = { STATUS_OK, RESULT_UNDEFINED, 0, 0 };
    nt_host_call((int32_t)(intptr_t)JS_GetContextOpaque(ctx), ints[0], ints + 1, n_ints - 1, payload, (int32_t)n_payload, payload_kind, result, out_buffer,
                 OUT_CAPACITY);
    if (string) JS_FreeCString(ctx, string);
    JS_FreeValue(ctx, buffer);
    int32_t length = result[RESULT_LENGTH];
    if (length < 0) length = 0;
    if (length > OUT_CAPACITY) length = OUT_CAPACITY;
    switch (result[RESULT_STATUS]) {
    case STATUS_OK:
        switch (result[RESULT_KIND]) {
        case RESULT_INT:
            return JS_NewInt32(ctx, result[RESULT_VALUE]);
        case RESULT_STRING:
            return JS_NewStringLen(ctx, (const char *)out_buffer, (size_t)length);
        case RESULT_BYTES:
            return JS_NewUint8ArrayCopy(ctx, out_buffer, (size_t)length);
        default:
            return JS_UNDEFINED;
        }
    case STATUS_RANGE:
        return JS_ThrowRangeError(ctx, "%.*s", (int)length, (const char *)out_buffer);
    case STATUS_TYPE:
        return JS_ThrowTypeError(ctx, "%.*s", (int)length, (const char *)out_buffer);
    default:
        return JS_ThrowInternalError(ctx, "%.*s", (int)length, (const char *)out_buffer);
    }
}

/* `__setDispatch(fn)`: the prelude's one function the host's events call. */
static JSValue js_set_dispatch(JSContext *ctx, JSValueConst this_value, int argc, JSValueConst *argv) {
    (void)this_value;
    if (argc < 1 || !JS_IsFunction(ctx, argv[0])) return JS_ThrowTypeError(ctx, "__setDispatch takes a function");
    int32_t number = (int32_t)(intptr_t)JS_GetContextOpaque(ctx);
    if (!context_of(number)) return JS_ThrowTypeError(ctx, "no context");
    JS_FreeValue(ctx, dispatch_functions[number - 1]);
    dispatch_functions[number - 1] = JS_DupValue(ctx, argv[0]);
    return JS_UNDEFINED;
}

/* `__setRejection(fn)`: the prelude's function for a rejection nobody handled. */
static JSValue js_set_rejection(JSContext *ctx, JSValueConst this_value, int argc, JSValueConst *argv) {
    (void)this_value;
    if (argc < 1 || !JS_IsFunction(ctx, argv[0])) return JS_ThrowTypeError(ctx, "__setRejection takes a function");
    int32_t number = (int32_t)(intptr_t)JS_GetContextOpaque(ctx);
    if (!context_of(number)) return JS_ThrowTypeError(ctx, "no context");
    JS_FreeValue(ctx, rejection_functions[number - 1]);
    rejection_functions[number - 1] = JS_DupValue(ctx, argv[0]);
    return JS_UNDEFINED;
}

/* Forgets the rejection kept at `index`. */
static void drop_rejection(int32_t index) {
    JS_FreeValue(rejected_contexts[index], rejected_promises[index]);
    JS_FreeValue(rejected_contexts[index], rejected_reasons[index]);
    for (int32_t next = index + 1; next < rejected_count; next++) {
        rejected_promises[next - 1] = rejected_promises[next];
        rejected_reasons[next - 1] = rejected_reasons[next];
        rejected_contexts[next - 1] = rejected_contexts[next];
    }
    rejected_count--;
}

/* QuickJS tells of a promise rejected with no handler, and again when one
 * is added later. A rejection still unhandled when the entry's promise
 * jobs have all run is the page's failure (script-api.md section 3.8). */
static void rejection_tracker(JSContext *ctx, JSValueConst promise, JSValueConst reason, bool is_handled, void *opaque) {
    (void)opaque;
    if (is_handled) {
        for (int32_t index = 0; index < rejected_count; index++) {
            if (rejected_contexts[index] == ctx && JS_IsStrictEqual(ctx, rejected_promises[index], promise)) {
                drop_rejection(index);
                return;
            }
        }
        return;
    }
    if (rejected_count >= MAX_KEPT_REJECTIONS) return;
    rejected_promises[rejected_count] = JS_DupValue(ctx, promise);
    rejected_reasons[rejected_count] = JS_DupValue(ctx, reason);
    rejected_contexts[rejected_count] = ctx;
    rejected_count++;
}

/* A limit this module was compiled with, for the host to compare with its
 * own; -1 for a question it does not know. */
NT_EXPORT("nt_limit")
int32_t nt_limit(int32_t which) {
    switch (which) {
    case LIMIT_INTS:
        return MAX_INTS;
    case LIMIT_OUT:
        return OUT_CAPACITY;
    case LIMIT_ERROR:
        return ERROR_CAPACITY;
    case LIMIT_COPY_CHECK:
        return nt_copy_check_bytes();
    case LIMIT_CONTEXTS:
        return CONTEXTS;
    default:
        return -1;
    }
}

/* Starts QuickJS's one runtime with `memory_limit` bytes for its heap and
 * `stack_limit` bytes of stack; local time is `zone` minutes east of UTC.
 * 0 when it started. */
NT_EXPORT("nt_init")
int32_t nt_init(int32_t memory_limit, int32_t stack_limit, int32_t zone) {
    if (runtime) return -3;
    heap_limit = memory_limit > 0 ? (size_t)memory_limit : 0;
    runtime = JS_NewRuntime2(&counted_allocator, NULL);
    if (!runtime) return -1;
    JS_SetMaxStackSize(runtime, (size_t)stack_limit);
    JS_SetHostPromiseRejectionTracker(runtime, rejection_tracker, NULL);
    zone_minutes = zone;
    for (int index = 0; index < CONTEXTS; index++) {
        contexts[index] = NULL;
        dispatch_functions[index] = JS_UNDEFINED;
        rejection_functions[index] = JS_UNDEFINED;
        stack_getters[index] = JS_UNDEFINED;
    }
    return 0;
}

/* Makes context `number` (1 the channel's, 2 the page's), its Math.random
 * and hash seeded with `seed` (a positive number below 2 to the 62), and
 * gives its realm the two host functions. 0 when made. */
NT_EXPORT("nt_context_open")
int32_t nt_context_open(int32_t number, int64_t seed) {
    if (!runtime) return -1;
    if (number < CONTEXT_CHANNEL || number > CONTEXT_PAGE) return -2;
    if (contexts[number - 1]) return -3;
    pending_seed = seed;
    seed_waiting = true;
    /* The standard objects, without WeakRef and FinalizationRegistry: they
     * make the collector's timing visible to a page, and a finalization
     * callback is page code that would run after its context's last
     * callback. No modules, no `os` or `std`. */
    JSContext *ctx = JS_NewContextRaw(runtime);
    seed_waiting = false;
    if (!ctx) return -4;
    if (JS_AddIntrinsicBaseObjects(ctx) || JS_AddIntrinsicDate(ctx) || JS_AddIntrinsicEval(ctx) || JS_AddIntrinsicRegExp(ctx) || JS_AddIntrinsicJSON(ctx) ||
        JS_AddIntrinsicProxy(ctx) || JS_AddIntrinsicMapSet(ctx) || JS_AddIntrinsicTypedArrays(ctx) || JS_AddIntrinsicPromise(ctx) ||
        JS_AddIntrinsicBigInt(ctx) || JS_AddIntrinsicAToB(ctx)) {
        JS_FreeContext(ctx);
        return -5;
    }
    JS_SetContextOpaque(ctx, (void *)(intptr_t)number);
    contexts[number - 1] = ctx;
    dispatch_functions[number - 1] = JS_UNDEFINED;
    rejection_functions[number - 1] = JS_UNDEFINED;
    JSValue global = JS_GetGlobalObject(ctx);
    /* Error.prototype's `stack` getter, while the realm is still as
     * QuickJS made it. */
    stack_getters[number - 1] = JS_UNDEFINED;
    JSValue error_constructor = JS_GetPropertyStr(ctx, global, "Error");
    JSValue error_prototype = JS_GetPropertyStr(ctx, error_constructor, "prototype");
    JSAtom stack_atom = JS_NewAtom(ctx, "stack");
    JSPropertyDescriptor stack_descriptor;
    if (JS_IsObject(error_prototype) && stack_atom != JS_ATOM_NULL && JS_GetOwnProperty(ctx, &stack_descriptor, error_prototype, stack_atom) > 0) {
        if ((stack_descriptor.flags & JS_PROP_GETSET) && JS_IsFunction(ctx, stack_descriptor.getter)) stack_getters[number - 1] = JS_DupValue(ctx, stack_descriptor.getter);
        JS_FreeValue(ctx, stack_descriptor.value);
        JS_FreeValue(ctx, stack_descriptor.getter);
        JS_FreeValue(ctx, stack_descriptor.setter);
    }
    JS_FreeAtom(ctx, stack_atom);
    JS_FreeValue(ctx, error_prototype);
    JS_FreeValue(ctx, error_constructor);
    JS_SetPropertyStr(ctx, global, "__host", JS_NewCFunction(ctx, js_host, "__host", 1));
    JS_SetPropertyStr(ctx, global, "__setDispatch", JS_NewCFunction(ctx, js_set_dispatch, "__setDispatch", 1));
    JS_SetPropertyStr(ctx, global, "__setRejection", JS_NewCFunction(ctx, js_set_rejection, "__setRejection", 1));
    JS_FreeValue(ctx, global);
    return 0;
}

/* Ends context `number`: its realm is freed and the collector run, so
 * that nothing of it is left. 0 when ended. */
NT_EXPORT("nt_context_close")
int32_t nt_context_close(int32_t number) {
    JSContext *ctx = context_of(number);
    if (!ctx) return -1;
    for (int32_t index = rejected_count - 1; index >= 0; index--) {
        if (rejected_contexts[index] == ctx) drop_rejection(index);
    }
    JS_FreeValue(ctx, dispatch_functions[number - 1]);
    dispatch_functions[number - 1] = JS_UNDEFINED;
    JS_FreeValue(ctx, rejection_functions[number - 1]);
    rejection_functions[number - 1] = JS_UNDEFINED;
    JS_FreeValue(ctx, stack_getters[number - 1]);
    stack_getters[number - 1] = JS_UNDEFINED;
    contexts[number - 1] = NULL;
    JS_FreeContext(ctx);
    JS_RunGC(runtime);
    return 0;
}

NT_EXPORT("nt_alloc")
void *nt_alloc(int32_t size) { return malloc(size > 0 ? (size_t)size : 1); }

NT_EXPORT("nt_free")
void nt_free(void *pointer) { free(pointer); }

/* Runs every promise job waiting, of either context, to exhaustion (the
 * time limit is the host's). A promise rejected with no handler by then
 * is the page's failure (script-api.md sections 3.8 and 4.2): its
 * context's rejection function is called, which raises
 * `unhandledrejection`, and the jobs that made are run in turn. The bits
 * to add to the entry's result; the first failure's text is kept unless
 * `threw` says one is kept already or the rejection's event was
 * cancelled, which stops the log line and not the count. */
static int32_t run_jobs(bool threw) {
    int32_t bits = 0;
    for (;;) {
        for (;;) {
            JSContext *job_context = NULL;
            int ran = JS_ExecutePendingJob(runtime, &job_context);
            if (ran == 0) break;
            if (ran < 0) {
                if (!threw && job_context) {
                    keep_exception(job_context);
                    threw = true;
                } else if (job_context) {
                    JS_FreeValue(job_context, JS_GetException(job_context));
                }
                bits |= ENTRY_THREW;
            }
        }
        if (rejected_count == 0) break;
        JSContext *ctx = rejected_contexts[0];
        JSValue promise = JS_DupValue(ctx, rejected_promises[0]);
        JSValue reason = JS_DupValue(ctx, rejected_reasons[0]);
        drop_rejection(0);
        bits |= ENTRY_REJECTED;
        bool quiet = false;
        int32_t number = (int32_t)(intptr_t)JS_GetContextOpaque(ctx);
        if (context_of(number) == ctx && JS_IsFunction(ctx, rejection_functions[number - 1])) {
            JSValueConst arguments[2] = { promise, reason };
            JSValue cancelled = JS_Call(ctx, rejection_functions[number - 1], JS_UNDEFINED, 2, arguments);
            if (JS_IsException(cancelled)) JS_FreeValue(ctx, JS_GetException(ctx));
            else quiet = JS_VALUE_GET_TAG(cancelled) == JS_TAG_BOOL && JS_VALUE_GET_BOOL(cancelled);
            JS_FreeValue(ctx, cancelled);
        }
        if (!quiet && !threw) {
            keep_thrown(ctx, reason);
            threw = true;
        }
        JS_FreeValue(ctx, promise);
        JS_FreeValue(ctx, reason);
    }
    if (memory_refused) {
        memory_refused = false;
        bits |= ENTRY_MEMORY;
    }
    return bits;
}

/* Runs `source` (NUL at `source[length]`) in context `number` as a classic
 * script named `name`, in strict mode when `strict` is not 0, then the
 * promise jobs it made. The entry's bits, or a negative number when there
 * is no such context. */
NT_EXPORT("nt_eval")
int32_t nt_eval(int32_t number, const char *source, int32_t length, const char *name, int32_t strict) {
    JSContext *ctx = context_of(number);
    if (!ctx || length < 0) return -1;
    JS_UpdateStackTop(runtime);
    int32_t bits = 0;
    JSValue value = JS_Eval(ctx, source, (size_t)length, name, JS_EVAL_TYPE_GLOBAL | (strict ? JS_EVAL_FLAG_STRICT : 0));
    if (JS_IsException(value)) {
        keep_exception(ctx);
        bits |= ENTRY_THREW;
    }
    JS_FreeValue(ctx, value);
    return bits | run_jobs(bits != 0);
}

/* The prelude compiled once and run in every context (rdm TODO-7386):
 * compiling the browser's own prelude was most of what a context's start
 * cost, and it is the same work every time. Only the host's own source is
 * ever compiled here and only what this module wrote is ever read back:
 * QuickJS does not check bytecode it reads, so the host gives nt_run
 * nothing that came from a channel, and no script can reach either
 * function. */
static uint8_t *compiled_bytes;
static size_t compiled_length;

/* Compiles `source` (NUL at `source[length]`) in context `number` as a
 * classic script named `name`, without running it, and keeps its
 * bytecode for the host to read at nt_compiled(). The bytecode's length;
 * 0 when the source does not compile (nt_error says why); a negative
 * number when there is no such context. */
NT_EXPORT("nt_compile")
int32_t nt_compile(int32_t number, const char *source, int32_t length, const char *name) {
    JSContext *ctx = context_of(number);
    if (!ctx || length < 0) return -1;
    JS_UpdateStackTop(runtime);
    if (compiled_bytes) js_free(ctx, compiled_bytes);
    compiled_bytes = NULL;
    compiled_length = 0;
    JSValue function = JS_Eval(ctx, source, (size_t)length, name, JS_EVAL_TYPE_GLOBAL | JS_EVAL_FLAG_COMPILE_ONLY);
    if (JS_IsException(function)) {
        keep_exception(ctx);
        return 0;
    }
    compiled_bytes = JS_WriteObject(ctx, &compiled_length, function, JS_WRITE_OBJ_BYTECODE);
    JS_FreeValue(ctx, function);
    if (!compiled_bytes || compiled_length > INT32_MAX) {
        keep_exception(ctx);
        return 0;
    }
    return (int32_t)compiled_length;
}

/* Where the bytecode nt_compile last made is. */
NT_EXPORT("nt_compiled")
int32_t nt_compiled(void) { return (int32_t)(intptr_t)compiled_bytes; }

/* Runs in context `number` the bytecode nt_compile made of a script, then
 * the promise jobs it made. The entry's bits, or a negative number when
 * there is no such context. */
NT_EXPORT("nt_run")
int32_t nt_run(int32_t number, const uint8_t *bytecode, int32_t length) {
    JSContext *ctx = context_of(number);
    if (!ctx || length < 0) return -1;
    JS_UpdateStackTop(runtime);
    int32_t bits = 0;
    JSValue function = JS_ReadObject(ctx, bytecode, (size_t)length, JS_READ_OBJ_BYTECODE);
    /* JS_EvalFunction frees the function, and passes an exception on. */
    JSValue value = JS_IsException(function) ? function : JS_EvalFunction(ctx, function);
    if (JS_IsException(value)) {
        keep_exception(ctx);
        bits |= ENTRY_THREW;
    }
    JS_FreeValue(ctx, value);
    return bits | run_jobs(bits != 0);
}

/* One event for context `number`: `dispatch(kind, a, b, c, d, payload)`,
 * the payload a string, a Uint8Array, an Int32Array or undefined by
 * `payload_kind`; then the promise jobs it made. The dispatch function's
 * whole-number result in the low bits (bit 0: the default was cancelled)
 * with the entry's bits above, or a negative number when there is no such
 * context. */
NT_EXPORT("nt_event")
int32_t nt_event(int32_t number, int32_t kind, int32_t a, int32_t b, int32_t c, int32_t d, const uint8_t *payload, int32_t payload_length, int32_t payload_kind) {
    JSContext *ctx = context_of(number);
    if (!ctx || payload_length < 0) return -1;
    JS_UpdateStackTop(runtime);
    if (!JS_IsFunction(ctx, dispatch_functions[number - 1])) return run_jobs(false);
    JSValue arguments[6];
    arguments[0] = JS_NewInt32(ctx, kind);
    arguments[1] = JS_NewInt32(ctx, a);
    arguments[2] = JS_NewInt32(ctx, b);
    arguments[3] = JS_NewInt32(ctx, c);
    arguments[4] = JS_NewInt32(ctx, d);
    switch (payload_kind) {
    case PAYLOAD_STRING:
        arguments[5] = JS_NewStringLen(ctx, (const char *)payload, (size_t)payload_length);
        break;
    case PAYLOAD_BYTES:
        arguments[5] = JS_NewUint8ArrayCopy(ctx, payload, (size_t)payload_length);
        break;
    case PAYLOAD_INTS: {
        /* An Int32Array over a copy of the words. */
        JSValue copy = JS_NewArrayBufferCopy(ctx, payload, (size_t)payload_length & ~(size_t)3);
        if (JS_IsException(copy)) {
            arguments[5] = copy;
        } else {
            JSValueConst over[1] = { copy };
            arguments[5] = JS_NewTypedArray(ctx, 1, over, JS_TYPED_ARRAY_INT32);
            JS_FreeValue(ctx, copy);
        }
        break;
    }
    default:
        arguments[5] = JS_UNDEFINED;
        break;
    }
    int32_t bits = 0;
    if (JS_IsException(arguments[5])) {
        keep_exception(ctx);
        return ENTRY_THREW | run_jobs(true);
    }
    JSValue value = JS_Call(ctx, dispatch_functions[number - 1], JS_UNDEFINED, 6, arguments);
    JS_FreeValue(ctx, arguments[5]);
    if (JS_IsException(value)) {
        keep_exception(ctx);
        bits |= ENTRY_THREW;
    } else if (JS_VALUE_GET_TAG(value) == JS_TAG_INT) {
        bits |= JS_VALUE_GET_INT(value) & ENTRY_DISPATCH_BITS;
    }
    JS_FreeValue(ctx, value);
    return bits | run_jobs((bits & ENTRY_THREW) != 0);
}

/* The last kept error's text, copied to `out`; its length. */
NT_EXPORT("nt_error")
int32_t nt_error(char *out, int32_t capacity) {
    int32_t length = error_length < capacity ? error_length : capacity;
    if (length < 0) length = 0;
    memcpy(out, error_text, (size_t)length);
    error_length = 0;
    return length;
}

/* Where the count of QuickJS's heap is kept, four bytes, for the host to
 * read from the module's memory whenever it wants the figure. */
NT_EXPORT("nt_heap_address")
int32_t nt_heap_address(void) { return (int32_t)(intptr_t)&heap_bytes; }
