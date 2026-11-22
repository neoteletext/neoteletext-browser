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
  const { host, things, handleOf, parentOf, nt, EventTarget, whole, report, listenCounts } = core;

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
    const epoch = Number(reply.slice(0, colon));
    for (const [number, record] of removed) if (record.epoch !== epoch) removed.delete(number);
    if (colon + 1 >= reply.length) return;
    for (const text of reply.slice(colon + 1).split(",")) {
      const number = Number(text);
      const drawing = things.get(number);
      if (drawing) {
        removed.set(number, { epoch, drawing });
        things.delete(number);
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
    for (let index = 1; index <= gone && index < numbers.length; index++) things.delete(numbers[index]);
    for (let index = 1 + gone; index < numbers.length; index++) {
      const record = removed.get(numbers[index]);
      if (record) things.set(numbers[index], record.drawing);
    }
    removed.clear();
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
  // ReGIS is written here. Work package R1 has the calls a drawing needs
  // to have any contents; work package R2 adds the rest to this class.
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
    // What work package R2's calls record with.
    static #record(painter, values) { return painter.#push(...values); }
    static { painterInternals = { finish: RegisPainter.#finish, record: RegisPainter.#record }; }
  }
  var painterInternals;

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

  class Drawing extends EventTarget {
    constructor(token, handle) {
      if (token !== MAKE_DRAWING) throw new TypeError("a Drawing is made by nt.regis.drawing");
      super();
      fullHandle.set(this, handle);
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
    moveTo(x, y) {
      const handle = fullHandle.get(this);
      host(OP.RG_SET, handle, PROP.X, whole(x, undefined, "x"));
      host(OP.RG_SET, handle, PROP.Y, whole(y, undefined, "y"));
    }
    moveBy(dx, dy) {
      const handle = fullHandle.get(this);
      host(OP.RG_SET, handle, PROP.X, host(OP.RG_GET, handle, PROP.X) + whole(dx, undefined, "dx"));
      host(OP.RG_SET, handle, PROP.Y, host(OP.RG_GET, handle, PROP.Y) + whole(dy, undefined, "dy"));
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
    x: numberProperty(PROP.X, "x"),
    y: numberProperty(PROP.Y, "y"),
    depth: numberProperty(PROP.DEPTH, "depth"),
    layer: numberProperty(PROP.LAYER, "layer"),
    visible: flagProperty(PROP.VISIBLE, "visible"),
    detectable: flagProperty(PROP.DETECTABLE, "detectable"),
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

  core.regis = Object.freeze({
    colours,
    drawing,
    symbol(fn) { return drawing({ visible: false }).draw(fn); },
    group(options) { return drawing(options); },
    clear() { settleRemoved(host(OP.RG_CLEAR)); },
  });
  // What work package R2 builds on: the painter's class and the way its
  // further calls record their whole numbers, and the program of a
  // function that paints (for macrographs).
  core.regisInternals = { RegisPainter, record: painterInternals.record, programOf, Drawing };
  for (const [name, value] of Object.entries({ Drawing, RegisPainter })) {
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true, enumerable: false });
  }
})();
