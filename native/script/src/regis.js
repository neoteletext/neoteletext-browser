// The script-side half of the ReGIS API, `nt.regis` (script-api.md section
// 6; work package R1). It follows prelude.js inside the same function, with
// the tables RG_FIELD, RG_PAINT and RG_BOUND written in front by the host
// (ops.rs).
//
// A script's objects are ReGIS v2's numbered drawings themselves. A
// Drawing here holds only the host's handle for one: every attribute is
// read from the host and staged by the host, so nothing here can disagree
// with what will be committed, and a commit the host refuses needs nothing
// put back here but which drawings exist. Like the rest of the prelude it
// is not trusted: every check that matters is the host's, in Rust.

(function () {
  const { things, handleOf, parentOf, nt, EventTarget, whole, report, listenCounts } = core;
  const rawHost = core.host;

  // A callback's assignments to the five attributes a script changes
  // most (x, y, depth, visible, detectable) are not a host call each: a
  // drawing remembers what was assigned, and the drawings so changed are
  // given to the host together, one call for each attribute (operation
  // RG_SET_MANY), before any other ReGIS call and when the callback's
  // code has run, which is before the step's commit. A read of one of
  // the five is answered from what the drawing remembers. The host
  // checks every value again when they arrive (TODO-7373: a host call
  // costs several microseconds, and a page of two thousand drawings
  // moved each tick made four thousand of them).
  //
  // What a drawing remembers holds for one epoch: a refused commit and a
  // list's `set` start a new one, and the drawing asks the host again.
  let epoch = 1;
  // What the page's drawings remember (each one's own array, below), and
  // whether any waits to be given to the host.
  // Marking a drawing is one bit set in it and nothing more: which
  // drawings wait is found by looking at them all when they are given,
  // which is cheaper in this engine than keeping a list as they change.
  const alive = [];
  let waits = false;
  let flushing = false;
  let drawingInternals;
  const later = typeof queueMicrotask === "function" ? queueMicrotask : (job) => Promise.resolve().then(job);
  function wait() {
    waits = true;
    if (!flushing) { flushing = true; later(flush); }
  }
  // Gives the host what the drawings wait to, in one call: for each
  // change a handle, a property and a value, a drawing's changes
  // together, so that they are one `D(S n(…))`.
  function flush() {
    flushing = false;
    if (!waits) return;
    waits = false;
    const count = alive.length;
    const numbers = new Int32Array(15 * count);
    let at = 0;
    for (let place = 0; place < count; place++) {
      const state = alive[place];
      const waiting = state[WAITING];
      if (waiting === 0) continue;
      const handle = state[OWN_HANDLE];
      state[WAITING] = 0;
      if (waiting & X) { numbers[at] = handle; numbers[at + 1] = PROP.X; numbers[at + 2] = state[AT_X]; at += 3; }
      if (waiting & Y) { numbers[at] = handle; numbers[at + 1] = PROP.Y; numbers[at + 2] = state[AT_Y]; at += 3; }
      if (waiting & DEPTH) { numbers[at] = handle; numbers[at + 1] = PROP.DEPTH; numbers[at + 2] = state[AT_DEPTH]; at += 3; }
      if (waiting & VISIBLE) { numbers[at] = handle; numbers[at + 1] = PROP.VISIBLE; numbers[at + 2] = state[AT_VISIBLE]; at += 3; }
      if (waiting & DETECTABLE) { numbers[at] = handle; numbers[at + 1] = PROP.DETECTABLE; numbers[at + 2] = state[AT_DETECTABLE]; at += 3; }
    }
    try {
      rawHost(OP.RG_SET_MANY, 0, numbers.slice(0, at));
    } catch (error) {
      // What was not taken is not what the drawings remember.
      epoch++;
      throw error;
    }
  }
  function host(...parameters) {
    if (waits) flush();
    return rawHost(...parameters);
  }
  const LEAST = RG_BOUND.MIN_COORDINATE, MOST = RG_BOUND.MAX_COORDINATE;
  // The error for a value that is no coordinate: of the wrong kind, or
  // out of range.
  function notACoordinate(value, name) {
    const number = whole(value, undefined, name);
    throw new RangeError("MAX_COORDINATE: " + name + " is " + LEAST + " to " + MOST + ", not " + number);
  }

  // A drawing's handle for the host's ReGIS operations. The low sixteen
  // bits are its drawing number, which is what the decoder names it by
  // in events and what `things` and operation LISTEN know it by.
  const fullHandle = new WeakMap();
  // The drawings removed since the last commit, by number, each with the
  // host's epoch of its removal: a refused commit brings them back.
  const removed = new Map();
  const MAKE_DRAWING = Symbol("drawing");
  const COMMIT_ERRORS = ["", "contents", "staged", "depth", "cycle", "parent", "work", "malformed", "drawings"];
  const COMMIT_BOUNDS = [
    "", "MAX_DRAWING_CONTENT_BYTES", "MAX_STAGED_BYTES", "MAX_DRAWING_DEPTH", "REGIS_COMMIT_CYCLE", "REGIS_COMMIT_PARENT",
    "MAX_REBUILD_WORK", "REGIS_COMMIT_MALFORMED", "MAX_DRAWING_RECORD_BYTES",
  ];
  const LOOKS = [null, "hover", "pressed", "alternative"];
  const OPTIONS = new Set([
    "x", "y", "depth", "visible", "parent", "source", "detectable", "hover", "pressed", "alternative", "clip", "layer", "frames", "paintRate",
    "label", "wheel", "track",
  ]);
  let listening = false;

  function numberOf(handle) {
    return handle % RG_BOUND.HANDLE_NUMBERS;
  }

  // The handle of a Drawing given as an argument, 0 for null.
  function handleArgument(value, name) {
    if (value === null || value === undefined) return 0;
    const handle = fullHandle.get(value);
    if (handle === undefined) throw new TypeError(name + " is a Drawing or null");
    return handle;
  }

  function drawingOf(handle) {
    return handle === 0 ? null : things.get(numberOf(handle)) || null;
  }

  function truth(value, name) {
    if (typeof value !== "boolean") throw new TypeError(name + " is true or false");
    return value ? 1 : 0;
  }

  // Moves the drawings a removal named out of `things`, keeping them in
  // case the step's commit is refused. Removals of earlier epochs are
  // settled: their commits were made, or were refused and told already.
  function settleRemoved(reply) {
    const colon = reply.indexOf(":");
    const step = Number(reply.slice(0, colon));
    for (const [number, record] of removed) if (record.step !== step) removed.delete(number);
    if (colon + 1 >= reply.length) return;
    for (const text of reply.slice(colon + 1).split(",")) {
      const number = Number(text);
      const drawing = things.get(number);
      if (drawing) {
        removed.set(number, { step, drawing });
        things.delete(number);
        drawingInternals.forget(drawing);
        drawingInternals.leave(drawing);
      }
    }
  }

  // A refused commit (section 6.2, "Commit"): the drawings made in the
  // step are gone, those removed in it are back, and an `error` event
  // carries a RangeError naming the commit error and the drawing.
  core.dispatchMore.set(EV.RG_COMMIT_FAILED, function (code, number, c, d, payload) {
    const numbers = payload || [];
    const gone = numbers[0] | 0;
    const concerned = things.get(number) || null;
    for (let index = 1; index <= gone && index < numbers.length; index++) {
      const drawing = things.get(numbers[index]);
      if (drawing) { drawingInternals.leave(drawing); drawingInternals.forget(drawing); }
      things.delete(numbers[index]);
    }
    for (let index = 1 + gone; index < numbers.length; index++) {
      const record = removed.get(numbers[index]);
      if (record) { things.set(numbers[index], record.drawing); drawingInternals.join(record.drawing); }
    }
    removed.clear();
    // What the drawings remember of their attributes is of a step that
    // was discarded.
    epoch++;
    // Parents are as they were before the step.
    for (const drawing of things.values()) {
      const handle = fullHandle.get(drawing);
      if (handle !== undefined) parentOf.set(drawing, drawingOf(host(OP.RG_GET, handle, PROP.PARENT)) || nt);
    }
    const name = COMMIT_ERRORS[code] || "malformed";
    const error = new RangeError((COMMIT_BOUNDS[code] || "REGIS_COMMIT_MALFORMED") + ": the step's ReGIS commit was refused (" + name + ") and its changes were discarded");
    Object.defineProperty(error, "commit", { value: Object.freeze({ code, name, drawing: concerned }), enumerable: true });
    report(error);
    return DISPATCH.HANDLED | DISPATCH.THREW;
  });

  // The painter (section 6.3). It records the calls as whole numbers, the
  // painter program the host checks and encodes; nothing is drawn and no
  // ReGIS is written here. Each method is one call of section 6.3's
  // table, ReGIS's own.
  class RegisPainter {
    #words = [];
    #open = true;
    #filling = null;
    #push(...values) {
      if (!this.#open) throw new TypeError("the painter is used only inside draw");
      for (const value of values) this.#words.push(whole(value, undefined, "a painter's argument"));
      if (this.#words.length > RG_BOUND.MAX_PAINTER_WORDS) throw new RangeError("MAX_PAINTER_WORDS: a drawing's painter program is too long");
      return this;
    }
    #points(call, points) {
      if (!Array.isArray(points)) throw new TypeError("points is an array of [x, y]");
      const flat = [];
      for (const point of points) {
        if (!Array.isArray(point) || point.length !== 2) throw new TypeError("a point is [x, y]");
        flat.push(point[0], point[1]);
      }
      return this.#push(call, points.length, ...flat);
    }
    moveTo(x, y) { return this.#push(RG_PAINT.MOVE_TO, x, y); }
    moveBy(dx, dy) { return this.#push(RG_PAINT.MOVE_BY, dx, dy); }
    lineTo(x, y) { return this.#push(RG_PAINT.LINE_TO, x, y); }
    lineBy(dx, dy) { return this.#push(RG_PAINT.LINE_BY, dx, dy); }
    dot() { return this.#push(RG_PAINT.DOT); }
    polyline(points) { return this.#points(RG_PAINT.POLYLINE, points); }
    polygon(points) { return this.#points(RG_PAINT.POLYGON, points); }
    rect(x, y, width, height) { return this.#push(RG_PAINT.RECT, x, y, width, height); }
    ink(entry) { return this.#push(RG_PAINT.INK, entry); }
    // Pixel vectors: a string of digits 0 to 7, each a move of
    // `multiplier` pixels; 0 is east and they go anticlockwise.
    vector(digits, options) {
      if (typeof digits !== "string" || !/^[0-7]+$/.test(digits)) throw new TypeError("a pixel vector is a string of the digits 0 to 7");
      const multiplier = options && options.multiplier !== undefined ? options.multiplier : 1;
      return this.#push(RG_PAINT.VECTOR, multiplier, digits.length, ...Array.from(digits, Number));
    }
    circle(radius) { return this.#push(RG_PAINT.CIRCLE, radius); }
    circleThrough(x, y) { return this.#push(RG_PAINT.CIRCLE_THROUGH, x, y); }
    arc(cx, cy, degrees) { return this.#push(RG_PAINT.ARC, cx, cy, degrees); }
    curve(points, options) {
      const closed = options && options.closed !== undefined ? truth(options.closed, "closed") : 0;
      if (!Array.isArray(points)) throw new TypeError("points is an array of [x, y]");
      const flat = [];
      for (const point of points) {
        if (!Array.isArray(point) || point.length !== 2) throw new TypeError("a point is [x, y]");
        flat.push(point[0], point[1]);
      }
      return this.#push(RG_PAINT.CURVE, closed, points.length, ...flat);
    }
    // ReGIS's writing modes, applied to the pixels beneath in depth order.
    mode(name) {
      const mode = MODES.indexOf(name);
      if (mode < 0) throw new TypeError("a writing mode is \"overlay\", \"replace\", \"complement\" or \"erase\"");
      return this.#push(RG_PAINT.MODE, mode);
    }
    negative(on) { return this.#push(RG_PAINT.NEGATIVE, truth(on, "negative")); }
    // A standard pattern 0 to 9, or eight bits as a string of "0" and "1".
    pattern(pattern, options) {
      const multiplier = options && options.multiplier !== undefined ? options.multiplier : 1;
      if (typeof pattern === "string") {
        if (!/^[01]{8}$/.test(pattern)) throw new TypeError("a pattern is 0 to 9, or eight bits as a string of \"0\" and \"1\"");
        return this.#push(RG_PAINT.PATTERN, 1, parseInt(pattern, 2), multiplier);
      }
      return this.#push(RG_PAINT.PATTERN, 0, pattern, multiplier);
    }
    planes(mask) { return this.#push(RG_PAINT.PLANES, mask); }
    // ReGIS's shading: on to the position's own line, off, to a line at
    // a y or at an x, or with a character.
    shade(how) {
      if (how === true) return this.#push(RG_PAINT.SHADE, 1, 0);
      if (how === false) return this.#push(RG_PAINT.SHADE, 0, 0);
      if (how && typeof how === "object") {
        if (how.toY !== undefined) return this.#push(RG_PAINT.SHADE, 2, how.toY);
        if (how.toX !== undefined) return this.#push(RG_PAINT.SHADE, 3, how.toX);
        if (typeof how.char === "string" && how.char.length === 1) return this.#push(RG_PAINT.SHADE, 4, how.char.charCodeAt(0));
      }
      throw new TypeError("shade takes true, false, { toY }, { toX } or { char }");
    }
    // Text at the position, in the provided character sets. The options
    // are ReGIS's text controls and stay in force for later text of the
    // drawing, as they do in ReGIS.
    text(string, options) {
      if (typeof string !== "string") throw new TypeError("text takes a string");
      return this.#push(RG_PAINT.TEXT, ...textOptions(options), ...stringWords(textLanguage(options)), ...stringWords(string));
    }
    call(macrograph) {
      const handle = macrographHandle.get(macrograph);
      if (handle === undefined) throw new TypeError("call takes a Macrograph");
      return this.#push(RG_PAINT.CALL, handle);
    }
    fill(fn) {
      if (typeof fn !== "function") throw new TypeError("fill takes a function");
      if (this.#filling !== null) throw new TypeError("a fill is not traced inside another fill");
      this.#push(RG_PAINT.FILL, 0);
      const at = this.#words.length - 1;
      this.#filling = at;
      try {
        fn(this);
      } finally {
        this.#filling = null;
      }
      this.#words[at] = this.#words.length - at - 1;
      return this;
    }
    // The program, and the painter closed: for the functions below only.
    static #finish(painter) { painter.#open = false; return new Int32Array(painter.#words); }
    static { painterInternals = { finish: RegisPainter.#finish }; }
  }
  var painterInternals;

  const MODES = ["overlay", "replace", "complement", "erase"];
  const NOT_GIVEN = RG_BOUND.OPTION_NOT_GIVEN;
  const TEXT_OPTIONS = new Set(["size", "cell", "unit", "height", "tilt", "italic", "spacing", "language"]);
  const macrographHandle = new WeakMap();
  const MAKE_MACROGRAPH = Symbol("macrograph");

  // A string as a painter program holds it: its length, then its code points.
  function stringWords(text) {
    const points = Array.from(text, (character) => character.codePointAt(0));
    return [points.length, ...points];
  }
  function optionPair(value, name) {
    if (value === undefined || value === null) return [NOT_GIVEN, NOT_GIVEN];
    if (!Array.isArray(value) || value.length !== 2) throw new TypeError(name + " is [width, height]");
    return [whole(value[0], undefined, name), whole(value[1], undefined, name)];
  }
  function optionNumber(value, name) {
    return value === undefined || value === null ? NOT_GIVEN : whole(value, undefined, name);
  }
  // The text controls a call gives, in the program's order: size, cell,
  // unit, height, tilt, italic, spacing.
  function textOptions(options) {
    const o = options === undefined || options === null ? {} : options;
    if (typeof o !== "object") throw new TypeError("text's options are an object");
    for (const name of Object.keys(o)) if (!TEXT_OPTIONS.has(name)) throw new TypeError("'" + name + "' is not an option of text");
    return [
      optionNumber(o.size, "size"), ...optionPair(o.cell, "cell"), ...optionPair(o.unit, "unit"), optionNumber(o.height, "height"),
      optionNumber(o.tilt, "tilt"), optionNumber(o.italic, "italic"), ...optionPair(o.spacing, "spacing"),
    ];
  }
  function textLanguage(options) {
    const language = options && options.language;
    if (language === undefined || language === null) return "";
    if (typeof language !== "string") throw new TypeError("language is a language tag");
    return language;
  }

  // One of the 26 macrographs (`@:x … @;`): a painter's calls kept by the
  // receiver and drawn wherever a painter calls it. The host allocates
  // its letter.
  class Macrograph {
    constructor(token, handle) {
      if (token !== MAKE_MACROGRAPH) throw new TypeError("a Macrograph is made by nt.regis.macrograph");
      macrographHandle.set(this, handle);
      Object.freeze(this);
    }
    redefine(fn) { host(OP.RG_MACROGRAPH, macrographHandle.get(this), programOf(fn)); }
  }
  Object.freeze(Macrograph.prototype);

  // Runs `fn` with a painter and gives the program it recorded.
  function programOf(fn) {
    if (typeof fn !== "function") throw new TypeError("draw takes a function");
    const painter = new RegisPainter();
    try {
      fn(painter);
    } catch (error) {
      painterInternals.finish(painter);
      throw error;
    }
    return painterInternals.finish(painter);
  }

  function numberProperty(property, name) {
    return {
      enumerable: true,
      get() { return host(OP.RG_GET, fullHandle.get(this), property); },
      set(value) { host(OP.RG_SET, fullHandle.get(this), property, whole(value, undefined, name)); },
    };
  }
  function flagProperty(property, name) {
    return {
      enumerable: true,
      get() { return host(OP.RG_GET, fullHandle.get(this), property) === 1; },
      set(value) { host(OP.RG_SET, fullHandle.get(this), property, truth(value, name)); },
    };
  }
  function drawingProperty(property, name) {
    return {
      enumerable: true,
      get() { return drawingOf(host(OP.RG_GET, fullHandle.get(this), property)); },
      set(value) { host(OP.RG_SET, fullHandle.get(this), property, handleArgument(value, name)); },
    };
  }

  const X = 1, Y = 2, DEPTH = 4, VISIBLE = 8, DETECTABLE = 16;
  // Where a drawing keeps what it remembers, in one array (one private
  // field to reach, which is what costs in this engine): the epoch it
  // holds for, which of the five wait to be given to the host, its
  // handle, its place among the page's drawings (-1 for none), and the
  // five as last assigned or read, true and false as 1 and 0.
  const EPOCH = 0, WAITING = 1, OWN_HANDLE = 2, PLACE = 3, AT_X = 4, AT_Y = 5, AT_DEPTH = 6, AT_VISIBLE = 7, AT_DETECTABLE = 8;
  class Drawing extends EventTarget {
    #state = [0, 0, 0, -1, 0, 0, 0, 1, 0];
    constructor(token, handle) {
      if (token !== MAKE_DRAWING) throw new TypeError("a Drawing is made by nt.regis.drawing");
      super();
      fullHandle.set(this, handle);
      this.#state[OWN_HANDLE] = handle;
    }
    static #join(drawing) {
      const state = drawing.#state;
      if (state[PLACE] < 0) { state[PLACE] = alive.length; alive.push(state); }
    }
    // Leaves the page's drawings: the last takes its place.
    static #leave(drawing) {
      const state = drawing.#state;
      const place = state[PLACE];
      if (place < 0) return;
      const last = alive.pop();
      if (last !== state) { alive[place] = last; last[PLACE] = place; }
      state[PLACE] = -1;
    }
    // Asks the host for the five; a drawing that was removed is refused here.
    static #ask(state) {
      const handle = state[OWN_HANDLE];
      state[AT_X] = host(OP.RG_GET, handle, PROP.X);
      state[AT_Y] = host(OP.RG_GET, handle, PROP.Y);
      state[AT_DEPTH] = host(OP.RG_GET, handle, PROP.DEPTH);
      state[AT_VISIBLE] = host(OP.RG_GET, handle, PROP.VISIBLE);
      state[AT_DETECTABLE] = host(OP.RG_GET, handle, PROP.DETECTABLE);
      // In the channel context a drawing is of the page shown, and the
      // page may change between any two callbacks: there a drawing asks
      // every time, and only its assignments are gathered.
      state[EPOCH] = isPage ? epoch : 0;
    }
    static #forget(drawing) { const state = drawing.#state; state[EPOCH] = 0; state[WAITING] = 0; }
    static #made(drawing, x, y, depth, visible, detectable) {
      const state = drawing.#state;
      state[AT_X] = x; state[AT_Y] = y; state[AT_DEPTH] = depth; state[AT_VISIBLE] = +visible; state[AT_DETECTABLE] = +detectable;
      state[EPOCH] = isPage ? epoch : 0;
    }
    static { drawingInternals = { ask: Drawing.#ask, forget: Drawing.#forget, made: Drawing.#made, join: Drawing.#join, leave: Drawing.#leave }; }
    get x() { const state = this.#state; if (state[EPOCH] !== epoch) Drawing.#ask(state); return state[AT_X]; }
    set x(value) {
      if (typeof value !== "number" || (value | 0) !== value || value < LEAST || value > MOST) notACoordinate(value, "x");
      const state = this.#state;
      if (state[EPOCH] !== epoch) Drawing.#ask(state);
      state[AT_X] = value;
      state[WAITING] |= X;
      if (!waits) wait();
    }
    get y() { const state = this.#state; if (state[EPOCH] !== epoch) Drawing.#ask(state); return state[AT_Y]; }
    set y(value) {
      if (typeof value !== "number" || (value | 0) !== value || value < LEAST || value > MOST) notACoordinate(value, "y");
      const state = this.#state;
      if (state[EPOCH] !== epoch) Drawing.#ask(state);
      state[AT_Y] = value;
      state[WAITING] |= Y;
      if (!waits) wait();
    }
    get depth() { const state = this.#state; if (state[EPOCH] !== epoch) Drawing.#ask(state); return state[AT_DEPTH]; }
    set depth(value) {
      if (typeof value !== "number" || (value | 0) !== value || value < LEAST || value > MOST) notACoordinate(value, "depth");
      const state = this.#state;
      if (state[EPOCH] !== epoch) Drawing.#ask(state);
      state[AT_DEPTH] = value;
      state[WAITING] |= DEPTH;
      if (!waits) wait();
    }
    get visible() { const state = this.#state; if (state[EPOCH] !== epoch) Drawing.#ask(state); return state[AT_VISIBLE] === 1; }
    set visible(value) {
      const visible = truth(value, "visible");
      const state = this.#state;
      if (state[EPOCH] !== epoch) Drawing.#ask(state);
      state[AT_VISIBLE] = visible;
      state[WAITING] |= VISIBLE;
      if (!waits) wait();
    }
    get detectable() { const state = this.#state; if (state[EPOCH] !== epoch) Drawing.#ask(state); return state[AT_DETECTABLE] === 1; }
    set detectable(value) {
      const detectable = truth(value, "detectable");
      const state = this.#state;
      if (state[EPOCH] !== epoch) Drawing.#ask(state);
      state[AT_DETECTABLE] = detectable;
      state[WAITING] |= DETECTABLE;
      if (!waits) wait();
    }
    // A listener on a drawing that was removed is refused, whatever
    // drawing has its number now.
    addEventListener(type, callback, options) {
      host(OP.RG_GET, fullHandle.get(this), PROP.X);
      super.addEventListener(type, callback, options);
    }
    get parent() { return drawingOf(host(OP.RG_GET, fullHandle.get(this), PROP.PARENT)); }
    set parent(value) {
      host(OP.RG_SET, fullHandle.get(this), PROP.PARENT, handleArgument(value, "parent"));
      // Pointer events go from a drawing up its parent chain, then to nt
      // (section 4.4).
      parentOf.set(this, value || nt);
    }
    get clip() {
      const text = host(OP.RG_GET, fullHandle.get(this), RG_FIELD.CLIP);
      if (text === undefined) return null;
      const [x, y, width, height] = text.split(",").map(Number);
      return { x, y, width, height };
    }
    set clip(value) {
      if (value === null || value === undefined) { host(OP.RG_CLIP, fullHandle.get(this), 0, 0, 0, 0, 0); return; }
      if (typeof value !== "object") throw new TypeError("clip is { x, y, width, height } or null");
      host(OP.RG_CLIP, fullHandle.get(this), 1, whole(value.x, 0, "clip.x"), whole(value.y, 0, "clip.y"), whole(value.width, undefined, "clip.width"), whole(value.height, undefined, "clip.height"));
    }
    get frames() {
      const text = host(OP.RG_GET, fullHandle.get(this), RG_FIELD.FRAMES);
      return text === undefined ? null : Object.freeze(text.split(",").map((handle) => drawingOf(Number(handle))));
    }
    set frames(value) {
      if (value === null || value === undefined) { host(OP.RG_FRAMES, fullHandle.get(this)); return; }
      if (!Array.isArray(value) || value.length < 1) throw new TypeError("frames is an array of 1 to 16 Drawings, or null");
      const handles = value.map((frame) => {
        const handle = handleArgument(frame, "a frame");
        if (handle === 0) throw new TypeError("a frame is a Drawing");
        return handle;
      });
      host(OP.RG_FRAMES, fullHandle.get(this), new Int32Array(handles));
    }
    get paintRate() {
      const rate = host(OP.RG_GET, fullHandle.get(this), PROP.PAINT_RATE);
      return rate === RG_BOUND.PAINT_RATE_UNSET ? null : rate;
    }
    set paintRate(value) {
      host(OP.RG_SET, fullHandle.get(this), PROP.PAINT_RATE, value === null || value === undefined ? RG_BOUND.PAINT_RATE_UNSET : whole(value, undefined, "paintRate"));
    }
    get label() {
      const text = host(OP.RG_GET, fullHandle.get(this), RG_FIELD.LABEL);
      return text === undefined ? null : text;
    }
    set label(value) {
      if (value === null || value === undefined) { host(OP.RG_LABEL, fullHandle.get(this)); return; }
      if (typeof value !== "string") throw new TypeError("label is a string or null");
      host(OP.RG_LABEL, fullHandle.get(this), value);
    }
    get frame() { return host(OP.RG_GET, fullHandle.get(this), RG_FIELD.FRAME); }
    set frame(value) { host(OP.RG_ANIMATE, fullHandle.get(this), RG_BOUND.ANIMATE_FRAME, whole(value, undefined, "frame"), 0, 0, 0); }
    get look() { return LOOKS[host(OP.RG_GET, fullHandle.get(this), RG_FIELD.LOOK)] || null; }
    set look(value) {
      const look = value === undefined ? 0 : LOOKS.indexOf(value);
      if (look < 0) throw new TypeError("look is null, \"hover\", \"pressed\" or \"alternative\"");
      host(OP.RG_ANIMATE, fullHandle.get(this), RG_BOUND.ANIMATE_LOOK, look, 0, 0, 0);
    }
    cycle(first, last, period, options) {
      const once = options && options.once !== undefined ? truth(options.once, "once") : 0;
      host(OP.RG_ANIMATE, fullHandle.get(this), RG_BOUND.ANIMATE_CYCLE, whole(first, undefined, "first"), whole(last, undefined, "last"), whole(period, undefined, "period"), once);
    }
    stopCycle() { host(OP.RG_ANIMATE, fullHandle.get(this), RG_BOUND.ANIMATE_STOP_CYCLE, 0, 0, 0, 0); }
    draw(fn) {
      host(OP.RG_DEFINE, fullHandle.get(this), programOf(fn));
      return this;
    }
    moveTo(x, y) { Drawing.#place(this.#state, x, y); }
    moveBy(dx, dy) {
      const state = this.#state;
      if (state[EPOCH] !== epoch) Drawing.#ask(state);
      Drawing.#place(state, state[AT_X] + whole(dx, undefined, "dx"), state[AT_Y] + whole(dy, undefined, "dy"));
    }
    // Both of a position at once: checked, then both kept.
    static #place(state, x, y) {
      if (typeof x !== "number" || (x | 0) !== x || x < LEAST || x > MOST) notACoordinate(x, "x");
      if (typeof y !== "number" || (y | 0) !== y || y < LEAST || y > MOST) notACoordinate(y, "y");
      if (state[EPOCH] !== epoch) Drawing.#ask(state);
      state[AT_X] = x;
      state[AT_Y] = y;
      state[WAITING] |= X | Y;
      if (!waits) wait();
    }
    remove() { settleRemoved(host(OP.RG_REMOVE, fullHandle.get(this))); }
    bounds() {
      const text = host(OP.RG_BOUNDS, fullHandle.get(this));
      if (text === undefined) return null;
      const [x, y, width, height] = text.split(",").map(Number);
      return { x, y, width, height };
    }
    overlaps(other) {
      if (!(other instanceof Drawing)) throw new TypeError("overlaps takes a Drawing");
      const mine = this.bounds();
      const theirs = other.bounds();
      return !!mine && !!theirs && mine.x < theirs.x + theirs.width && theirs.x < mine.x + mine.width && mine.y < theirs.y + theirs.height && theirs.y < mine.y + mine.height;
    }
  }
  Object.defineProperties(Drawing.prototype, {
    layer: numberProperty(PROP.LAYER, "layer"),
    wheel: flagProperty(PROP.WHEEL, "wheel"),
    track: flagProperty(PROP.TRACK, "track"),
    source: drawingProperty(PROP.SOURCE, "source"),
    hover: drawingProperty(PROP.HOVER, "hover"),
    pressed: drawingProperty(PROP.PRESSED, "pressed"),
    alternative: drawingProperty(PROP.ALTERNATIVE, "alternative"),
  });
  Object.freeze(Drawing.prototype);
  Object.freeze(RegisPainter.prototype);

  function drawing(options) {
    const o = options === undefined || options === null ? {} : options;
    if (typeof o !== "object") throw new TypeError("a drawing's options are an object");
    for (const name of Object.keys(o)) if (!OPTIONS.has(name)) throw new TypeError("'" + name + "' is not an option of a drawing");
    const flags =
      (o.visible === undefined || truth(o.visible, "visible") ? RG_BOUND.FLAG_VISIBLE : 0) |
      (o.detectable !== undefined && truth(o.detectable, "detectable") ? RG_BOUND.FLAG_DETECTABLE : 0) |
      (o.wheel !== undefined && truth(o.wheel, "wheel") ? RG_BOUND.FLAG_WHEEL : 0) |
      (o.track !== undefined && truth(o.track, "track") ? RG_BOUND.FLAG_TRACK : 0);
    // The channel context draws only in the overlay layer (section 3.1).
    const handle = host(
      OP.RG_DRAWING, handleArgument(o.parent, "parent"), handleArgument(o.source, "source"),
      whole(o.layer, isPage ? RG_BOUND.DEFAULT_LAYER : RG_BOUND.OVERLAY_LAYER, "layer"), whole(o.x, 0, "x"), whole(o.y, 0, "y"), whole(o.depth, 0, "depth"),
      o.paintRate === null || o.paintRate === undefined ? RG_BOUND.PAINT_RATE_UNSET : whole(o.paintRate, undefined, "paintRate"), flags);
    const made = new Drawing(MAKE_DRAWING, handle);
    // The host took these five as they were given.
    drawingInternals.made(made, whole(o.x, 0, "x"), whole(o.y, 0, "y"), whole(o.depth, 0, "depth"), o.visible === undefined || o.visible, !!o.detectable);
    drawingInternals.join(made);
    const number = numberOf(handle);
    // Whatever was counted for a drawing that had this number before is
    // not this drawing's.
    for (const kind of Object.values(EV)) listenCounts.delete(number + ":" + kind);
    removed.delete(number);
    things.set(number, made);
    handleOf.set(made, number);
    parentOf.set(made, o.parent || nt);
    if (!listening) {
      // The host tells a refused commit only to a context that listens.
      host(OP.LISTEN, HANDLE.NT, EV.RG_COMMIT_FAILED, 1);
      listening = true;
    }
    try {
      // The options the making operation does not carry.
      for (const name of ["hover", "pressed", "alternative", "clip", "frames", "label"]) {
        if (o[name] !== undefined && o[name] !== null) made[name] = o[name];
      }
    } catch (error) {
      made.remove();
      throw error;
    }
    core.scopeEnds(() => { try { made.remove(); } catch (error) { /* removed already */ } });
    return made;
  }

  // Many drawings set at once (an addition to section 6.2; TODO-7373):
  // one attribute of every drawing of a list from a typed array of as
  // many values, in one host call, so that what a script computes for
  // two thousand drawings each tick is not two thousand assignments to
  // properties. The attributes are the five a script moves things by.
  const LIST_PROPERTIES = { x: PROP.X, y: PROP.Y, depth: PROP.DEPTH, visible: PROP.VISIBLE, detectable: PROP.DETECTABLE };
  const MAKE_LIST = Symbol("list");
  class DrawingList {
    #numbers;
    #count;
    constructor(token, drawings) {
      if (token !== MAKE_LIST) throw new TypeError("a DrawingList is made by nt.regis.many");
      if (!Array.isArray(drawings)) throw new TypeError("many takes an array of Drawings");
      this.#count = drawings.length;
      // The handles, with room after them for as many values.
      this.#numbers = new Int32Array(2 * drawings.length);
      drawings.forEach((drawing, index) => {
        const handle = handleArgument(drawing, "a drawing of the list");
        if (handle === 0) throw new TypeError("a drawing of the list is a Drawing");
        this.#numbers[index] = handle;
      });
      Object.freeze(this);
    }
    get length() { return this.#count; }
    // `values`: an Int32Array of as many values as the list has drawings,
    // 1 and 0 for true and false.
    set(property, values) {
      if (!Object.prototype.hasOwnProperty.call(LIST_PROPERTIES, property)) throw new TypeError("a list sets x, y, depth, visible or detectable");
      if (!(values instanceof Int32Array) || values.length !== this.#count) throw new TypeError("values is an Int32Array of as many values as the list has drawings");
      this.#numbers.set(values, this.#count);
      host(OP.RG_SET_MANY, LIST_PROPERTIES[property], this.#numbers);
      // The drawings' own record of these attributes is no longer what
      // the host holds: each asks again when it is next read.
      epoch++;
    }
    moveTo(xs, ys) { this.set("x", xs); this.set("y", ys); }
  }
  Object.freeze(DrawingList.prototype);

  function colourArguments(colour) {
    if (typeof colour === "string") {
      const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i.exec(colour);
      const long = /^#([0-9a-f]{6})$/i.exec(colour);
      if (short) return [parseInt(short[1] + short[1] + short[2] + short[2] + short[3] + short[3], 16)];
      if (long) return [parseInt(long[1], 16)];
    } else if (colour && typeof colour === "object") {
      return [whole(colour.h, undefined, "h"), whole(colour.l, undefined, "l"), whole(colour.s, undefined, "s")];
    }
    throw new TypeError("a colour is { h, l, s }, \"#rgb\" or \"#rrggbb\"");
  }

  const colours = Object.freeze({
    define(name, colour) {
      if (typeof name !== "string") throw new TypeError("a colour's name is a string");
      return host(OP.RG_COLOUR_DEFINE, ...colourArguments(colour), name);
    },
    set(entry, colour) { host(OP.RG_COLOUR_SET, whole(entry, undefined, "a colour entry"), ...colourArguments(colour)); },
  });

  const api = Object.freeze({
    colours,
    drawing,
    symbol(fn) { return drawing({ visible: false }).draw(fn); },
    group(options) { return drawing(options); },
    clear() { settleRemoved(host(OP.RG_CLEAR)); },
    many(drawings) { return new DrawingList(MAKE_LIST, drawings); },
    macrograph(fn) { return new Macrograph(MAKE_MACROGRAPH, host(OP.RG_MACROGRAPH, 0, programOf(fn))); },
    // The pixels a string takes at the cell the options select: the
    // renderer has no measuring call, so the host computes it.
    measure(string, options) {
      if (typeof string !== "string") throw new TypeError("measure takes a string");
      const o = options === undefined || options === null ? {} : options;
      const [width, height] = optionPair(o.cell, "cell");
      const packed = host(OP.RG_MEASURE, optionNumber(o.size, "size"), width, height, optionNumber(o.height, "height"), optionNumber(o.tilt, "tilt"), textLanguage(o) + "\0" + string);
      return { width: packed % RG_BOUND.HANDLE_NUMBERS, height: Math.floor(packed / RG_BOUND.HANDLE_NUMBERS) };
    },
  });
  // A page context has the API of its own page's renderer and no other;
  // the channel context has it for whatever page is shown, and the host
  // refuses its calls while that page is not a ReGIS page (section 3.1).
  if (!isPage || host(OP.INFO, INFO.PAGE_RENDERER) === "regis") core.regis = api;
  // What the components on ReGIS (work package U2) build on.
  core.regisInternals = { RegisPainter, programOf, Drawing, Macrograph };
  for (const [name, value] of Object.entries({ Drawing, RegisPainter, Macrograph, DrawingList })) {
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true, enumerable: false });
  }
})();
