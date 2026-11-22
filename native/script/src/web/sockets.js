// The web APIs' third part (work package W2; script-api.md section 9.1):
// HTML's EventSource (section 9.2, server-sent events) and MessageEvent,
// the WebSockets standard's WebSocket and CloseEvent, and the function
// the shared core's dispatcher gives the network's events (30 to 41 of
// section 10.3) to. The event-stream parser, reconnection, the WebSocket
// handshake and framing are the web client's; the objects are here.

// HTML section 9.4.2 (MessageEvent) and WebSockets section 4 (CloseEvent).
class MessageEvent extends Event {
  #data; #origin; #lastEventId; #source; #ports;
  constructor(type, init = undefined) {
    super(type, init);
    const o = init || {};
    this.#data = o.data === undefined ? null : o.data;
    this.#origin = o.origin === undefined ? "" : usv(o.origin);
    this.#lastEventId = o.lastEventId === undefined ? "" : String(o.lastEventId);
    this.#source = o.source === undefined ? null : o.source;
    this.#ports = Object.freeze(o.ports === undefined ? [] : Array.from(o.ports));
  }
  get data() { return this.#data; }
  get origin() { return this.#origin; }
  get lastEventId() { return this.#lastEventId; }
  get source() { return this.#source; }
  get ports() { return this.#ports; }
}

class CloseEvent extends Event {
  #wasClean; #code; #reason;
  constructor(type, init = undefined) {
    super(type, init);
    const o = init || {};
    this.#wasClean = !!o.wasClean;
    this.#code = o.code === undefined ? 0 : Number(o.code) & 0xffff;
    this.#reason = o.reason === undefined ? "" : usv(o.reason);
  }
  get wasClean() { return this.#wasClean; }
  get code() { return this.#code; }
  get reason() { return this.#reason; }
}

// An event handler IDL attribute (HTML section 8.1.8.1): `on` + type,
// one listener that is replaced when the attribute is set.
function handlerAttributes(Class, types) {
  for (const type of types) {
    const slot = Symbol("on" + type);
    Object.defineProperty(Class.prototype, "on" + type, {
      configurable: true,
      enumerable: true,
      get() { return this[slot] ? this[slot].handler : null; },
      set(handler) {
        if (this[slot]) this.removeEventListener(type, this[slot].listener);
        this[slot] = null;
        if (typeof handler !== "function" && (typeof handler !== "object" || handler === null)) return;
        const listener = (event) => { if (typeof handler === "function") return handler.call(this, event); };
        this[slot] = { handler, listener };
        this.addEventListener(type, listener);
      },
    });
  }
}

// EventSource (HTML section 9.2) ----------------------------------------------------

class EventSource extends EventTarget {
  #state = { kind: "stream", id: 0, readyState: 0, url: "", origin: "", withCredentials: false, target: null };
  constructor(url, init = undefined) {
    if (arguments.length < 1) throw new TypeError("an EventSource takes an address");
    super();
    const state = this.#state;
    state.target = this;
    // Section 9.2.2 step 4: an address that does not parse is a SyntaxError.
    const record = parseUrl(url, SCRIPT_URL);
    if (!record) throw domException("'" + url + "' is not a valid address", "SyntaxError");
    state.url = record.href;
    state.origin = record.origin;
    state.withCredentials = !!(init && init.withCredentials);
    // Steps 8 to 14: a GET in mode cors, credentials same-origin or
    // include, cache no-store, redirects followed.
    const request = {
      method: "GET", url: record.href, referrer: CLIENT, integrity: "", headers: headersWith("none", undefined),
      mode: "cors", credentials: state.withCredentials ? "include" : "same-origin", cache: "no-store", redirect: "follow", referrerPolicy: "", keepalive: false,
    };
    let id = 0;
    try {
      id = host(OP.FETCH, fetchFlags(request, 1, false), 0, fetchHead(request, ""));
    } catch (error) {
      id = 0;
    }
    if (id > 0) {
      state.id = id;
      network.set(id, state);
    } else {
      // Past a bound (departure D22), without the network, or refused:
      // "fail the connection" (section 9.2.3), from a task of its own.
      queueMicrotask(() => {
        if (state.readyState === 2) return;
        state.readyState = 2;
        fire(this, trusted(new Event("error")), false);
      });
    }
  }
  get url() { return this.#state.url; }
  get withCredentials() { return this.#state.withCredentials; }
  get readyState() { return this.#state.readyState; }
  close() {
    const state = this.#state;
    if (state.readyState === 2) return;
    state.readyState = 2;
    if (state.id && network.delete(state.id)) host(OP.FETCH_ABORT, state.id);
  }
}
for (const [name, value] of [["CONNECTING", 0], ["OPEN", 1], ["CLOSED", 2]]) {
  Object.defineProperty(EventSource, name, { value, enumerable: true });
  Object.defineProperty(EventSource.prototype, name, { value, enumerable: true });
}
handlerAttributes(EventSource, ["open", "message", "error"]);

// Events 33 and 39 to 41 for an event stream.
function streamEvent(state, kind, a, b, c, payload) {
  const target = state.target;
  switch (kind) {
    case EV.SSE_OPENED:
      // Section 9.2.3, "announce the connection". The final address
      // gives the origin of the stream's events (section 9.2.6); while
      // the web client does not say it, it is the address asked for.
      if (payload) {
        const record = parseUrl(payload, undefined);
        if (record) state.origin = record.origin;
      }
      state.readyState = 1;
      fire(target, trusted(new Event("open")), false);
      break;
    case EV.SSE_EVENT: {
      // Section 9.2.6, "dispatch the event": parsed by the web client.
      if (state.readyState !== 1) break;
      const type = payload.slice(0, b);
      const lastEventId = payload.slice(b, b + c);
      const data = payload.slice(b + c);
      fire(target, trusted(new MessageEvent(type === "" ? "message" : type, { data, origin: state.origin, lastEventId })), false);
      break;
    }
    case EV.SSE_RETRYING:
      // Section 9.2.3, "reestablish the connection".
      state.readyState = 0;
      fire(target, trusted(new Event("error")), false);
      break;
    case EV.FETCH_FAILED:
      // "fail the connection".
      network.delete(state.id);
      state.readyState = 2;
      fire(target, trusted(new Event("error")), false);
      break;
  }
}

// WebSocket (WebSockets standard section 3) -------------------------------------------

const BINARY_TYPES = ["blob", "arraybuffer"];

class WebSocket extends EventTarget {
  #state = {
    kind: "socket", id: 0, readyState: 0, url: "", origin: "", protocol: "", binaryType: "blob", bufferedAmount: 0, target: null,
    pieces: [], pieceBytes: 0,
  };
  constructor(url, protocols = []) {
    if (arguments.length < 1) throw new TypeError("a WebSocket takes an address");
    super();
    const state = this.#state;
    state.target = this;
    // Section 3.1 steps 1 to 6.
    let record = parseUrl(url, SCRIPT_URL);
    if (!record) throw domException("'" + url + "' is not a valid address", "SyntaxError");
    let scheme = record.scheme;
    if (scheme === "http" || scheme === "https") {
      record = urlRecord(host(OP.URL_SET, URL_PART.PROTOCOL, record.href.length, record.href + (scheme === "http" ? "ws" : "wss")));
      scheme = record.scheme;
    }
    if (scheme !== "ws" && scheme !== "wss") throw domException("a WebSocket's address is ws: or wss:", "SyntaxError");
    if (record.href.indexOf("#") >= 0) throw domException("a WebSocket's address has no fragment", "SyntaxError");
    // The second argument is the subprotocols or, as the Local Network
    // Access draft adds, a dictionary of them and `targetAddressSpace`.
    let target = "";
    if (typeof protocols === "object" && protocols !== null && !(Symbol.iterator in protocols)) {
      const options = protocols;
      if (options.targetAddressSpace !== undefined) target = oneOf(options.targetAddressSpace, TARGET_ADDRESS_SPACES, "target address space");
      protocols = options.protocols === undefined ? [] : options.protocols;
    }
    if (typeof protocols === "string") protocols = [protocols];
    protocols = Array.from(protocols, String);
    // Steps 8 and 9: no protocol twice, each a token.
    if (new Set(protocols.map((protocol) => protocol.toLowerCase())).size !== protocols.length) throw domException("a subprotocol is named twice", "SyntaxError");
    for (const protocol of protocols) if (!TOKEN.test(protocol)) throw domException("'" + protocol + "' is not a subprotocol", "SyntaxError");
    if (protocols.length > WEB_BOUND.MAX_WS_PROTOCOLS) throw domException("MAX_WS_PROTOCOLS: too many subprotocols", "SyntaxError");
    state.url = record.href;
    state.origin = record.origin;
    let id = 0;
    try {
      id = host(OP.WS_OPEN, record.href.length, record.href + protocols.join(",") + (target === "" ? "" : "\n" + target));
    } catch (error) {
      id = 0;
    }
    if (id > 0) {
      state.id = id;
      network.set(id, state);
    } else {
      // Past a bound (departure D22), without the network, or refused:
      // the socket fails as one whose connection could not be made.
      queueMicrotask(() => socketClosed(state, WEB_BOUND.WS_ABNORMAL, false, ""));
    }
  }
  get url() { return this.#state.url; }
  get readyState() { return this.#state.readyState; }
  get bufferedAmount() { return this.#state.bufferedAmount; }
  // No extension is ever offered (departure D20).
  get extensions() { return ""; }
  get protocol() { return this.#state.protocol; }
  get binaryType() { return this.#state.binaryType; }
  set binaryType(value) { if (BINARY_TYPES.indexOf(value) >= 0) this.#state.binaryType = value; }
  // Section 3.1, close.
  close(code = undefined, reason = undefined) {
    const state = this.#state;
    if (code !== undefined) {
      // Web IDL's [Clamp] unsigned short.
      code = Math.min(Math.max(Math.round(Number(code)) || 0, 0), 0xffff);
      if (code !== 1000 && (code < 3000 || code > 4999)) throw domException("a close code is 1000 or 3000 to 4999", "InvalidAccessError");
    }
    let text = "";
    if (reason !== undefined) {
      text = usv(reason);
      if (utf8Bytes(text) > WEB_BOUND.MAX_CLOSE_REASON_BYTES) throw domException("a close reason is 123 bytes at most", "SyntaxError");
    }
    if (state.readyState >= 2) return;
    state.readyState = 2;
    if (state.id) host(OP.WS_CLOSE, state.id, code === undefined ? WEB_BOUND.WS_NO_CODE : code, text);
  }
  // Section 3.1, send.
  send(data) {
    if (arguments.length < 1) throw new TypeError("send takes what to send");
    const state = this.#state;
    if (state.readyState === 0) throw domException("the socket is still connecting", "InvalidStateError");
    let bytes;
    let binary = 1;
    if (data instanceof Blob) bytes = blobInternals.bytes(data);
    else if (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) bytes = viewOf(data, "send");
    else { bytes = utf8Encode(usv(data)); binary = 0; }
    state.bufferedAmount += bytes.byteLength;
    // Closing or closed: the data is counted and discarded.
    if (state.readyState !== 1) return;
    let from = 0;
    do {
      const to = Math.min(bytes.byteLength, from + WEB_BOUND.MAX_HOST_BYTES);
      if (host(OP.WS_SEND, state.id, binary, to === bytes.byteLength ? 1 : 0, bytes.subarray(from, to)) !== 1) {
        // The socket is full, or the message is past MAX_WS_MESSAGE_BYTES:
        // "flag the WebSocket as full and then close the WebSocket connection".
        state.readyState = 2;
        host(OP.WS_CLOSE, state.id, WEB_BOUND.WS_TOO_BIG, "");
        return;
      }
      from = to;
    } while (from < bytes.byteLength);
  }
}
for (const [name, value] of [["CONNECTING", 0], ["OPEN", 1], ["CLOSING", 2], ["CLOSED", 3]]) {
  Object.defineProperty(WebSocket, name, { value, enumerable: true });
  Object.defineProperty(WebSocket.prototype, name, { value, enumerable: true });
}
handlerAttributes(WebSocket, ["open", "message", "error", "close"]);

// Section 5, "when the WebSocket connection is closed": `error` first
// when it was not clean, then `close`.
function socketClosed(state, code, clean, reason) {
  if (state.readyState === 3) return;
  if (state.id) network.delete(state.id);
  state.readyState = 3;
  state.pieces = [];
  state.pieceBytes = 0;
  if (!clean) fire(state.target, trusted(new Event("error")), false);
  fire(state.target, trusted(new CloseEvent("close", { wasClean: clean, code, reason })), false);
}

// Events 33 and 35 to 38 for a socket.
function socketEvent(state, kind, a, b, c, payload) {
  switch (kind) {
    case EV.WS_OPENED:
      if (state.readyState !== 0) break;
      state.readyState = 1;
      state.protocol = payload || "";
      fire(state.target, trusted(new Event("open")), false);
      break;
    case EV.WS_MESSAGE: {
      // Section 5, "when a WebSocket message has been received". A message
      // crosses in pieces; the host holds the whole to MAX_WS_MESSAGE_BYTES
      // (departure D21).
      if (state.readyState !== 1) break;
      state.pieces.push(payload);
      state.pieceBytes += payload.byteLength;
      if (b !== 1) break;
      const bytes = concatBytes(state.pieces, state.pieceBytes);
      state.pieces = [];
      state.pieceBytes = 0;
      let data;
      if (a === 0) data = utf8DecodeWithoutBom(bytes);
      else if (state.binaryType === "blob") data = blobInternals.make(bytes, "");
      else data = bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? bytes.buffer : bytes.slice().buffer;
      fire(state.target, trusted(new MessageEvent("message", { data, origin: state.origin })), false);
      break;
    }
    case EV.WS_CLOSED:
      socketClosed(state, a, b === 1, payload || "");
      break;
    case EV.WS_SENT:
      state.bufferedAmount = Math.max(0, state.bufferedAmount - a);
      break;
    case EV.FETCH_FAILED:
      socketClosed(state, WEB_BOUND.WS_ABNORMAL, false, "");
      break;
  }
}

// The network's events (30 to 41 of section 10.3): each names its
// request by the host's number, and goes to the object that made it.
function webDispatch(kind, id, b, c, d, payload) {
  if (kind < EV.FETCH_HEAD || kind > EV.SSE_RETRYING) return 0;
  const object = network.get(id);
  if (!object) return 0;
  // The objects are the prelude's own, so the event is always heard.
  dispatching.handled = true;
  try {
    if (object.kind === "fetch") fetchEvent(object, kind, b, b, c, payload);
    else if (object.kind === "stream") streamEvent(object, kind, b, b, c, payload);
    else socketEvent(object, kind, b, c, d, payload);
  } catch (error) {
    report(error);
  }
  return DISPATCH.HANDLED | (dispatching.threw ? DISPATCH.THREW : 0);
}

for (const [name, value] of Object.entries({ MessageEvent, CloseEvent, EventSource, WebSocket })) define(name, value);
