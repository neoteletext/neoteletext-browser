
// Teletext sprites, `nt.teletext.block` and `nt.teletext.sprite`
// (script-api.md section 5.3; work package T2). It follows the teletext
// part in the same function, and gives it the two calls its frozen API
// names. A block is a script's own drawing of one frame, kept here; a
// sprite is the host's: its frames become Passive objects on the Object
// page, and the receiver moves, shows, orders and cycles it. Like the
// rest of the prelude this only turns objects into flat host calls:
// every check that matters is the host's.
{
  const { optionsOf, entryOf, textOf, languageOf } = teletextHelpers;

  // A cell's colours: Colour Map entries, or what a Passive object
  // starts with, white on black (EN 300 706 clause 13.5).
  const inkOf = (o) => { const ink = entryOf(o.ink, "ink"); return ink === TT.NONE ? TT.DEFAULT_INK : ink; };
  const paperOf = (o) => { const paper = entryOf(o.paper, "paper"); return paper === TT.NONE ? TT.DEFAULT_PAPER : paper; };
  const colourByte = (entry, name) => {
    if (entry > 255) throw new RangeError("COLOUR_MAP_ENTRIES: " + name + " is a Colour Map entry, 0 to 31, not " + entry);
    return entry;
  };
  const flagOf = (value, name, fallback) => {
    if (value === undefined) return fallback;
    if (typeof value !== "boolean") throw new TypeError(name + " is true or false");
    return value;
  };

  // Blocks: a rectangle of cells, each never set (transparent), a mosaic
  // or a character, as the six bytes operation TT_SPRITE takes.
  const blockStates = new WeakMap();
  const blockOf = (block) => {
    const state = blockStates.get(block);
    if (!state) throw new TypeError("not a block");
    return state;
  };
  const cellAt = (state, row, column) => {
    whole(row, undefined, "row");
    whole(column, undefined, "column");
    if (!(row >= 0 && row < state.rows)) throw new RangeError("TELETEXT_ROWS: a row of this block is 0 to " + (state.rows - 1) + ", not " + row);
    if (!(column >= 0 && column < state.columns)) throw new RangeError("COLUMNS: a column of this block is 0 to " + (state.columns - 1) + ", not " + column);
    return (row * state.columns + column) * TT.CELL_BYTES;
  };
  const setCell = (state, at, kind, code, ink, paper, mode, sets) => {
    state.cells[at] = kind;
    state.cells[at + 1] = code;
    state.cells[at + 2] = colourByte(ink, "ink");
    state.cells[at + 3] = colourByte(paper, "paper");
    state.cells[at + 4] = mode;
    state.cells[at + 5] = sets;
  };
  // A text as cells: the engine's encoder is asked, since a character may
  // be a letter with a mark or one of another set (section 5.6).
  const encoded = (text, language) => host(OP.TT_BLOCK_TEXT, languageOf(language), textOf(text));
  const putText = (state, row, column, bytes, ink, paper) => {
    const count = bytes.length / TT.TEXT_BYTES;
    const first = cellAt(state, row, column);
    if (column + count > state.columns) throw new RangeError("COLUMNS: the text takes " + count + " cells from column " + column + " of a block " + state.columns + " wide");
    for (let index = 0; index < count; index++) {
      const from = index * TT.TEXT_BYTES;
      setCell(state, first + index * TT.CELL_BYTES, TT.CELL_CHARACTER, bytes[from + 1], ink, paper, bytes[from], bytes[from + 2]);
    }
  };
  class Block {
    get columns() { return blockOf(this).columns; }
    get rows() { return blockOf(this).rows; }
    // One cell: a character, or a mosaic given as six bits, bit 0 the
    // top left sixel and bit 5 the bottom right; with neither, a space
    // in the cell's paper.
    cell(row, column, options) {
      const state = blockOf(this);
      const o = optionsOf(options);
      const at = cellAt(state, row, column);
      const ink = inkOf(o);
      const paper = paperOf(o);
      const separated = flagOf(o.separated, "separated", false);
      if (o.char !== undefined && o.sixels !== undefined) throw new TypeError("a cell is a character or a mosaic, not both");
      if (o.sixels !== undefined) {
        const sixels = whole(o.sixels, undefined, "sixels");
        if (!(sixels >= 0 && sixels <= TT.LAST_SIXELS)) throw new RangeError("MOSAIC_PATTERNS: a mosaic is six sixels, 0 to " + TT.LAST_SIXELS + ", not " + sixels);
        setCell(state, at, separated ? TT.CELL_SEPARATED : TT.CELL_CONTIGUOUS, sixels, ink, paper, 0, 0);
      } else if (o.char !== undefined) {
        const bytes = encoded(o.char, o.language);
        if (bytes.length !== TT.TEXT_BYTES) throw new TypeError("char is one character, which takes one cell");
        putText(state, row, column, bytes, ink, paper);
      } else {
        setCell(state, at, TT.CELL_CHARACTER, TT.SPACE, ink, paper, TT.SPACE_MODE, TT.NO_SETS);
      }
      return this;
    }
    // Mosaics from lines of "0" and "1", two characters a cell across and
    // three lines a cell down, for the whole block. A cell with no sixel
    // lit is left as it was: transparent, if it was never set.
    sixels(lines, options) {
      const state = blockOf(this);
      const o = optionsOf(options);
      const ink = inkOf(o);
      const paper = paperOf(o);
      const kind = flagOf(o.separated, "separated", false) ? TT.CELL_SEPARATED : TT.CELL_CONTIGUOUS;
      if (!Array.isArray(lines) || lines.length !== state.rows * 3) throw new TypeError("a block of " + state.rows + " rows is drawn by " + state.rows * 3 + " lines of sixels");
      const patterns = [];
      for (let line = 0; line < lines.length; line++) {
        const text = lines[line];
        if (typeof text !== "string" || text.length !== state.columns * 2 || /[^01]/.test(text)) {
          throw new TypeError("a line of sixels for a block " + state.columns + " wide is " + state.columns * 2 + " characters, each \"0\" or \"1\"");
        }
        for (let across = 0; across < text.length; across++) {
          if (text[across] !== "1") continue;
          const cell = ((line - (line % 3)) / 3) * state.columns + (across >> 1);
          patterns[cell] = (patterns[cell] || 0) | (1 << ((line % 3) * 2 + (across & 1)));
        }
      }
      for (let cell = 0; cell < state.rows * state.columns; cell++) {
        if (patterns[cell]) setCell(state, cell * TT.CELL_BYTES, kind, patterns[cell], ink, paper, 0, 0);
      }
      return this;
    }
    // Characters along a row of the block.
    text(row, column, text, options) {
      const state = blockOf(this);
      const o = optionsOf(options);
      const ink = inkOf(o);
      const paper = paperOf(o);
      cellAt(state, row, column);
      putText(state, row, column, encoded(text, o.language), ink, paper);
      return this;
    }
    // Makes every cell transparent again.
    clear() {
      blockOf(this).cells.fill(0);
      return this;
    }
  }
  Object.freeze(Block.prototype);
  const block = (columns, rows) => {
    whole(columns, undefined, "columns");
    whole(rows, undefined, "rows");
    if (!(columns >= 1 && columns <= TT.MAX_BLOCK_COLUMNS)) throw new RangeError("COLUMNS: a block is 1 to " + TT.MAX_BLOCK_COLUMNS + " columns, not " + columns);
    if (!(rows >= 1 && rows <= TT.MAX_BLOCK_ROWS)) throw new RangeError("TELETEXT_ROWS: a block is 1 to " + TT.MAX_BLOCK_ROWS + " rows, not " + rows);
    const made = new Block();
    blockStates.set(made, { columns, rows, cells: new Uint8Array(columns * rows * TT.CELL_BYTES) });
    return made;
  };

  // Sprites: things, whose pointer and wheel events go on to nt.
  const spriteStates = new WeakMap();
  const live = new Set();
  const stateOf = (sprite) => {
    const state = spriteStates.get(sprite);
    if (!state) throw new TypeError("not a sprite");
    return state;
  };
  const liveOf = (sprite) => {
    const state = stateOf(sprite);
    if (state.removed) throw new TypeError("the sprite was removed");
    return state;
  };
  const animate = (state, operation, a = 0, b = 0, c = 0, d = 0) => host(OP.TT_ANIMATE, state.handle, operation, a, b, c, d);
  const unregister = (sprite, state) => {
    state.removed = true;
    live.delete(sprite);
    things.delete(state.handle);
    handleOf.delete(sprite);
    parentOf.delete(sprite);
  };
  // Forgets a sprite here: its listeners first, while the host still
  // knows its handle, so that the host is told each is gone.
  const forget = (sprite) => {
    const state = spriteStates.get(sprite);
    if (!state || state.removed) return false;
    const byType = listenersOf.get(sprite);
    if (byType) for (const [type, list] of Array.from(byType)) for (const record of list.slice()) removeRecord(sprite, type, record);
    unregister(sprite, state);
    return true;
  };
  // The frame a cycle shows at the tick `now`, by the receiver's own rule
  // (ANIM-14): it began at the tick after its call.
  const cycleFrame = (cycle, now) => {
    const steps = Math.max(0, Math.floor((now - cycle.start) / cycle.period));
    const span = cycle.last - cycle.first;
    return cycle.first + (cycle.once ? Math.min(steps, span) : steps % (span + 1));
  };
  class Sprite extends EventTarget {
    get row() { return stateOf(this).row; }
    set row(row) { this.moveTo(row, stateOf(this).column); }
    get column() { return stateOf(this).column; }
    set column(column) { this.moveTo(stateOf(this).row, column); }
    get columns() { return stateOf(this).columns; }
    get rows() { return stateOf(this).rows; }
    get frames() { return stateOf(this).frames; }
    get detectable() { return stateOf(this).detectable; }
    get wheel() { return stateOf(this).wheel; }
    moveTo(row, column) {
      const state = liveOf(this);
      animate(state, TT.MOVE, TT.MOVE_TO, whole(column, undefined, "column"), whole(row, undefined, "row"));
      state.row = row;
      state.column = column;
    }
    moveBy(rows, columns) {
      const state = liveOf(this);
      animate(state, TT.MOVE, TT.MOVE_BY, whole(columns, undefined, "columns"), whole(rows, undefined, "rows"));
      state.row += rows;
      state.column += columns;
    }
    get shown() { return stateOf(this).shown; }
    set shown(shown) {
      const state = liveOf(this);
      if (typeof shown !== "boolean") throw new TypeError("shown is true or false");
      animate(state, shown ? TT.SHOW : TT.HIDE);
      state.shown = shown;
    }
    get layer() { return stateOf(this).layer; }
    set layer(layer) {
      const state = liveOf(this);
      animate(state, TT.ORDER, whole(layer, undefined, "layer"), state.order);
      state.layer = layer;
    }
    get order() { return stateOf(this).order; }
    set order(order) {
      const state = liveOf(this);
      animate(state, TT.ORDER, TT.KEEP_LAYER, whole(order, undefined, "order"));
      state.order = order;
    }
    // The frame shown. While a cycle runs the receiver changes it, and
    // this follows by the cycle's own rule.
    get frame() {
      const state = stateOf(this);
      return state.cycle ? cycleFrame(state.cycle, host(OP.NOW)) : state.frame;
    }
    set frame(frame) {
      const state = liveOf(this);
      animate(state, TT.FRAME, whole(frame, undefined, "frame"));
      state.frame = frame;
      state.cycle = null;
    }
    get cycling() {
      const state = stateOf(this);
      if (state.cycle && state.cycle.once && cycleFrame(state.cycle, host(OP.NOW)) === state.cycle.last) {
        state.frame = state.cycle.last;
        state.cycle = null;
      }
      return !!state.cycle;
    }
    cycle(first, last, period, options) {
      const state = liveOf(this);
      const once = flagOf(optionsOf(options).once, "once", false);
      animate(state, TT.CYCLE, whole(first, undefined, "first"), whole(last, undefined, "last"), whole(period, undefined, "period"), once ? 1 : 0);
      state.frame = first;
      state.cycle = { first, last, period, once, start: host(OP.NOW) + 1 };
    }
    stopCycle() {
      const state = liveOf(this);
      animate(state, TT.STOP_CYCLE);
      if (state.cycle) state.frame = cycleFrame(state.cycle, host(OP.NOW));
      state.cycle = null;
    }
    get label() { return stateOf(this).label; }
    set label(text) {
      const state = liveOf(this);
      if (typeof text !== "string") throw new TypeError("a label is a string");
      host(OP.TT_LABEL, state.handle, text);
      state.label = text;
    }
    remove() {
      const state = stateOf(this);
      if (state.removed) return;
      const handle = state.handle;
      forget(this);
      host(OP.TT_REMOVE, handle);
    }
    // In cells (section 3.9), cut to the page as the receiver cuts it
    // (ANIM-13): null while hidden, removed, or wholly off the page.
    bounds() {
      const state = stateOf(this);
      if (state.removed || !state.shown) return null;
      const left = Math.max(0, state.column);
      const top = Math.max(0, state.row);
      const right = Math.min(TT.PAGE_COLUMNS, state.column + state.columns);
      const bottom = Math.min(TT.PAGE_ROWS, state.row + state.rows);
      return right > left && bottom > top ? { x: left, y: top, width: right - left, height: bottom - top } : null;
    }
    overlaps(other) {
      const a = this.bounds();
      const b = other && typeof other.bounds === "function" ? other.bounds() : null;
      return !!a && !!b && a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
    }
  }
  Object.freeze(Sprite.prototype);
  const sprite = (options) => {
    if (options === undefined || options === null || typeof options !== "object") throw new TypeError("a sprite is made from an object of options");
    const o = options;
    if (!Array.isArray(o.frames) || o.frames.length < 1) throw new TypeError("frames is an array of blocks");
    if (o.frames.length > TT.MAX_OBJECT_FRAMES) throw new RangeError("MAX_OBJECT_FRAMES: a sprite has at most " + TT.MAX_OBJECT_FRAMES + " frames, not " + o.frames.length);
    const frames = o.frames.map(blockOf);
    const { columns, rows } = frames[0];
    if (frames.some((frame) => frame.columns !== columns || frame.rows !== rows)) throw new TypeError("the frames of a sprite are blocks of one size");
    if (o.label !== undefined && typeof o.label !== "string") throw new TypeError("a label is a string");
    const row = whole(o.row, undefined, "row");
    const column = whole(o.column, undefined, "column");
    if (row === undefined || column === undefined) throw new TypeError("a sprite has a row and a column");
    // The channel context draws in the overlay's layer alone (section 3.1).
    const layer = whole(o.layer, isPage ? TT.SPRITE_LAYER : TT.CHANNEL_LAYER, "layer");
    const order = whole(o.order, 0, "order");
    const shown = flagOf(o.shown, "shown", true);
    const detectable = flagOf(o.detectable, "detectable", false);
    const wheel = flagOf(o.wheel, "wheel", false);
    const cells = new Uint8Array(frames.length * frames[0].cells.length);
    frames.forEach((frame, index) => cells.set(frame.cells, index * frame.cells.length));
    const flags = (shown ? TT.SPRITE_SHOWN : 0) | (detectable ? TT.SPRITE_DETECTABLE : 0) | (wheel ? TT.SPRITE_WHEEL : 0);
    const handle = host(OP.TT_SPRITE, columns, rows, frames.length, layer, order, row, column, flags, cells);
    const made = new Sprite();
    spriteStates.set(made, { handle, row, column, columns, rows, frames: frames.length, layer, order, shown, detectable, wheel, frame: 0, cycle: null, label: "", removed: false });
    things.set(handle, made);
    handleOf.set(made, handle);
    parentOf.set(made, nt);
    live.add(made);
    core.scopeEnds(() => { forget(made); });
    if (o.label !== undefined) made.label = o.label;
    return made;
  };
  // Everything the channel context drew is dropped with the page (section
  // 3.1): the host has dropped its sprites already, listeners and all, so
  // they are only forgotten here.
  if (!isPage) {
    core.pageHides.push(() => {
      for (const one of Array.from(live)) {
        const state = spriteStates.get(one);
        listenersOf.delete(one);
        for (const key of Array.from(listenCounts.keys())) if (key.startsWith(state.handle + ":")) listenCounts.delete(key);
        unregister(one, state);
      }
    });
  }
  teletextSprites = Object.freeze({ block, sprite });
}
