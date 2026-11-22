// The script-side half of the API `nt`'s shared core (script-api.md
// sections 3, 4 and 10; work package F2). It is the body of one function
// that the host runs first in each context's realm, with the tables OP,
// EV, INFO, PROP, POINTER_ACTION, KEY_BIT, DISPATCH, CAUSE, HANDLE, FAMILY
// and BOUND written in front of it by the host (ops.rs), so nothing here
// is a global unless it is put on globalThis by name.
//
// It is not trusted: it runs in the script's own realm, and a script can
// change the objects it uses. It only turns the API's objects into flat
// host calls, whole numbers and at most one string or typed array each.
// Every check that matters is the host's, in Rust.

const host = globalThis.__host;
const setDispatch = globalThis.__setDispatch;
const setRejection = globalThis.__setRejection;
delete globalThis.__host;
delete globalThis.__setDispatch;
delete globalThis.__setRejection;
// No shared memory and no threads (departures D13 and D14; DEC-7282 item
// 11): the build has no Atomics, and SharedArrayBuffer is taken away.
delete globalThis.SharedArrayBuffer;
delete globalThis.Atomics;

const TICKS_PER_SECOND = BOUND.TICKS_PER_SECOND;
const LEVEL = host(OP.INFO, INFO.LEVEL);
const isPage = LEVEL === 2;

function domException(message, name) {
  return new DOMException(message, name);
}

// Events (DOM) ----------------------------------------------------------------

// The event types only the host raises: a script cannot dispatch one.
const HOST_TYPES = new Set([
  "keydown", "keyup", "text", "pointerdown", "pointerup", "pointermove", "pointerenter", "pointerleave", "pointercancel",
  "wheel", "focus", "blur", "visibilitychange", "tick", "navigate", "pageshow", "pagehide", "close", "sharedchange",
  "painted", "cycleend", "cue", "ended", "allowed", "fault", "error", "unhandledrejection",
]);
// The host's kind of event behind each type a thing can be listened to
// for, so that the host is told which things have listeners (LISTEN).
const KIND_OF_TYPE = {
  keydown: EV.KEY_DOWN, keyup: EV.KEY_UP, text: EV.TEXT,
  pointerdown: EV.POINTER, pointerup: EV.POINTER, pointermove: EV.POINTER, pointerenter: EV.POINTER, pointerleave: EV.POINTER, pointercancel: EV.POINTER,
  wheel: EV.WHEEL, focus: EV.FOCUS, blur: EV.BLUR, visibilitychange: EV.VISIBILITY, tick: EV.TICK, navigate: EV.NAVIGATE,
  pageshow: EV.PAGE_SHOW, pagehide: EV.PAGE_HIDE, close: EV.CLOSE, sharedchange: EV.SHARED, painted: EV.PAINTED, cycleend: EV.CYCLE_END,
  cue: EV.SOUND_CUE, ended: EV.SOUND_ENDED, allowed: EV.SOUND_ALLOWED, fault: EV.FAULT,
};

const trustedEvents = new WeakSet();
const listenersOf = new WeakMap(); // target -> Map(type -> array of records)
const handleOf = new WeakMap(); // target -> the host's handle for it
const parentOf = new WeakMap(); // thing -> the thing its events go to next
const things = new Map(); // handle -> thing
const listenCounts = new Map(); // "handle:kind" -> listeners
let currentScope = null;
// What the dispatch running now has seen: a listener ran; one threw.
let dispatching = { handled: false, threw: false };

class Event {
  #type;
  #cancelable;
  #bubbles;
  #canceled = false;
  #target = null;
  #currentTarget = null;
  #stop = false;
  #stopNow = false;
  #phase = 0;
  #untargeted = false;
  #time;
  constructor(type, init) {
    if (arguments.length < 1) throw new TypeError("an event has a type");
    this.#type = String(type);
    this.#cancelable = !!(init && init.cancelable);
    this.#bubbles = !!(init && init.bubbles);
    this.#time = performanceNow();
  }
  get type() { return this.#type; }
  get target() { return this.#untargeted ? null : this.#target; }
  get currentTarget() { return this.#currentTarget; }
  get srcElement() { return this.#untargeted ? null : this.#target; }
  get cancelable() { return this.#cancelable; }
  get bubbles() { return this.#bubbles; }
  get composed() { return false; }
  get defaultPrevented() { return this.#canceled; }
  get returnValue() { return !this.#canceled; }
  get isTrusted() { return trustedEvents.has(this); }
  get eventPhase() { return this.#phase; }
  get timeStamp() { return this.#time; }
  get cancelBubble() { return this.#stop; }
  set cancelBubble(value) { if (value) this.#stop = true; }
  preventDefault() { if (this.#cancelable) this.#canceled = true; }
  stopPropagation() { this.#stop = true; }
  stopImmediatePropagation() { this.#stop = true; this.#stopNow = true; }
  composedPath() { return this.#currentTarget ? [this.#currentTarget] : []; }
  // The dispatcher's own way in; a script cannot name these.
  static #begin(event, target) { event.#target = target; event.#stop = false; event.#stopNow = false; }
  static #at(event, node, phase) { event.#currentTarget = node; event.#phase = phase; }
  static #stopped(event, now) { return now ? event.#stopNow : event.#stop; }
  static #isDispatching(event) { return event.#phase !== 0; }
  static #untarget(event) { event.#untargeted = true; }
  static {
    eventInternals = { begin: Event.#begin, at: Event.#at, stopped: Event.#stopped, isDispatching: Event.#isDispatching, untarget: Event.#untarget };
  }
}
var eventInternals;
Event.NONE = 0;
Event.CAPTURING_PHASE = 1;
Event.AT_TARGET = 2;
Event.BUBBLING_PHASE = 3;

// An event class whose instances carry the fields `names` from their
// init dictionary, read-only.
function eventClass(name, names, Base) {
  const Class = class extends (Base || Event) {
    constructor(type, init) {
      super(type, init);
      for (const field of names) {
        Object.defineProperty(this, field, { value: init && field in init ? init[field] : FIELD_DEFAULTS[field], enumerable: true });
      }
    }
  };
  Object.defineProperty(Class, "name", { value: name });
  return Class;
}
const FIELD_DEFAULTS = {
  key: "", repeat: false, shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, lost: false,
  data: "", detail: null, pointerId: 0, pointerType: "mouse", isPrimary: true, button: 0, buttons: 0, x: 0, y: 0, localX: 0, localY: 0,
  things: Object.freeze([]), deltaX: 0, deltaY: 0, tick: 0, missed: 0, destination: null, cause: "script", message: "", error: undefined,
  promise: undefined, reason: undefined,
};
const CustomEvent = eventClass("CustomEvent", ["detail"]);
// No `code`, `location` or `keyCode` (departure D18).
const KeyboardEvent = eventClass("KeyboardEvent", ["key", "repeat", "shiftKey", "ctrlKey", "altKey", "metaKey", "lost"]);
// No pressure, size, tilt, screen or client coordinates (departure D19).
const PointerEvent = eventClass("PointerEvent", ["pointerId", "pointerType", "isPrimary", "button", "buttons", "x", "y", "localX", "localY", "things"]);
const WheelEvent = eventClass("WheelEvent", ["deltaX", "deltaY", "x", "y"]);
const NtTextEvent = eventClass("NtTextEvent", ["data"]);
const NtTickEvent = eventClass("NtTickEvent", ["tick", "missed"]);
const NtNavigateEvent = eventClass("NtNavigateEvent", ["destination", "cause"]);
const ErrorEvent = eventClass("ErrorEvent", ["message", "error"]);
const PromiseRejectionEvent = eventClass("PromiseRejectionEvent", ["promise", "reason"]);

// Tells the host that a thing has gained or lost its last listener for a
// kind of event.
function countListener(target, type, by) {
  const handle = handleOf.get(target);
  const kind = KIND_OF_TYPE[type];
  if (handle === undefined || kind === undefined) return;
  const key = handle + ":" + kind;
  const before = listenCounts.get(key) || 0;
  const after = before + by;
  if (after <= 0) listenCounts.delete(key);
  else listenCounts.set(key, after);
  if (before === 0 && after > 0) host(OP.LISTEN, handle, kind, 1);
  if (before > 0 && after <= 0) host(OP.LISTEN, handle, kind, 0);
}

function removeRecord(target, type, record) {
  if (record.removed) return;
  record.removed = true;
  const byType = listenersOf.get(target);
  const list = byType && byType.get(type);
  if (!list) return;
  const index = list.indexOf(record);
  if (index >= 0) list.splice(index, 1);
  countListener(target, type, -1);
}

class EventTarget {
  addEventListener(type, callback, options) {
    if (callback === null || callback === undefined) return;
    if (typeof callback !== "function" && typeof callback !== "object") throw new TypeError("a listener is a function or an object with handleEvent");
    type = String(type);
    const once = !!(options && typeof options === "object" && options.once);
    const signal = options && typeof options === "object" ? options.signal : undefined;
    if (signal !== undefined && signal !== null) {
      if (!(signal instanceof AbortSignal)) throw new TypeError("signal is an AbortSignal");
      if (signal.aborted) return;
    }
    let byType = listenersOf.get(this);
    if (!byType) listenersOf.set(this, (byType = new Map()));
    let list = byType.get(type);
    if (!list) byType.set(type, (list = []));
    if (list.some((record) => record.callback === callback)) return;
    const record = { callback, once, removed: false };
    // The host is told first: if it refuses (a bound), nothing was added.
    countListener(this, type, 1);
    list.push(record);
    if (signal) onAbort(signal, () => removeRecord(this, type, record));
    if (currentScope) currentScope.listeners.push([this, type, record]);
  }
  removeEventListener(type, callback) {
    type = String(type);
    const byType = listenersOf.get(this);
    const list = byType && byType.get(type);
    if (!list) return;
    const record = list.find((one) => one.callback === callback);
    if (record) removeRecord(this, type, record);
  }
  dispatchEvent(event) {
    if (!(event instanceof Event)) throw new TypeError("dispatchEvent takes an Event");
    // A script cannot forge the host's events (section 3.3).
    if (HOST_TYPES.has(event.type)) throw new TypeError("'" + event.type + "' is an event only the host raises");
    if (eventInternals.isDispatching(event)) throw domException("the event is being dispatched", "InvalidStateError");
    return fire(this, event, event.bubbles);
  }
}

// Calls the listeners of `event` on `target`, then outwards by the
// target's parents when it bubbles. There is no capture phase. A listener
// that throws is reported and the rest still run, as DOM says.
function fire(target, event, bubbles) {
  eventInternals.begin(event, target);
  let node = target;
  let first = true;
  while (node) {
    const byType = listenersOf.get(node);
    const list = byType && byType.get(event.type);
    if (list && list.length) {
      eventInternals.at(event, node, first ? Event.AT_TARGET : Event.BUBBLING_PHASE);
      for (const record of list.slice()) {
        if (record.removed) continue;
        if (record.once) removeRecord(node, event.type, record);
        dispatching.handled = true;
        try {
          if (typeof record.callback === "function") record.callback.call(node, event);
          else record.callback.handleEvent(event);
        } catch (error) {
          report(error);
        }
        if (eventInternals.stopped(event, true)) break;
      }
    }
    if (!bubbles || eventInternals.stopped(event, false)) break;
    node = parentOf.get(node);
    first = false;
  }
  eventInternals.at(event, null, Event.NONE);
  return !event.defaultPrevented;
}

function trusted(event) {
  trustedEvents.add(event);
  return event;
}

// Errors (sections 3.8 and 4.2; departure D26) ---------------------------------

const globalTarget = new EventTarget();
let reporting = false;

function describe(value) {
  try {
    if (value instanceof Error) {
      const stack = typeof value.stack === "string" ? "\n" + value.stack : "";
      return value.name + ": " + value.message + stack;
    }
    if (typeof value === "string") return value;
    const json = JSON.stringify(value);
    return json === undefined ? String(value) : json;
  } catch (error) {
    try { return String(value); } catch (again) { return "a value that cannot be described"; }
  }
}

function logLine(level, text) {
  host(OP.LOG, level, text.length > BOUND.MAX_LOG_CHARS ? text.slice(0, BOUND.MAX_LOG_CHARS) : text);
}

// An exception nobody caught: an `error` event on globalThis, then the
// log unless a listener cancelled it. It counts as a failure either way.
function report(error) {
  dispatching.threw = true;
  if (reporting) {
    logLine(4, "uncaught while reporting an error: " + describe(error));
    return;
  }
  reporting = true;
  let quiet = false;
  try {
    const event = trusted(new ErrorEvent("error", { cancelable: true, message: describe(error), error }));
    quiet = !fire(globalTarget, event, false);
  } finally {
    reporting = false;
  }
  if (!quiet) logLine(4, "uncaught: " + describe(error));
}

setRejection(function (promise, reason) {
  const event = trusted(new PromiseRejectionEvent("unhandledrejection", { cancelable: true, promise, reason }));
  const before = dispatching;
  dispatching = { handled: false, threw: false };
  try {
    return !fire(globalTarget, event, false);
  } finally {
    dispatching = before;
  }
});

// AbortController and AbortSignal (DOM) ----------------------------------------

const signalState = new WeakMap(); // signal -> { aborted, reason, steps }
const MAKE = Symbol("make");

class AbortSignal extends EventTarget {
  constructor(token) {
    if (token !== MAKE) throw new TypeError("an AbortSignal is made by an AbortController");
    super();
    signalState.set(this, { aborted: false, reason: undefined, steps: [] });
  }
  get aborted() { return signalState.get(this).aborted; }
  get reason() { return signalState.get(this).reason; }
  throwIfAborted() { const state = signalState.get(this); if (state.aborted) throw state.reason; }
  static abort(reason) {
    const signal = new AbortSignal(MAKE);
    abortSignal(signal, reason);
    return signal;
  }
  // Rounded up to ticks (departure D16).
  static timeout(milliseconds) {
    const signal = new AbortSignal(MAKE);
    setTimer(() => abortSignal(signal, domException("the operation timed out", "TimeoutError")), milliseconds, [], false);
    return signal;
  }
  static any(signals) {
    const result = new AbortSignal(MAKE);
    for (const signal of signals) {
      if (!(signal instanceof AbortSignal)) throw new TypeError("any takes AbortSignals");
      if (signal.aborted) { abortSignal(result, signal.reason); return result; }
    }
    for (const signal of signals) onAbort(signal, () => abortSignal(result, signal.reason));
    return result;
  }
}

function onAbort(signal, step) {
  signalState.get(signal).steps.push(step);
}

function abortSignal(signal, reason) {
  const state = signalState.get(signal);
  if (state.aborted) return;
  state.aborted = true;
  state.reason = reason === undefined ? domException("the signal was aborted", "AbortError") : reason;
  const steps = state.steps;
  state.steps = [];
  for (const step of steps) step();
  fire(signal, trusted(new Event("abort")), false);
}

class AbortController {
  #signal = new AbortSignal(MAKE);
  get signal() { return this.#signal; }
  abort(reason) { abortSignal(this.#signal, reason); }
}

// Time and chance (section 3.3; departures D12 to D16) -------------------------

function performanceNow() {
  return (host(OP.NOW) * 1000) / TICKS_PER_SECOND;
}

const timers = new Map(); // the host's number -> { callback, args, repeat }

function setTimer(callback, delay, args, repeat) {
  // A string is never compiled: there is no timer that evaluates text.
  if (typeof callback !== "function") throw new TypeError("a timer's handler is a function");
  let milliseconds = Number(delay);
  if (!(milliseconds > 0)) milliseconds = 0;
  // Rounded up to whole ticks; the least is one tick (departure D12).
  let ticks = Math.ceil((milliseconds * TICKS_PER_SECOND) / 1000);
  if (ticks < 1) ticks = 1;
  // Past the bound the host refuses it, naming the bound.
  if (ticks > BOUND.MAX_TIMER_TICKS) ticks = BOUND.MAX_TIMER_TICKS + 1;
  const id = host(OP.TIMER, ticks, repeat ? 1 : 0);
  timers.set(id, { callback, args, repeat });
  if (currentScope) currentScope.timers.push(id);
  return id;
}

function clearTimer(id) {
  if (typeof id === "number" && timers.delete(id)) host(OP.CANCEL, id);
}

function define(name, value) {
  Object.defineProperty(globalThis, name, { value, writable: true, configurable: true, enumerable: false });
}

define("setTimeout", function setTimeout(callback, delay, ...args) { return setTimer(callback, delay, args, false); });
define("setInterval", function setInterval(callback, delay, ...args) { return setTimer(callback, delay, args, true); });
define("clearTimeout", function clearTimeout(id) { clearTimer(id); });
define("clearInterval", function clearInterval(id) { clearTimer(id); });
define("performance", Object.freeze({
  // It advances once a tick (departure D13).
  now() { return performanceNow(); },
  timeOrigin: Number(host(OP.INFO, INFO.ORIGIN)),
}));

function inspect(value) {
  return typeof value === "string" ? value : describe(value);
}
function logger(level) {
  return function (...values) { logLine(level, values.map(inspect).join(" ")); };
}
define("console", Object.freeze({ debug: logger(0), log: logger(1), info: logger(2), warn: logger(3), error: logger(4) }));

// JSON-representable values, Date, Map, Set, ArrayBuffer and typed arrays;
// no transfer list (departure D17).
function structuredCloneOf(value, seen) {
  if (value === null || (typeof value !== "object" && typeof value !== "function")) {
    if (typeof value === "symbol") throw domException("a symbol cannot be cloned", "DataCloneError");
    return value;
  }
  if (typeof value === "function") throw domException("a function cannot be cloned", "DataCloneError");
  if (seen.has(value)) return seen.get(value);
  let copy;
  if (value instanceof Date) {
    copy = new Date(value.getTime());
    seen.set(value, copy);
  } else if (value instanceof ArrayBuffer) {
    copy = value.slice(0);
    seen.set(value, copy);
  } else if (ArrayBuffer.isView(value)) {
    const buffer = structuredCloneOf(value.buffer, seen);
    copy = value instanceof DataView ? new DataView(buffer, value.byteOffset, value.byteLength) : new value.constructor(buffer, value.byteOffset, value.length);
    seen.set(value, copy);
  } else if (value instanceof Map) {
    copy = new Map();
    seen.set(value, copy);
    for (const [key, item] of value) copy.set(structuredCloneOf(key, seen), structuredCloneOf(item, seen));
  } else if (value instanceof Set) {
    copy = new Set();
    seen.set(value, copy);
    for (const item of value) copy.add(structuredCloneOf(item, seen));
  } else if (Array.isArray(value)) {
    copy = new Array(value.length);
    seen.set(value, copy);
    for (const key of Object.keys(value)) copy[key] = structuredCloneOf(value[key], seen);
  } else {
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) throw domException("the object cannot be cloned", "DataCloneError");
    copy = {};
    seen.set(value, copy);
    for (const key of Object.keys(value)) copy[key] = structuredCloneOf(value[key], seen);
  }
  return copy;
}
define("structuredClone", function structuredClone(value, options) {
  if (options && options.transfer && options.transfer.length) throw domException("nothing can be transferred", "DataCloneError");
  return structuredCloneOf(value, new Map());
});

const INTEGER_ARRAYS = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, BigInt64Array, BigUint64Array];
const HEX = Array.from({ length: 256 }, (_, byte) => (byte + 256).toString(16).slice(1));
// The Web Cryptography standard's two, from the context's cryptographic
// generator, never from Math.random's (departure D15).
define("crypto", Object.freeze({
  getRandomValues(array) {
    if (!INTEGER_ARRAYS.some((Type) => array instanceof Type)) throw domException("getRandomValues takes an integer typed array", "TypeMismatchError");
    if (array.byteLength > BOUND.MAX_RANDOM_BYTES) throw domException("getRandomValues gives at most 65536 bytes a call", "QuotaExceededError");
    new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(host(OP.RANDOM, array.byteLength));
    return array;
  },
  randomUUID() {
    const bytes = host(OP.RANDOM, 16);
    // Version 4, variant 1 (RFC 9562 section 5.4).
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = "";
    for (let index = 0; index < 16; index++) {
      if (index === 4 || index === 6 || index === 8 || index === 10) text += "-";
      text += HEX[bytes[index]];
    }
    return text;
  },
}));

// Scopes (section 3.3) ----------------------------------------------------------

class Scope {
  #record;
  constructor(record) { this.#record = record; }
  end() {
    const record = this.#record;
    if (record.ended) return;
    record.ended = true;
    for (const [target, type, listener] of record.listeners) removeRecord(target, type, listener);
    for (const id of record.timers) timers.delete(id);
    for (const end of record.ends) end();
    // The host removes everything of the scope it holds: its timers, and
    // the things later parts of the API add.
    host(OP.SCOPE_END, record.id);
  }
}

// nt (section 3.3) ----------------------------------------------------------------

function utf8Length(text) {
  let bytes = 0;
  for (const character of text) {
    const point = character.codePointAt(0);
    bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
  }
  return bytes;
}

function whole(value, fallback, name) {
  if (value === undefined) return fallback;
  if (typeof value !== "number" || !Number.isInteger(value)) throw new TypeError(name + " is a whole number");
  return value;
}

function destinationOf(target) {
  const page = /^([1-8][0-9A-Fa-f]{2})(?:\/([0-9A-Fa-f]{4}))?$/.exec(target);
  return Object.freeze(page ? { url: null, number: page[1].toUpperCase(), subpage: page[2] ? page[2].toUpperCase() : null } : { url: target, number: null, subpage: null });
}

const CAUSE_NAMES = { [CAUSE.SCRIPT]: "script", [CAUSE.LINK]: "link", [CAUSE.KEY]: "key", [CAUSE.BROWSER]: "browser" };
// A renderer by the code the messages give it (protocol::script::Renderer).
const RENDERER_NAMES = { 1: "teletext", 2: "regis", 3: "naplps" };

function pageNow() {
  const number = host(OP.INFO, INFO.PAGE_NUMBER);
  if (number === undefined) return null;
  const subpage = host(OP.INFO, INFO.PAGE_SUBPAGE);
  const page = {
    number,
    subpage: subpage === undefined ? null : subpage,
    url: host(OP.INFO, INFO.PAGE_URL),
    renderer: host(OP.INFO, INFO.PAGE_RENDERER),
  };
  Object.defineProperty(page, "title", {
    enumerable: true,
    get() { return host(OP.INFO, INFO.PAGE_TITLE); },
    // Only the page context may; the host refuses the channel context's.
    set(value) { host(OP.TITLE, String(value)); },
  });
  return Object.freeze(page);
}
// The page context's page never changes; the channel context's is the
// page shown now, read when it is asked for.
const ownPage = isPage ? pageNow() : null;

const channel = Object.freeze({ url: host(OP.INFO, INFO.CHANNEL_URL), title: host(OP.INFO, INFO.CHANNEL_TITLE) });
const VERSION = host(OP.INFO, INFO.VERSION);

const keys = Object.freeze({
  down(name) { return host(OP.KEY_STATE, 0, String(name)) === 1; },
  pressed(name) { return host(OP.KEY_STATE, 1, String(name)) === 1; },
});

const shared = Object.freeze({
  get(key) {
    const text = host(OP.SHARED_GET, String(key));
    return text === undefined ? undefined : JSON.parse(text);
  },
  set(key, value) {
    key = String(key);
    const json = JSON.stringify(value);
    if (json === undefined) throw new TypeError("a shared value is something JSON can hold");
    host(OP.SHARED_SET, utf8Length(key), key + json);
  },
  delete(key) { host(OP.SHARED_DELETE, String(key)); },
  keys() { return JSON.parse(host(OP.SHARED_KEYS)); },
});

class NtSound extends EventTarget {
  get allowed() { return host(OP.INFO, INFO.SOUND_ALLOWED) === 1; }
  play(sound, options) {
    const o = options || {};
    host(OP.SOUND_PLAY, whole(sound, undefined, "a sound"), whole(o.voice, 0, "voice"), whole(o.gain, BOUND.MAX_GAIN, "gain"));
  }
  stop(voice) { host(OP.SOUND_STOP, whole(voice, 0, "voice")); }
}
const sound = new NtSound();
handleOf.set(sound, HANDLE.SOUND);
Object.freeze(sound);

const synth = Object.freeze({
  start(part, family, options) {
    if (!Object.prototype.hasOwnProperty.call(FAMILY, family)) throw new TypeError("family is one of " + Object.keys(FAMILY).join(", "));
    const o = options || {};
    host(OP.SYNTH_START, whole(part, undefined, "part"), FAMILY[family], whole(o.variant, 0, "variant"), whole(o.gain, BOUND.MAX_GAIN, "gain"));
  },
  note(part, note, options) {
    const o = options || {};
    host(OP.SYNTH_NOTE, whole(part, undefined, "part"), whole(note, undefined, "note"), whole(o.velocity, 100, "velocity"), whole(o.ticks, 15, "ticks"));
  },
  // MIDI channel messages 0x90, 0x80, 0xB0, 0xC0 and 0xE0 (MIDI 1.0).
  noteOn(part, note, velocity) { host(OP.SYNTH_MIDI, part, 0x90, note, whole(velocity, 100, "velocity")); },
  noteOff(part, note) { host(OP.SYNTH_MIDI, part, 0x80, note, 0); },
  control(part, controller, value) { host(OP.SYNTH_MIDI, part, 0xb0, controller, value); },
  program(part, number) { host(OP.SYNTH_MIDI, part, 0xc0, number, 0); },
  // A bend is 14 bits, sent as its low and high seven.
  bend(part, value) {
    const bend = whole(value, undefined, "a bend");
    if (bend < 0 || bend > BOUND.MAX_MIDI_BEND) throw new RangeError("MAX_MIDI_BEND: a bend is 0 to " + BOUND.MAX_MIDI_BEND);
    host(OP.SYNTH_MIDI, part, 0xe0, bend & 0x7f, bend >> 7);
  },
  stop(part) { host(OP.SYNTH_STOP, whole(part, undefined, "part")); },
});

class Nt extends EventTarget {
  get version() { return VERSION; }
  get level() { return isPage ? "page" : "channel"; }
  get hidden() { return host(OP.INFO, INFO.HIDDEN) === 1; }
  get channel() { return channel; }
  get page() { return isPage ? ownPage : pageNow(); }
  get keys() { return keys; }
  get pointer() {
    const [x, y, buttons, inside, handle] = host(OP.POINTER_STATE).split(",").map(Number);
    return { x, y, buttons, inside: inside === 1, target: things.get(handle) || null };
  }
  get shared() { return shared; }
  get sound() { return sound; }
  get synth() { return synth; }
  get teletext() { return teletextApi; }
  get regis() { return core.regis; }
  now() { return host(OP.NOW); }
  navigate(target) {
    if (typeof target !== "string") throw new TypeError("a navigation's target is a string");
    // The script's own handlers hear it first and may cancel it.
    const event = trusted(new NtNavigateEvent("navigate", { cancelable: true, destination: destinationOf(target), cause: "script" }));
    if (fire(this, event, false)) host(OP.NAVIGATE, target);
  }
  scope(fn) {
    if (typeof fn !== "function") throw new TypeError("scope takes a function");
    const record = { id: host(OP.SCOPE_BEGIN), listeners: [], timers: [], ends: [], ended: false };
    const outer = currentScope;
    currentScope = record;
    try {
      fn();
    } finally {
      currentScope = outer;
      host(OP.SCOPE_LEAVE, record.id);
    }
    return new Scope(record);
  }
  budget() { return JSON.parse(host(OP.BUDGET)); }
  log(...values) { logLine(1, values.map(inspect).join(" ")); }
}
const nt = new Nt();
handleOf.set(nt, HANDLE.NT);
Object.freeze(Nt.prototype);
Object.freeze(nt);
Object.defineProperty(globalThis, "nt", { value: nt, enumerable: true });

// globalThis hears `error` and `unhandledrejection`, as a worker's global
// does (departure D26).
define("addEventListener", function addEventListener(type, callback, options) { globalTarget.addEventListener(type, callback, options); });
define("removeEventListener", function removeEventListener(type, callback) { globalTarget.removeEventListener(type, callback); });
define("dispatchEvent", function dispatchEvent(event) { return globalTarget.dispatchEvent(event); });
define("reportError", function reportError(error) { report(error); });
for (const [name, value] of Object.entries({
  Event, EventTarget, CustomEvent, KeyboardEvent, PointerEvent, WheelEvent, NtTextEvent, NtTickEvent, NtNavigateEvent, ErrorEvent, PromiseRejectionEvent,
  AbortController, AbortSignal,
})) {
  define(name, value);
}

// The host's events (section 10.3) ---------------------------------------------

const POINTER_TYPES = {
  [POINTER_ACTION.DOWN]: "pointerdown", [POINTER_ACTION.UP]: "pointerup", [POINTER_ACTION.MOVE]: "pointermove",
  [POINTER_ACTION.ENTER]: "pointerenter", [POINTER_ACTION.LEAVE]: "pointerleave", [POINTER_ACTION.CANCEL]: "pointercancel",
};
const POINTER_KINDS = ["mouse", "touch", "pen", "mouse"];

function withFields(event, fields) {
  for (const name of Object.keys(fields)) Object.defineProperty(event, name, { value: fields[name], enumerable: true });
  return event;
}

function keyEvent(type, bits, key, cancelable) {
  const last = !!(bits & KEY_BIT.REPEAT_OR_LOST);
  return new KeyboardEvent(type, {
    cancelable, key,
    shiftKey: !!(bits & KEY_BIT.SHIFT), ctrlKey: !!(bits & KEY_BIT.CTRL), altKey: !!(bits & KEY_BIT.ALT), metaKey: !!(bits & KEY_BIT.META),
    repeat: type === "keydown" && last, lost: type === "keyup" && last,
  });
}

// One event from the host: builds the DOM event, dispatches it, and
// answers with the DISPATCH bits.
function dispatch(kind, a, b, c, d, payload) {
  let event = null;
  let target = nt;
  let bubbles = false;
  switch (kind) {
    case EV.TICK:
      event = new NtTickEvent("tick", { tick: a, missed: b });
      break;
    case EV.KEY_DOWN:
      event = keyEvent("keydown", a, payload, true);
      break;
    case EV.KEY_UP:
      event = keyEvent("keyup", a, payload, false);
      break;
    case EV.TEXT:
      event = new NtTextEvent("text", { data: payload });
      break;
    case EV.POINTER: {
      const action = a & 15;
      const type = POINTER_TYPES[action];
      if (!type) return 0;
      const thing = things.get(d) || null;
      const more = payload || [];
      const under = [];
      if (thing) under.push(thing);
      for (let index = 4; index < more.length; index++) {
        const other = things.get(more[index]);
        if (other) under.push(other);
      }
      event = new PointerEvent(type, {
        cancelable: action === POINTER_ACTION.DOWN || action === POINTER_ACTION.UP,
        pointerId: more[0] | 0, buttons: more[1] | 0, localX: more[2] | 0, localY: more[3] | 0,
        button: (a >> 4) & 15, pointerType: POINTER_KINDS[(a >> 8) & 3], isPrimary: !!(a & 1024), x: b, y: c, things: Object.freeze(under),
      });
      if (thing) target = thing;
      // Presses, releases and moves go outwards from the thing to nt;
      // enter, leave and cancel are the thing's alone.
      bubbles = action === POINTER_ACTION.DOWN || action === POINTER_ACTION.UP || action === POINTER_ACTION.MOVE;
      break;
    }
    case EV.WHEEL: {
      const thing = things.get(c) || null;
      event = new WheelEvent("wheel", { deltaX: a, deltaY: b });
      if (thing) target = thing;
      bubbles = true;
      break;
    }
    case EV.FOCUS:
      event = new Event("focus");
      break;
    case EV.BLUR:
      event = new Event("blur");
      break;
    case EV.VISIBILITY:
      event = new Event("visibilitychange");
      break;
    case EV.TIMER: {
      const timer = timers.get(a);
      if (!timer) return 0;
      if (!timer.repeat) timers.delete(a);
      dispatching.handled = true;
      try {
        timer.callback(...timer.args);
      } catch (error) {
        report(error);
      }
      return DISPATCH.HANDLED | (dispatching.threw ? DISPATCH.THREW : 0);
    }
    case EV.NAVIGATE:
      event = new NtNavigateEvent("navigate", { cancelable: a !== CAUSE.BROWSER, destination: destinationOf(payload || ""), cause: CAUSE_NAMES[a] || "browser" });
      break;
    case EV.PAGE_SHOW:
      event = withFields(new Event("pageshow"), {
        page: Object.freeze({ number: (a + 0x1000).toString(16).slice(1).toUpperCase(), subpage: b <= 0 ? null : (b + 0x10000).toString(16).slice(1).toUpperCase(), url: payload || "", renderer: RENDERER_NAMES[c] || "teletext" }),
      });
      break;
    case EV.PAGE_HIDE:
      event = withFields(new Event("pagehide"), {
        page: Object.freeze({ number: (a + 0x1000).toString(16).slice(1).toUpperCase(), subpage: b <= 0 ? null : (b + 0x10000).toString(16).slice(1).toUpperCase() }),
      });
      break;
    case EV.CLOSE:
      event = new Event("close");
      break;
    case EV.SHARED:
      event = withFields(new Event("sharedchange"), { key: payload });
      break;
    case EV.PAINTED:
    case EV.CYCLE_END:
      target = things.get(a);
      if (!target) return 0;
      event = new Event(kind === EV.PAINTED ? "painted" : "cycleend");
      break;
    case EV.SOUND_CUE:
      target = sound;
      event = withFields(new Event("cue"), { voice: a, cue: b });
      break;
    case EV.SOUND_ENDED:
      target = sound;
      event = withFields(new Event("ended"), { voice: a });
      break;
    case EV.SOUND_ALLOWED:
      target = sound;
      event = new Event("allowed");
      break;
    case EV.FAULT:
      event = withFields(new Event("fault"), { code: a, bound: payload || "" });
      break;
    default:
      // A renderer's own events (its part of the prelude).
      return core.dispatchMore.has(kind) ? core.dispatchMore.get(kind)(a, b, c, d, payload) : 0;
  }
  // The channel context is given the page's input with the same fields
  // and a target of null (section 4.5): things are the page context's.
  if (!isPage && (kind === EV.KEY_DOWN || kind === EV.KEY_UP || kind === EV.TEXT || kind === EV.POINTER || kind === EV.WHEEL)) eventInternals.untarget(event);
  fire(target, trusted(event), bubbles);
  return (event.defaultPrevented ? DISPATCH.CANCELLED : 0) | (dispatching.handled ? DISPATCH.HANDLED : 0) | (dispatching.threw ? DISPATCH.THREW : 0);
}

setDispatch(function (kind, a, b, c, d, payload) {
  dispatching = { handled: false, threw: false };
  return dispatch(kind, a, b, c, d, payload);
});

// What the renderers' and components' parts of the prelude build on: a
// thing is registered with its handle and the thing its events go to next.
const core = {
  host, things, handleOf, parentOf, nt, EventTarget, Event, fire, trusted, withFields, whole, report, listenCounts,
  dispatchMore: new Map(),
  scopeEnds(end) { if (currentScope) currentScope.ends.push(end); },
};
