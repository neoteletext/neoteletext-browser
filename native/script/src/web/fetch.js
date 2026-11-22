// The web APIs' second part (work package W2; script-api.md section 9):
// the File API's Blob and File, XMLHttpRequest's FormData, and the Fetch
// standard's Headers, Request, Response and fetch, over host operations
// 41 to 44. A script never touches the network (section 9.2): fetch turns
// its Request into one flat start for the partition's web client, which
// alone applies the network's rules, and the answer returns as events.
// No rule of the network is decided here (DEC-7260).

// Blob and File (File API) -------------------------------------------------------

const blobInternals = { bytes: null, make: null };

// File API section 3.1, "process blob parts".
function blobParts(parts, endings) {
  if (parts === undefined) return new Uint8Array(0);
  if (parts === null || typeof parts !== "object" || typeof parts[Symbol.iterator] !== "function") throw new TypeError("a blob's parts are a sequence");
  const chunks = [];
  let total = 0;
  for (const part of parts) {
    let chunk;
    if (part instanceof Blob) chunk = blobInternals.bytes(part);
    else if (part instanceof ArrayBuffer || ArrayBuffer.isView(part)) chunk = viewOf(part, "a blob").slice();
    else {
      let text = usv(part);
      // "native" line endings are a line feed here, as on every system
      // but one the standard names.
      if (endings === "native") text = text.replace(/\r\n?/g, "\n");
      chunk = utf8Encode(text);
    }
    chunks.push(chunk);
    total += chunk.byteLength;
  }
  return chunks.length === 1 ? chunks[0] : concatBytes(chunks, total);
}

// A blob's type: lower case, or empty when it holds a character outside
// U+0020 to U+007E (File API section 3.1).
function blobType(type) {
  const text = String(type);
  return /^[\x20-\x7e]*$/.test(text) ? text.toLowerCase() : "";
}

class Blob {
  #bytes;
  #type;
  constructor(parts = undefined, options = undefined) {
    const endings = options && options.endings !== undefined ? String(options.endings) : "transparent";
    if (endings !== "transparent" && endings !== "native") throw new TypeError("endings is 'transparent' or 'native'");
    this.#bytes = blobParts(parts, endings);
    this.#type = options && options.type !== undefined ? blobType(options.type) : "";
  }
  get size() { return this.#bytes.byteLength; }
  get type() { return this.#type; }
  // File API section 3.3.1, slice.
  slice(start = 0, end = undefined, contentType = "") {
    const size = this.#bytes.byteLength;
    const clamp = (value) => {
      value = Math.trunc(Number(value)) || 0;
      return value < 0 ? Math.max(size + value, 0) : Math.min(value, size);
    };
    const from = clamp(start);
    const to = end === undefined ? size : clamp(end);
    return blobInternals.make(this.#bytes.subarray(from, Math.max(from, to)), blobType(contentType));
  }
  stream() {
    const bytes = this.#bytes;
    return new ReadableStream({
      type: "bytes",
      pull(controller) {
        if (bytes.byteLength > 0) controller.enqueue(bytes.slice());
        controller.close();
      },
    });
  }
  text() { return Promise.resolve().then(() => utf8Decode(this.#bytes)); }
  arrayBuffer() { return Promise.resolve().then(() => this.#bytes.slice().buffer); }
  bytes() { return Promise.resolve().then(() => this.#bytes.slice()); }
  get [Symbol.toStringTag]() { return "Blob"; }
  static {
    // The bytes are never changed once the blob is made, so they are shared.
    blobInternals.bytes = (blob) => blob.#bytes;
    blobInternals.make = (bytes, type) => {
      const blob = new Blob();
      blob.#bytes = bytes;
      blob.#type = type;
      return blob;
    };
  }
}

class File extends Blob {
  #name;
  #lastModified;
  constructor(parts, name, options = undefined) {
    if (arguments.length < 2) throw new TypeError("a File takes its parts and its name");
    super(parts, options);
    this.#name = usv(name);
    const modified = options && options.lastModified !== undefined ? Number(options.lastModified) : Date.now();
    this.#lastModified = Number.isFinite(modified) ? Math.trunc(modified) : 0;
  }
  get name() { return this.#name; }
  get lastModified() { return this.#lastModified; }
  get webkitRelativePath() { return ""; }
  get [Symbol.toStringTag]() { return "File"; }
}

// FormData (XMLHttpRequest standard section 5) ------------------------------------

const formInternals = { list: null };

// "create an entry" (XMLHttpRequest standard section 5.1).
function formEntry(name, value, filename) {
  name = usv(name);
  if (value instanceof Blob) {
    if (!(value instanceof File) || filename !== undefined) {
      value = new File([value], filename !== undefined ? filename : value instanceof File ? value.name : "blob", { type: value.type, lastModified: value instanceof File ? value.lastModified : undefined });
    }
  } else {
    if (filename !== undefined) throw new TypeError("only a Blob is given a file name");
    value = usv(value);
  }
  return [name, value];
}

class FormData {
  #list = [];
  constructor(form = undefined, submitter = undefined) {
    // There is no document, so there is no form to read.
    if (form !== undefined || (submitter !== undefined && submitter !== null)) throw new TypeError("there is no form element here");
  }
  append(name, value, filename = undefined) {
    if (arguments.length < 2) throw new TypeError("append takes a name and a value");
    this.#list.push(formEntry(name, value, filename));
  }
  delete(name) {
    name = usv(name);
    this.#list = this.#list.filter((entry) => entry[0] !== name);
  }
  get(name) {
    name = usv(name);
    const found = this.#list.find((entry) => entry[0] === name);
    return found ? found[1] : null;
  }
  getAll(name) {
    name = usv(name);
    return this.#list.filter((entry) => entry[0] === name).map((entry) => entry[1]);
  }
  has(name) {
    name = usv(name);
    return this.#list.some((entry) => entry[0] === name);
  }
  set(name, value, filename = undefined) {
    if (arguments.length < 2) throw new TypeError("set takes a name and a value");
    const entry = formEntry(name, value, filename);
    const first = this.#list.findIndex((one) => one[0] === entry[0]);
    if (first < 0) {
      this.#list.push(entry);
    } else {
      this.#list[first] = entry;
      this.#list = this.#list.filter((one, index) => index <= first || one[0] !== entry[0]);
    }
  }
  forEach(callback, thisArg = undefined) {
    if (typeof callback !== "function") throw new TypeError("forEach takes a function");
    for (let index = 0; index < this.#list.length; index++) callback.call(thisArg, this.#list[index][1], this.#list[index][0], this);
  }
  *entries() { for (let index = 0; index < this.#list.length; index++) yield [this.#list[index][0], this.#list[index][1]]; }
  *keys() { for (let index = 0; index < this.#list.length; index++) yield this.#list[index][0]; }
  *values() { for (let index = 0; index < this.#list.length; index++) yield this.#list[index][1]; }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "FormData"; }
  static {
    formInternals.list = (form) => form.#list;
  }
}

// HTML section 4.10.21.8, the multipart/form-data encoding algorithm.
function multipartEncode(form) {
  // The boundary is the host's random bytes: no part can be made to hold it.
  const random = host(OP.RANDOM, 16);
  let boundary = "----NeoTeletextFormBoundary";
  for (let index = 0; index < random.length; index++) boundary += HEX[random[index]];
  const escape = (text) => text.replace(/\n/g, "%0A").replace(/\r/g, "%0D").replace(/"/g, "%22");
  const chunks = [];
  let total = 0;
  const push = (bytes) => { chunks.push(bytes); total += bytes.byteLength; };
  for (const [name, value] of formInternals.list(form)) {
    let head = "--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + escape(name.replace(/\r\n?|\n/g, "\r\n")) + "\"";
    if (typeof value === "string") {
      push(utf8Encode(head + "\r\n\r\n" + value.replace(/\r\n?|\n/g, "\r\n") + "\r\n"));
    } else {
      head += "; filename=\"" + escape(value.name) + "\"\r\nContent-Type: " + (value.type || "application/octet-stream") + "\r\n\r\n";
      push(utf8Encode(head));
      push(blobInternals.bytes(value));
      push(utf8Encode("\r\n"));
    }
  }
  push(utf8Encode("--" + boundary + "--\r\n"));
  return { bytes: concatBytes(chunks, total), type: "multipart/form-data; boundary=" + boundary };
}

function indexOfBytes(bytes, pattern, from) {
  const last = bytes.byteLength - pattern.byteLength;
  outer: for (let at = from; at <= last; at++) {
    for (let index = 0; index < pattern.byteLength; index++) if (bytes[at + index] !== pattern[index]) continue outer;
    return at;
  }
  return -1;
}

// A parameter of a header value: `name=value` or `name="value"`.
function headerParameter(text, name) {
  const pattern = new RegExp("(?:^|;)\\s*" + name + "\\s*=\\s*(?:\"((?:[^\"\\\\]|\\\\.)*)\"|([^;]*))", "i");
  const found = pattern.exec(text);
  if (!found) return null;
  return found[1] !== undefined ? found[1].replace(/\\(.)/g, "$1") : found[2].trim();
}

// A multipart/form-data body (RFC 7578) as a FormData, or a TypeError
// (Fetch standard section 5.3, formData).
function multipartParse(bytes, contentType) {
  const boundary = headerParameter(contentType.slice(contentType.indexOf(";") + 1), "boundary");
  if (!boundary) throw new TypeError("the multipart body names no boundary");
  const form = new FormData();
  const delimiter = utf8Encode("--" + boundary);
  const crlfDelimiter = utf8Encode("\r\n--" + boundary);
  const blank = utf8Encode("\r\n\r\n");
  let at = indexOfBytes(bytes, delimiter, 0);
  if (at < 0) throw new TypeError("the multipart body has no part");
  for (;;) {
    at += delimiter.byteLength;
    // The closing delimiter ends with two hyphens.
    if (bytes[at] === 0x2d && bytes[at + 1] === 0x2d) return form;
    // Past the delimiter's line: optional white space, then CRLF.
    while (bytes[at] === 0x20 || bytes[at] === 0x09) at++;
    if (bytes[at] !== 0x0d || bytes[at + 1] !== 0x0a) throw new TypeError("the multipart body is malformed");
    at += 2;
    const headEnd = indexOfBytes(bytes, blank, at - 2);
    if (headEnd < 0) throw new TypeError("the multipart body is malformed");
    const head = headEnd < at ? "" : utf8DecodeWithoutBom(bytes.subarray(at, headEnd));
    const bodyStart = headEnd + blank.byteLength;
    const bodyEnd = indexOfBytes(bytes, crlfDelimiter, bodyStart - 2 < at - 2 ? bodyStart : bodyStart - 2);
    if (bodyEnd < 0) throw new TypeError("the multipart body is malformed");
    const body = bytes.subarray(bodyStart, Math.max(bodyStart, bodyEnd));
    let name = null;
    let filename = null;
    let type = "";
    for (const line of head.split("\r\n")) {
      const colon = line.indexOf(":");
      if (colon < 0) continue;
      const field = line.slice(0, colon).trim().toLowerCase();
      const value = line.slice(colon + 1).trim();
      if (field === "content-disposition") {
        name = headerParameter(value, "name");
        filename = headerParameter(value, "filename");
      } else if (field === "content-type") {
        type = value;
      }
    }
    if (name === null) throw new TypeError("a multipart part names no field");
    if (filename !== null) form.append(name, new File([body], filename, { type }));
    else form.append(name, utf8DecodeWithoutBom(body));
    at = bodyEnd + 2;
  }
}

// Headers (Fetch standard sections 2.2 and 5.1) -----------------------------------

const TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

// Web IDL's ByteString: a character for each byte, none past U+00FF.
function byteString(value, what) {
  const text = String(value);
  if (/[^\x00-\xff]/.test(text)) throw new TypeError(what + " holds a character that is no byte");
  return text;
}

// "normalize" a header value (section 2.2.2): no HTTP white space at either end.
function normalizeValue(value) {
  return value.replace(/^[\t\n\r ]+|[\t\n\r ]+$/g, "");
}

// Section 2.2.2, forbidden request-header: what a script may not set
// because the network layer owns it. The web client removes them again.
const FORBIDDEN_REQUEST_HEADERS = new Set([
  "accept-charset", "accept-encoding", "access-control-request-headers", "access-control-request-method", "connection", "content-length",
  "cookie", "cookie2", "date", "dnt", "expect", "host", "keep-alive", "origin", "referer", "set-cookie", "te", "trailer",
  "transfer-encoding", "upgrade", "via",
]);
const METHOD_OVERRIDE_HEADERS = new Set(["x-http-method", "x-http-method-override", "x-method-override"]);
const FORBIDDEN_METHODS = new Set(["CONNECT", "TRACE", "TRACK"]);

function isForbiddenRequestHeader(name, value) {
  if (FORBIDDEN_REQUEST_HEADERS.has(name) || name.startsWith("proxy-") || name.startsWith("sec-")) return true;
  if (METHOD_OVERRIDE_HEADERS.has(name)) {
    for (const method of value.split(",")) if (FORBIDDEN_METHODS.has(method.trim().toUpperCase())) return true;
  }
  return false;
}

// Section 2.2.2, forbidden response-header name.
function isForbiddenResponseHeader(name) {
  return name === "set-cookie" || name === "set-cookie2";
}

const headersInternals = { list: null, guard: null, fill: null };

class Headers {
  // Each entry: the name as given, its lower case, and the value.
  #list = [];
  #guard = "none";
  constructor(init = undefined) {
    if (init !== undefined) Headers.#fill(this, init);
  }
  // Section 5.1, "fill".
  static #fill(headers, init) {
    if (init === null || (typeof init !== "object" && typeof init !== "function")) throw new TypeError("headers are a sequence of pairs or a record");
    if (init instanceof Headers) {
      for (const [name, , value] of init.#list.slice()) headers.#append(name, value);
      return;
    }
    const iterate = init[Symbol.iterator];
    if (iterate !== undefined && iterate !== null) {
      if (typeof iterate !== "function") throw new TypeError("the headers' iterator is not a function");
      for (const pair of init) {
        const items = pair === null || typeof pair !== "object" ? null : Array.from(pair);
        if (!items || items.length !== 2) throw new TypeError("each header is a name and a value");
        headers.#append(items[0], items[1]);
      }
      return;
    }
    for (const key of Reflect.ownKeys(init)) {
      if (typeof key !== "string") continue;
      const own = Reflect.getOwnPropertyDescriptor(init, key);
      if (own && own.enumerable) headers.#append(key, init[key]);
    }
  }
  // Section 5.1, "validate": whether the pair may be set under the guard.
  #validate(name, lower, value) {
    if (!TOKEN.test(name)) throw new TypeError("'" + name + "' is not a header name");
    if (/[\0\n\r]/.test(value)) throw new TypeError("a header's value holds no NUL, CR or LF");
    if (this.#guard === "immutable") throw new TypeError("these headers cannot be changed");
    if (this.#guard === "request" && isForbiddenRequestHeader(lower, value)) return false;
    if (this.#guard === "response" && isForbiddenResponseHeader(lower)) return false;
    return true;
  }
  #append(name, value) {
    name = byteString(name, "a header's name");
    value = normalizeValue(byteString(value, "a header's value"));
    const lower = name.toLowerCase();
    if (!this.#validate(name, lower, value)) return;
    this.#list.push([name, lower, value]);
  }
  append(name, value) {
    if (arguments.length < 2) throw new TypeError("append takes a name and a value");
    this.#append(name, value);
  }
  delete(name) {
    name = byteString(name, "a header's name");
    const lower = name.toLowerCase();
    if (!this.#validate(name, lower, "")) return;
    this.#list = this.#list.filter((entry) => entry[1] !== lower);
  }
  // Section 2.2.2, "get": the values joined by a comma and a space.
  get(name) {
    name = byteString(name, "a header's name");
    if (!TOKEN.test(name)) throw new TypeError("'" + name + "' is not a header name");
    const lower = name.toLowerCase();
    const values = this.#list.filter((entry) => entry[1] === lower).map((entry) => entry[2]);
    return values.length ? values.join(", ") : null;
  }
  getSetCookie() {
    return this.#list.filter((entry) => entry[1] === "set-cookie").map((entry) => entry[2]);
  }
  has(name) {
    name = byteString(name, "a header's name");
    if (!TOKEN.test(name)) throw new TypeError("'" + name + "' is not a header name");
    const lower = name.toLowerCase();
    return this.#list.some((entry) => entry[1] === lower);
  }
  set(name, value) {
    if (arguments.length < 2) throw new TypeError("set takes a name and a value");
    name = byteString(name, "a header's name");
    value = normalizeValue(byteString(value, "a header's value"));
    const lower = name.toLowerCase();
    if (!this.#validate(name, lower, value)) return;
    const first = this.#list.findIndex((entry) => entry[1] === lower);
    if (first < 0) {
      this.#list.push([name, lower, value]);
    } else {
      this.#list[first][2] = value;
      this.#list = this.#list.filter((entry, index) => index <= first || entry[1] !== lower);
    }
  }
  // Section 2.2.2, "sort and combine": names in lower case, sorted, the
  // values of one name joined, but each Set-Cookie its own.
  #sorted() {
    const names = Array.from(new Set(this.#list.map((entry) => entry[1]))).sort();
    const out = [];
    for (const lower of names) {
      const values = this.#list.filter((entry) => entry[1] === lower).map((entry) => entry[2]);
      if (lower === "set-cookie") for (const value of values) out.push([lower, value]);
      else out.push([lower, values.join(", ")]);
    }
    return out;
  }
  forEach(callback, thisArg = undefined) {
    if (typeof callback !== "function") throw new TypeError("forEach takes a function");
    for (const [name, value] of this.#sorted()) callback.call(thisArg, value, name, this);
  }
  *entries() { for (const pair of this.#sorted()) yield pair; }
  *keys() { for (const pair of this.#sorted()) yield pair[0]; }
  *values() { for (const pair of this.#sorted()) yield pair[1]; }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "Headers"; }
  static {
    headersInternals.list = (headers) => headers.#list;
    headersInternals.guard = (headers, guard) => { headers.#guard = guard; };
    headersInternals.fill = (headers, init) => Headers.#fill(headers, init);
  }
}

function headersWith(guard, init) {
  const headers = new Headers();
  headersInternals.guard(headers, guard);
  if (init !== undefined) headersInternals.fill(headers, init);
  return headers;
}

// Bodies (Fetch standard sections 2.2.4 and 5.3) -----------------------------------

// A body: `source` is its bytes when they are known whole (and then the
// stream is made only when it is asked for), `stream` its stream.
function makeBody(source, stream) {
  return { source, stream, used: false };
}

function bodyStream(body) {
  if (!body.stream) {
    const bytes = body.source;
    body.stream = new ReadableStream({
      type: "bytes",
      pull(controller) {
        if (bytes.byteLength > 0) controller.enqueue(bytes.slice());
        controller.close();
      },
    });
  }
  return body.stream;
}

// Section 5.3, "unusable": disturbed or locked.
function bodyUnusable(body) {
  if (body === null) return false;
  if (body.used) return true;
  return body.stream !== null && (isDisturbed(body.stream) || body.stream.locked);
}

function bodyUsed(body) {
  return body !== null && (body.used || (body.stream !== null && isDisturbed(body.stream)));
}

// Section 5.2, "extract a body": the body and the Content-Type it brings.
function extractBody(object, keepalive) {
  if (object instanceof ReadableStream) {
    if (keepalive) throw new TypeError("a keepalive request's body is not a stream");
    if (isDisturbed(object) || object.locked) throw new TypeError("the stream is disturbed or locked");
    return { body: makeBody(null, object), type: null };
  }
  if (object instanceof Blob) return { body: makeBody(blobInternals.bytes(object), null), type: object.type === "" ? null : object.type };
  if (object instanceof ArrayBuffer || ArrayBuffer.isView(object)) return { body: makeBody(viewOf(object, "a body").slice(), null), type: null };
  if (object instanceof FormData) {
    const encoded = multipartEncode(object);
    return { body: makeBody(encoded.bytes, null), type: encoded.type };
  }
  if (object instanceof URLSearchParams) return { body: makeBody(utf8Encode(object.toString()), null), type: "application/x-www-form-urlencoded;charset=UTF-8" };
  return { body: makeBody(utf8Encode(usv(object)), null), type: "text/plain;charset=UTF-8" };
}

// Every byte of a body (section 5.3, "fully read"), at most
// MAX_RESPONSE_BUFFER_BYTES of them (departure D8): past it the stream is
// cancelled and the promise rejects with a TypeError.
function readAll(body, bound, boundName) {
  if (body === null) return Promise.resolve(new Uint8Array(0));
  if (bodyUnusable(body)) return Promise.reject(new TypeError("the body has already been read"));
  if (body.stream === null) {
    body.used = true;
    return Promise.resolve(body.source);
  }
  const reader = body.stream.getReader();
  const chunks = [];
  let total = 0;
  const step = () => reader.read().then((result) => {
    if (result.done) return concatBytes(chunks, total);
    if (!(result.value instanceof Uint8Array)) throw new TypeError("a body's stream gives Uint8Array chunks");
    total += result.value.byteLength;
    if (total > bound) {
      const error = new TypeError(boundName + ": the body is too long to be read whole");
      reader.cancel(error).catch(() => {});
      throw error;
    }
    chunks.push(result.value);
    return step();
  });
  return step();
}

// The essence of a Content-Type header's value, in lower case.
function mimeEssence(type) {
  if (type === null) return "";
  const end = type.indexOf(";");
  return (end < 0 ? type : type.slice(0, end)).trim().toLowerCase();
}

// Section 5.3, the Body mixin's methods, for a class whose instances'
// bodies and headers `state` gives.
function bodyMixin(Class, state) {
  const all = (object) => readAll(state(object).body, WEB_BOUND.MAX_RESPONSE_BUFFER_BYTES, "MAX_RESPONSE_BUFFER_BYTES");
  const methods = {
    arrayBuffer() { return all(this).then((bytes) => bytes.slice().buffer); },
    bytes() { return all(this).then((bytes) => bytes.slice()); },
    blob() {
      const type = state(this).headers.get("content-type");
      return all(this).then((bytes) => blobInternals.make(bytes.slice(), type === null ? "" : blobType(type)));
    },
    text() { return all(this).then(utf8Decode); },
    json() { return all(this).then((bytes) => JSON.parse(utf8Decode(bytes))); },
    formData() {
      const type = state(this).headers.get("content-type");
      return all(this).then((bytes) => {
        const essence = mimeEssence(type);
        if (essence === "multipart/form-data") return multipartParse(bytes, type);
        if (essence === "application/x-www-form-urlencoded") {
          const form = new FormData();
          for (const [name, value] of formParse(utf8DecodeWithoutBom(bytes))) form.append(name, value);
          return form;
        }
        throw new TypeError("the body is not form data");
      });
    },
  };
  for (const name of Object.keys(methods)) {
    Object.defineProperty(Class.prototype, name, { value: methods[name], writable: true, configurable: true, enumerable: true });
  }
  Object.defineProperty(Class.prototype, "body", {
    get() { const body = state(this).body; return body === null ? null : bodyStream(body); }, configurable: true, enumerable: true,
  });
  Object.defineProperty(Class.prototype, "bodyUsed", { get() { return bodyUsed(state(this).body); }, configurable: true, enumerable: true });
}

// Section 2.2.4, "clone" a body: both get every byte.
function cloneBody(body) {
  if (body === null) return null;
  if (body.stream === null) return makeBody(body.source, null);
  const [left, right] = body.stream.tee();
  body.stream = left;
  return makeBody(null, right);
}

// Request (Fetch standard section 5.4) ----------------------------------------------

const REFERRER_POLICIES = ["", "no-referrer", "no-referrer-when-downgrade", "same-origin", "origin", "strict-origin", "origin-when-cross-origin", "strict-origin-when-cross-origin", "unsafe-url"];
const MODES = ["same-origin", "cors"];
const CREDENTIALS = ["omit", "same-origin", "include"];
const CACHE_MODES = ["default", "no-store", "reload", "no-cache", "force-cache", "only-if-cached"];
const REDIRECT_MODES = ["follow", "error", "manual"];
const NORMALIZED_METHODS = new Set(["DELETE", "GET", "HEAD", "OPTIONS", "POST", "PUT"]);
// The referrer that names the client (section 2.2.1): the script itself (departure D25).
const CLIENT = "about:client";

function oneOf(value, list, what) {
  value = String(value);
  if (list.indexOf(value) < 0) throw new TypeError("'" + value + "' is not a " + what);
  return value;
}

// A signal that follows `parent` (DOM section 3.2.2, dependent signals).
function followSignal(parent) {
  const signal = new AbortSignal(MAKE);
  if (parent) {
    if (parent.aborted) abortSignal(signal, parent.reason);
    else onAbort(parent, () => abortSignal(signal, parent.reason));
  }
  return signal;
}

const requestInternals = { state: null };

class Request {
  #state;
  constructor(input, init = undefined) {
    if (arguments.length < 1) throw new TypeError("a Request takes an address or a Request");
    if (init === undefined || init === null) init = {};
    if (typeof init !== "object" && typeof init !== "function") throw new TypeError("a request's options are an object");
    let state;
    let inputBody = null;
    let signal = null;
    if (input instanceof Request) {
      const from = input.#state;
      state = { ...from, headers: null, body: null };
      inputBody = from.body;
      signal = from.signal;
    } else {
      // Section 5.4 step 5: parsed against the script's own address.
      const record = parseUrl(input, SCRIPT_URL);
      if (!record) throw new TypeError("'" + input + "' is not a valid address");
      if (record.username !== "" || record.password !== "") throw new TypeError("a request's address holds no user name or password");
      state = {
        method: "GET", url: record.href, origin: record.origin, scheme: record.scheme,
        // The address without its fragment, as a response's `url` gives it.
        bare: record.bare,
        mode: "cors", credentials: "same-origin", cache: "default", redirect: "follow", referrer: CLIENT, referrerPolicy: "",
        integrity: "", keepalive: false, headers: null, body: null, signal: null,
      };
    }
    // Step 12: there is no window here.
    if (init.window !== undefined && init.window !== null) throw new TypeError("window is null");
    if (init.referrer !== undefined) {
      const referrer = usv(init.referrer);
      if (referrer === "") {
        state.referrer = "";
      } else {
        const parsed = parseUrl(referrer, SCRIPT_URL);
        if (!parsed) throw new TypeError("'" + referrer + "' is not a valid referrer");
        // Step 14.3: a referrer of another origin is the client.
        state.referrer = parsed.href === CLIENT || parsed.origin !== SCRIPT_ORIGIN ? CLIENT : parsed.href;
      }
    }
    if (init.referrerPolicy !== undefined) state.referrerPolicy = oneOf(init.referrerPolicy, REFERRER_POLICIES, "referrer policy");
    if (init.mode !== undefined) {
      const mode = String(init.mode);
      // Departure D5: an opaque response has nothing here to feed.
      if (mode === "no-cors") throw new TypeError("mode 'no-cors' is not provided (departure D5)");
      if (mode === "navigate" || mode === "websocket") throw new TypeError("a request cannot be made with mode '" + mode + "'");
      state.mode = oneOf(mode, MODES, "request mode");
    }
    if (init.credentials !== undefined) state.credentials = oneOf(init.credentials, CREDENTIALS, "credentials mode");
    if (init.cache !== undefined) state.cache = oneOf(init.cache, CACHE_MODES, "cache mode");
    if (state.cache === "only-if-cached" && state.mode !== "same-origin") throw new TypeError("'only-if-cached' needs mode 'same-origin'");
    if (init.redirect !== undefined) state.redirect = oneOf(init.redirect, REDIRECT_MODES, "redirect mode");
    if (init.integrity !== undefined) state.integrity = String(init.integrity);
    if (init.keepalive !== undefined) state.keepalive = !!init.keepalive;
    if (init.priority !== undefined) oneOf(init.priority, ["high", "low", "auto"], "priority");
    if (init.duplex !== undefined) oneOf(init.duplex, ["half"], "duplex");
    if (init.method !== undefined) {
      const method = byteString(init.method, "a method");
      if (!TOKEN.test(method)) throw new TypeError("'" + method + "' is not a method");
      const upper = method.toUpperCase();
      if (FORBIDDEN_METHODS.has(upper)) throw new TypeError("the method '" + method + "' is forbidden");
      state.method = NORMALIZED_METHODS.has(upper) ? upper : method;
    }
    if (init.signal !== undefined) {
      if (init.signal !== null && !(init.signal instanceof AbortSignal)) throw new TypeError("signal is an AbortSignal");
      signal = init.signal;
    }
    state.signal = followSignal(signal);
    state.headers = headersWith("request", init.headers !== undefined ? init.headers : input instanceof Request ? input.#state.headers : undefined);
    const initBody = init.body !== undefined && init.body !== null ? init.body : null;
    if ((initBody !== null || inputBody !== null) && (state.method === "GET" || state.method === "HEAD")) throw new TypeError("a GET or HEAD request has no body");
    if (initBody !== null) {
      const extracted = extractBody(initBody, state.keepalive);
      state.body = extracted.body;
      if (extracted.type !== null && !state.headers.has("content-type")) state.headers.append("Content-Type", extracted.type);
      // Step 38: a body that is a stream needs duplex.
      if (extracted.body.source === null && init.duplex === undefined) throw new TypeError("a request whose body is a stream needs duplex: 'half'");
    } else if (inputBody !== null) {
      if (bodyUnusable(inputBody)) throw new TypeError("the request's body has already been read");
      // Step 42: the new request takes the body, and the old one's is used.
      state.body = makeBody(inputBody.source, inputBody.stream);
      inputBody.used = true;
      if (inputBody.stream !== null) {
        const through = new TransformStream();
        inputBody.stream.pipeTo(through.writable).catch(() => {});
        state.body = makeBody(null, through.readable);
      }
    }
    this.#state = state;
  }
  get method() { return this.#state.method; }
  get url() { return this.#state.url; }
  get headers() { return this.#state.headers; }
  // There is no document for a destination to name (departure D25).
  get destination() { return ""; }
  get referrer() { return this.#state.referrer; }
  get referrerPolicy() { return this.#state.referrerPolicy; }
  get mode() { return this.#state.mode; }
  get credentials() { return this.#state.credentials; }
  get cache() { return this.#state.cache; }
  get redirect() { return this.#state.redirect; }
  get integrity() { return this.#state.integrity; }
  get keepalive() { return this.#state.keepalive; }
  get isReloadNavigation() { return false; }
  get isHistoryNavigation() { return false; }
  get signal() { return this.#state.signal; }
  get duplex() { return "half"; }
  clone() {
    if (bodyUnusable(this.#state.body)) throw new TypeError("the request's body has already been read");
    const copy = new Request(this.#state.url);
    copy.#state = { ...this.#state, headers: headersWith("request", this.#state.headers), body: cloneBody(this.#state.body), signal: followSignal(this.#state.signal) };
    return copy;
  }
  get [Symbol.toStringTag]() { return "Request"; }
  static {
    requestInternals.state = (request) => request.#state;
  }
}
const SCRIPT_ORIGIN = (parseUrl(SCRIPT_URL, undefined) || { origin: "null" }).origin;
bodyMixin(Request, requestInternals.state);

// Response (Fetch standard section 5.5) ----------------------------------------------

// Section 2.2.3: a response with one of these has no body.
const NULL_BODY_STATUSES = new Set([101, 103, 204, 205, 304]);
const REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);
// RFC 9112 section 4, reason-phrase.
const REASON_PHRASE = /^[\t\x20-\x7e\x80-\xff]*$/;

const responseInternals = { state: null, make: null };

// Section 5.5, "initialize a response".
function initResponse(state, init, extracted) {
  if (init === undefined || init === null) init = {};
  // Web IDL's unsigned short, then the standard's range.
  const status = init.status === undefined ? 200 : Number(init.status) & 0xffff;
  if (status < 200 || status > 599) throw new RangeError("a status is 200 to 599");
  const statusText = init.statusText === undefined ? "" : byteString(init.statusText, "a status text");
  if (!REASON_PHRASE.test(statusText)) throw new TypeError("the status text is not a reason phrase");
  state.status = status;
  state.statusText = statusText;
  if (init.headers !== undefined) headersInternals.fill(state.headers, init.headers);
  if (extracted) {
    if (NULL_BODY_STATUSES.has(status)) throw new TypeError("a response with status " + status + " has no body");
    state.body = extracted.body;
    if (extracted.type !== null && !state.headers.has("content-type")) state.headers.append("Content-Type", extracted.type);
  }
}

class Response {
  #state;
  constructor(body = null, init = undefined) {
    const state = { type: "default", url: "", redirected: false, status: 200, statusText: "", headers: headersWith("response", undefined), body: null };
    initResponse(state, init, body === null || body === undefined ? null : extractBody(body, false));
    this.#state = state;
  }
  static error() {
    return responseInternals.make({ type: "error", url: "", redirected: false, status: 0, statusText: "", headers: headersWith("immutable", undefined), body: null });
  }
  static redirect(url, status = 302) {
    const record = parseUrl(url, SCRIPT_URL);
    if (!record) throw new TypeError("'" + url + "' is not a valid address");
    status = Number(status) & 0xffff;
    if (!REDIRECT_STATUSES.has(status)) throw new RangeError("a redirect's status is 301, 302, 303, 307 or 308");
    const headers = headersWith("none", [["Location", record.href]]);
    headersInternals.guard(headers, "immutable");
    return responseInternals.make({ type: "default", url: "", redirected: false, status, statusText: "", headers, body: null });
  }
  static json(data, init = undefined) {
    const text = JSON.stringify(data);
    if (text === undefined) throw new TypeError("the value has no JSON text");
    const state = { type: "default", url: "", redirected: false, status: 200, statusText: "", headers: headersWith("response", undefined), body: null };
    initResponse(state, init, { body: makeBody(utf8Encode(text), null), type: "application/json" });
    return responseInternals.make(state);
  }
  get type() { return this.#state.type; }
  get url() { return this.#state.url; }
  get redirected() { return this.#state.redirected; }
  get status() { return this.#state.status; }
  get ok() { return this.#state.status >= 200 && this.#state.status <= 299; }
  get statusText() { return this.#state.statusText; }
  get headers() { return this.#state.headers; }
  clone() {
    if (bodyUnusable(this.#state.body)) throw new TypeError("the response's body has already been read");
    const headers = headersWith("none", this.#state.headers);
    headersInternals.guard(headers, this.#state.type === "default" ? "response" : "immutable");
    return responseInternals.make({ ...this.#state, headers, body: cloneBody(this.#state.body) });
  }
  get [Symbol.toStringTag]() { return "Response"; }
  static {
    responseInternals.state = (response) => response.#state;
    responseInternals.make = (state) => {
      const response = new Response();
      response.#state = state;
      return response;
    };
  }
}
bodyMixin(Response, responseInternals.state);

// fetch (Fetch standard sections 4 and 5.6) --------------------------------------------

// The network's objects of this context by the host's number for each:
// a fetch under way, an event stream, a socket.
const network = new Map();

// The error a failed request gives its script: the standard's and no
// more (section 9.2). Why is in the developer log, never here.
function networkError(kind) {
  if (kind === WEB_BOUND.ERROR_ABORT) return domException("the request was aborted", "AbortError");
  if (kind === WEB_BOUND.ERROR_TIMEOUT) return domException("the request timed out", "TimeoutError");
  return new TypeError("Failed to fetch");
}

// Section 4.2.1 of the Fetch standard's "data: URL processor": the
// response for a `data:` address (departure D10: fetched here), or null.
function dataResponse(url) {
  const comma = url.indexOf(",");
  if (comma < 0) return null;
  let type = url.slice(5, comma).replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "");
  const encoded = url.slice(comma + 1);
  // Percent-decode: the bytes of the text with each %XX made its byte.
  const raw = utf8Encode(encoded);
  let bytes = new Uint8Array(raw.byteLength);
  let length = 0;
  const hex = (byte) => (byte >= 0x30 && byte <= 0x39 ? byte - 0x30 : byte >= 0x41 && byte <= 0x46 ? byte - 0x37 : byte >= 0x61 && byte <= 0x66 ? byte - 0x57 : -1);
  for (let index = 0; index < raw.byteLength; index++) {
    if (raw[index] === 0x25 && index + 2 < raw.byteLength && hex(raw[index + 1]) >= 0 && hex(raw[index + 2]) >= 0) {
      bytes[length++] = hex(raw[index + 1]) * 16 + hex(raw[index + 2]);
      index += 2;
    } else {
      bytes[length++] = raw[index];
    }
  }
  bytes = bytes.subarray(0, length);
  const base64 = /;[ ]*base64$/i.exec(type);
  if (base64) {
    // Forgiving-base64 decode (Infra section 7): ASCII white space is
    // dropped, and what is not base64 is a failure.
    let text = "";
    for (let index = 0; index < bytes.byteLength; index++) text += String.fromCharCode(bytes[index]);
    let decoded;
    try {
      decoded = atob(text.replace(/[\t\n\f\r ]/g, ""));
    } catch (error) {
      return null;
    }
    bytes = new Uint8Array(decoded.length);
    for (let index = 0; index < decoded.length; index++) bytes[index] = decoded.charCodeAt(index);
    type = type.slice(0, base64.index).replace(/[ ]+$/, "");
  }
  if (type.startsWith(";")) type = "text/plain" + type;
  if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+\/[!#$%&'*+\-.^_`|~0-9A-Za-z]+[\t\n\r ]*(;.*)?$/.test(type)) type = "text/plain;charset=US-ASCII";
  const headers = headersWith("none", [["Content-Type", type]]);
  headersInternals.guard(headers, "immutable");
  return responseInternals.make({ type: "basic", url, redirected: false, status: 200, statusText: "OK", headers, body: makeBody(bytes, null) });
}

function fetchFlags(state, kind, hasBody) {
  return (kind << FETCH_FLAG.KIND)
    | (MODES.indexOf(state.mode) << FETCH_FLAG.MODE)
    | (CREDENTIALS.indexOf(state.credentials) << FETCH_FLAG.CREDENTIALS)
    | (CACHE_MODES.indexOf(state.cache) << FETCH_FLAG.CACHE)
    | (REDIRECT_MODES.indexOf(state.redirect) << FETCH_FLAG.REDIRECT)
    | (REFERRER_POLICIES.indexOf(state.referrerPolicy) << FETCH_FLAG.POLICY)
    | ((state.keepalive ? 1 : 0) << FETCH_FLAG.KEEPALIVE)
    | ((hasBody ? 1 : 0) << FETCH_FLAG.BODY);
}

// A request's head as operation FETCH takes it: the method, the address,
// the referrer, the integrity metadata and the last event ID, a NUL
// after each, then a line for each header.
function fetchHead(state, lastEventId) {
  if (state.integrity.indexOf("\0") >= 0) throw new TypeError("integrity metadata holds no NUL");
  let head = state.method + "\0" + state.url + "\0" + state.referrer + "\0" + state.integrity + "\0" + lastEventId + "\0";
  for (const [name, , value] of headersInternals.list(state.headers)) head += name + ":" + value + "\n";
  return head;
}

// A response's head as event FETCH_HEAD gives it.
function responseOf(pending, status, flags, payload) {
  const [url, statusText, lines] = payload.split("\0");
  const headers = headersWith("none", undefined);
  for (const line of lines.split("\n")) {
    if (line === "") continue;
    const colon = line.indexOf(":");
    // The web client has filtered them (section 11.3); a name or value
    // that is not one is dropped and the rest stand.
    try {
      headers.append(line.slice(0, colon), line.slice(colon + 1));
    } catch (error) { /* not a header */ }
  }
  headersInternals.guard(headers, "immutable");
  const kind = flags & 3;
  const type = kind === WEB_BOUND.RESPONSE_CORS ? "cors" : kind === WEB_BOUND.RESPONSE_OPAQUE_REDIRECT ? "opaqueredirect" : "basic";
  let body = null;
  // Section 4.1 "main fetch" step 21 and section 2.2.3: no body for HEAD
  // or for a null body status; none can be read of a redirect not followed.
  if (type !== "opaqueredirect" && pending.method !== "HEAD" && !NULL_BODY_STATUSES.has(status)) {
    const stream = new ReadableStream({
      type: "bytes",
      start(controller) { pending.controller = controller; },
      pull() { giveCredit(pending); },
      cancel() { finishFetch(pending, true); },
    }, { highWaterMark: WEB_BOUND.FETCH_WINDOW_BYTES });
    body = makeBody(null, stream);
  }
  // A redirect not followed is answered at the address that was asked.
  return responseInternals.make({ type, url: type === "opaqueredirect" && url === "" ? pending.url : url, redirected: !!(flags & WEB_BOUND.HEAD_REDIRECTED), status, statusText, headers, body });
}

// Makes room at the web client for as much as the script has read: the
// room is what the stream still wants (section 11.3, `More`), given in
// pieces of half a window or more so that a slow reader costs few messages.
function giveCredit(pending) {
  if (!pending.controller || pending.done) return;
  const wanted = pending.received + Math.max(pending.controller.desiredSize || 0, 0) - pending.credited;
  if (wanted >= WEB_BOUND.FETCH_WINDOW_BYTES / 2) {
    host(OP.FETCH_CREDIT, pending.id, wanted);
    pending.credited += wanted;
  }
}

// Ends a fetch here; when the script gave it up, the host is told.
function finishFetch(pending, abort) {
  if (pending.done) return;
  pending.done = true;
  network.delete(pending.id);
  if (abort) host(OP.FETCH_ABORT, pending.id);
}

// Sends as much of a request's body as the host takes now; the host says
// with event FETCH_SENT when to go on.
function pumpBody(pending) {
  const body = pending.upload;
  while (!pending.done && pending.uploaded < body.byteLength) {
    const to = Math.min(body.byteLength, pending.uploaded + WEB_BOUND.MAX_HOST_BYTES);
    const last = to === body.byteLength ? 1 : 0;
    if (host(OP.FETCH_BODY, pending.id, last, body.subarray(pending.uploaded, to)) !== 1) return;
    pending.uploaded = to;
  }
  pending.upload = null;
}

function startFetch(state, bytes, resolve, reject) {
  const signal = state.signal;
  if (signal.aborted) { reject(signal.reason); return; }
  let id;
  try {
    id = host(OP.FETCH, fetchFlags(state, 0, bytes !== null), bytes === null ? 0 : bytes.byteLength, fetchHead(state, ""));
  } catch (error) {
    // Past a bound, or without the manifest's `network`: the standard's
    // TypeError, whose message names the bound (departure D8).
    reject(new TypeError(String(error && error.message)));
    return;
  }
  const pending = {
    kind: "fetch", id: Math.abs(id), method: state.method, url: state.bare, resolve, reject, controller: null, response: null, done: false,
    received: 0, credited: WEB_BOUND.FETCH_WINDOW_BYTES, upload: bytes !== null && bytes.byteLength > 0 ? bytes : null, uploaded: 0,
  };
  network.set(pending.id, pending);
  // Section 4.2 "fetch" step 11 and DOM section 3.2: an abort rejects the
  // promise, errors the response's body and ends the request.
  onAbort(signal, () => {
    if (pending.done) return;
    finishFetch(pending, true);
    reject(signal.reason);
    if (pending.controller) { try { pending.controller.error(signal.reason); } catch (error) { /* already closed */ } }
  });
  if (pending.upload && id > 0) pumpBody(pending);
}

function fetch(input, init = undefined) {
  return new Promise((resolve, reject) => {
    const request = new Request(input, init);
    const state = requestInternals.state(request);
    if (state.signal.aborted) { reject(state.signal.reason); return; }
    const scheme = state.scheme;
    if (scheme === "data") {
      const response = dataResponse(state.bare);
      if (response) resolve(response); else reject(new TypeError("Failed to fetch"));
      return;
    }
    // Departure D10: no object URLs exist, and `about:` names nothing.
    if (scheme !== "http" && scheme !== "https") { reject(new TypeError("Failed to fetch")); return; }
    const body = state.body;
    if (body === null) { startFetch(state, null, resolve, reject); return; }
    if (bodyUnusable(body)) { reject(new TypeError("the request's body has already been read")); return; }
    // Departure D7: a stream is read to its end and sent with its length.
    readAll(body, WEB_BOUND.MAX_REQUEST_BODY_BYTES, "MAX_REQUEST_BODY_BYTES").then((bytes) => startFetch(state, bytes, resolve, reject), reject);
  });
}

// Events 30 to 34 for a fetch.
function fetchEvent(pending, kind, a, b, c, payload) {
  switch (kind) {
    case EV.FETCH_HEAD: {
      const response = responseOf(pending, b, c, payload);
      pending.response = response;
      pending.resolve(response);
      break;
    }
    case EV.FETCH_CHUNK:
      pending.received += payload.byteLength;
      if (pending.controller) {
        pending.controller.enqueue(payload);
      } else {
        // A body the response does not have (HEAD, a null body status)
        // is read past, so that the request can end.
        pending.credited += payload.byteLength;
        host(OP.FETCH_CREDIT, pending.id, payload.byteLength);
      }
      break;
    case EV.FETCH_END:
      finishFetch(pending, false);
      if (pending.controller) {
        pending.controller.close();
        // A reader waiting with its own buffer is answered (Streams section 4.7.4).
        if (pending.controller.byobRequest) pending.controller.byobRequest.respond(0);
      }
      break;
    case EV.FETCH_FAILED: {
      finishFetch(pending, false);
      const error = networkError(a);
      if (pending.response) {
        if (pending.controller) pending.controller.error(error);
      } else {
        pending.reject(error);
      }
      break;
    }
    case EV.FETCH_SENT:
      if (pending.upload) pumpBody(pending);
      break;
  }
}

define("fetch", fetch);
for (const [name, value] of Object.entries({ Blob, File, FormData, Headers, Request, Response })) define(name, value);
