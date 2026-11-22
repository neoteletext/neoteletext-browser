
// Components, `nt.ui` (script-api.md section 8; work package U1). A
// component is the host's: its options, its view state, the focus and
// what a key means to it are all there, and its drawing is the page's
// renderer's own. This part only turns options into the JSON a host call
// carries, and turns what the host says a component would do into the
// component's event, whose default action is taken when no listener
// cancels it. It is the same on every renderer.
let uiApi;
{
  const TYPES = { [UI.MOVE]: "move", [UI.ACTIVATE]: "activate", [UI.CHANGE]: "change", [UI.PAGE]: "page", [UI.SUBMIT]: "submit", [UI.CANCEL]: "cancel" };
  // The events whose default action a listener may cancel.
  const CANCELABLE = { activate: true, change: true, move: true, page: true };
  const KINDS = {
    frame: UI.FRAME, text: UI.TEXT, paragraph: UI.PARAGRAPH, band: UI.BAND, link: UI.LINK, button: UI.BUTTON, menu: UI.MENU,
    table: UI.TABLE, field: UI.FIELD, toggle: UI.TOGGLE, slider: UI.SLIDER, progress: UI.PROGRESS, clock: UI.CLOCK,
  };
  const states = new WeakMap();
  const stateOf = (component) => {
    const state = states.get(component);
    if (!state) throw new TypeError("not a component");
    return state;
  };
  const liveOf = (component) => {
    const state = stateOf(component);
    if (state.removed) throw new TypeError("the component was removed");
    return state;
  };
  // Options cross as strict JSON, which the host reads and checks.
  const encode = (options, withoutAt) => {
    if (options === undefined) return "{}";
    if (options === null || typeof options !== "object" || Array.isArray(options)) throw new TypeError("a component's options are an object");
    const given = withoutAt ? Object.assign({}, options, { at: undefined }) : options;
    const text = JSON.stringify(given);
    if (typeof text !== "string") throw new TypeError("a component's options are an object");
    return text;
  };
  // Forgets a component here: its listeners first, while the host still
  // knows its handle, so that the host is told each is gone.
  const forget = (component) => {
    const state = states.get(component);
    if (!state || state.removed) return false;
    const byType = listenersOf.get(component);
    if (byType) for (const [type, list] of Array.from(byType)) for (const record of list.slice()) removeRecord(component, type, record);
    state.removed = true;
    things.delete(state.handle);
    handleOf.delete(component);
    parentOf.delete(component);
    return true;
  };
  class Component extends EventTarget {
    get kind() { return stateOf(this).kind; }
    get id() { return stateOf(this).id; }
    // The component's value as the host holds it: a menu's highlighted
    // item's value, a field's text, a toggle's state, a slider's number.
    get value() { return JSON.parse(host(OP.UI_VALUE, liveOf(this).handle)); }
    set(options) {
      const state = liveOf(this);
      host(OP.UI_SET, state.handle, encode(options, false));
      if (options && typeof options.id === "string") state.id = options.id;
      return this;
    }
    focus() { host(OP.UI_FOCUS, liveOf(this).handle); }
    remove() {
      const state = stateOf(this);
      if (state.removed) return;
      const handle = state.handle;
      forget(this);
      host(OP.UI_REMOVE, handle);
    }
  }
  Object.freeze(Component.prototype);
  const make = (name) => (options) => {
    if (options === undefined || options === null || typeof options !== "object") throw new TypeError("a component is made from an object of options");
    const at = options.at;
    if (at === undefined || at === null || typeof at !== "object") throw new TypeError("a component has at: { row, column, columns, rows }");
    const place = [at.row, at.column, at.columns, at.rows].map((value, index) => {
      const number = whole(value, index === 3 ? 1 : undefined, "at." + ["row", "column", "columns", "rows"][index]);
      if (number === undefined) throw new TypeError("a component has at: { row, column, columns, rows }");
      return number;
    });
    const handle = host(OP.UI_CREATE, KINDS[name], place[0], place[1], place[2], place[3], encode(options, true));
    const made = new Component();
    states.set(made, { handle, kind: name, id: typeof options.id === "string" ? options.id : "", removed: false });
    things.set(handle, made);
    handleOf.set(made, handle);
    parentOf.set(made, nt);
    core.scopeEnds(() => { forget(made); });
    return made;
  };

  // A component's event, and its default action: the host carries the
  // change out only when no listener cancelled it, and a target is gone
  // to as nt.navigate goes.
  const report = (offered) => {
    const component = things.get(offered.h);
    const type = TYPES[offered.what];
    if (!component || !type) return false;
    const event = withFields(new Event(type, { cancelable: CANCELABLE[type] === true }), { component, value: offered.value });
    fire(component, trusted(event), true);
    const go = !event.defaultPrevented;
    host(OP.UI_SETTLE, offered.h, go ? 1 : 0);
    if (go && typeof offered.target === "string") nt.navigate(offered.target);
    return true;
  };
  const offer = (kind, a, b, payload) => {
    const text = payload === undefined ? host(OP.UI_OFFER, kind, a, b) : host(OP.UI_OFFER, kind, a, b, payload);
    return text === undefined ? null : JSON.parse(text);
  };
  core.ui = {
    // A key or typed text: the focused component is its target, and is
    // offered it once the listeners have let it through.
    key(kind, bits, focused, payload) {
      const target = things.get(focused) || null;
      return {
        target,
        settle(prevented) {
          if (prevented) return false;
          const offered = offer(kind, bits, 0, payload);
          if (!offered) return false;
          if (offered.what) report(offered);
          return true;
        },
      };
    },
    // The pointer on a thing of the renderer that is no thing of the
    // script's: the component it is a part of, when it is one.
    pointer(thing, action) {
      const offered = offer(EV.POINTER, thing, action);
      if (!offered) return null;
      return {
        target: things.get(offered.h) || null,
        settle(prevented) {
          if (!offered.what) return false;
          if (prevented) host(OP.UI_SETTLE, offered.h, 0);
          else report(offered);
          return !prevented;
        },
      };
    },
  };

  const api = { keys(kind) { if (!Object.prototype.hasOwnProperty.call(KINDS, kind)) throw new TypeError("no such kind of component"); return JSON.parse(host(OP.UI_KEYS, KINDS[kind])); } };
  for (const name of Object.keys(KINDS)) api[name] = make(name);
  // Components are the page context's: they are drawn on the page.
  if (isPage) uiApi = Object.freeze(api);
}
