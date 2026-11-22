// The web APIs' first part (work package W2; script-api.md section 9.1):
// the Encoding standard's TextEncoder, TextDecoder and their streams, the
// URL standard's URL and URLSearchParams, and the Streams standard's
// classes from the bundled ponyfill (WebStreams, written in front of this
// by the host). It is part of the prelude's one function, after the
// shared core, whose names (host, OP, define, domException, core) it
// uses, with the tables DECODE_FLAG, URL_PART and WEB_BOUND written in
// front of it by the host (host/web.rs).
//
// Like all of the prelude it is not trusted: every check that matters is
// the host's or the web client's. What is here follows each standard's
// algorithm, cited by section, so that a script sees the standard's API.

// A string as Web IDL's USVString (Web IDL section 3.2.11): every lone
// surrogate becomes U+FFFD.
function usv(value) {
  return String(value).toWellFormed();
}

// Bytes of a BufferSource, viewed and not copied (Web IDL section 3.2.26).
function viewOf(source, what) {
  if (source instanceof ArrayBuffer) return new Uint8Array(source);
  if (ArrayBuffer.isView(source)) return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
  throw new TypeError(what + " takes an ArrayBuffer or a view of one");
}

function concatBytes(chunks, total) {
  if (chunks.length === 1 && chunks[0].byteLength === total) return chunks[0];
  const out = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    out.set(chunk, at);
    at += chunk.byteLength;
  }
  return out;
}

// Streams (Streams standard) --------------------------------------------------

const {
  ReadableStream, ReadableStreamDefaultReader, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableByteStreamController,
  ReadableStreamDefaultController, WritableStream, WritableStreamDefaultWriter, WritableStreamDefaultController, TransformStream,
  TransformStreamDefaultController, ByteLengthQueuingStrategy, CountQueuingStrategy,
} = WebStreams;

// Whether a stream has been read from or cancelled (Streams standard
// section 4.9.6, "disturbed"), which the ponyfill keeps under this name
// and the Fetch standard's body needs. The ponyfill is pinned to one
// version, and the crate's tests hold this to it.
function isDisturbed(stream) {
  return stream._disturbed === true;
}

// Encoding (Encoding standard) -------------------------------------------------

// The UTF-16 code units of a string given to the host at a time: at
// UTF-8's three bytes a unit at most, within a host call's payload.
const ENCODE_UNITS = Math.floor(WEB_BOUND.MAX_HOST_BYTES / 3);

// UTF-8 encode (Encoding standard section 4.3), by the host.
function utf8Encode(text) {
  if (text.length <= ENCODE_UNITS) return host(OP.DECODE, 0, DECODE_FLAG.ENCODE, 0, text);
  const chunks = [];
  let total = 0;
  for (let from = 0; from < text.length;) {
    let to = Math.min(text.length, from + ENCODE_UNITS);
    // Never between the two halves of a surrogate pair.
    const last = text.charCodeAt(to - 1);
    if (to < text.length && last >= 0xd800 && last <= 0xdbff) to--;
    const chunk = host(OP.DECODE, 0, DECODE_FLAG.ENCODE, 0, text.slice(from, to));
    chunks.push(chunk);
    total += chunk.byteLength;
    from = to;
  }
  return concatBytes(chunks, total);
}

// Where to cut UTF-8 bytes so that no character is cut: at `end`, or
// before the lead byte of a character that `end` would cut. A lead byte
// is looked for in the three bytes before `end` only: a longer run of
// continuation bytes is malformed wherever it is cut.
function utf8Cut(bytes, end) {
  for (let back = 1; back <= 3 && back <= end; back++) {
    const byte = bytes[end - back];
    if (byte < 0x80) return end;
    if (byte >= 0xc0) {
      const needed = byte >= 0xf0 ? 4 : byte >= 0xe0 ? 3 : 2;
      return needed > back ? end - back : end;
    }
  }
  return end;
}

// How many bytes at the end of `bytes` begin a UTF-8 character that the
// next piece of a stream may complete (Encoding standard section 8.1.1,
// the UTF-8 decoder's lower and upper boundaries): those are held for it.
function utf8Unfinished(bytes) {
  const end = bytes.byteLength;
  for (let back = 1; back <= 3 && back <= end; back++) {
    const lead = bytes[end - back];
    if (lead < 0x80) return 0;
    if (lead < 0xc0) continue;
    if (lead < 0xc2 || lead > 0xf4) return 0;
    const needed = lead >= 0xf0 ? 4 : lead >= 0xe0 ? 3 : 2;
    if (needed <= back) return 0;
    if (back >= 2) {
      const second = bytes[end - back + 1];
      const low = lead === 0xe0 ? 0xa0 : lead === 0xf0 ? 0x90 : 0x80;
      const high = lead === 0xed ? 0x9f : lead === 0xf4 ? 0x8f : 0xbf;
      if (second < low || second > high) return 0;
    }
    return back;
  }
  return 0;
}

// Decodes whole UTF-8 by the host, a payload at a time, cut between
// characters. `flags` holds FATAL and IGNORE_BOM; a byte order mark is
// looked for at the start of the first piece only.
function utf8DecodeWhole(bytes, flags) {
  const most = WEB_BOUND.MAX_HOST_BYTES;
  if (bytes.byteLength <= most) return host(OP.DECODE, 0, flags, 0, bytes);
  let text = "";
  for (let from = 0; from < bytes.byteLength;) {
    let to = Math.min(bytes.byteLength, from + most);
    if (to < bytes.byteLength) to = utf8Cut(bytes, to);
    text += host(OP.DECODE, 0, from === 0 ? flags : flags | DECODE_FLAG.IGNORE_BOM, 0, bytes.subarray(from, to));
    from = to;
  }
  return text;
}

// "UTF-8 decode" (Encoding standard section 6): a byte order mark is
// removed, errors are U+FFFD.
function utf8Decode(bytes) {
  return utf8DecodeWhole(bytes, 0);
}
// "UTF-8 decode without BOM" (section 6).
function utf8DecodeWithoutBom(bytes) {
  return utf8DecodeWhole(bytes, DECODE_FLAG.IGNORE_BOM);
}

class TextEncoder {
  get encoding() { return "utf-8"; }
  encode(input = "") { return utf8Encode(usv(input)); }
  // Encoding standard section 7.2, encodeInto: as many whole characters
  // as fit.
  encodeInto(source, destination) {
    if (!(destination instanceof Uint8Array)) throw new TypeError("encodeInto writes to a Uint8Array");
    const text = usv(source);
    let read = 0;
    let written = 0;
    const room = destination.byteLength;
    while (read < text.length) {
      const point = text.codePointAt(read);
      const bytes = point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      if (written + bytes > room) break;
      written += bytes;
      read += point >= 0x10000 ? 2 : 1;
    }
    if (written > 0) destination.set(utf8Encode(text.slice(0, read)));
    return { read, written };
  }
}

class TextDecoder {
  #index;
  #name;
  #fatal;
  #ignoreBOM;
  // UTF-8: the bytes of an unfinished character, and whether the stream
  // has begun (so that a byte order mark is looked for once).
  #held = null;
  #begun = false;
  // Other encodings: the host's decoder for a stream under way.
  #decoder = 0;
  constructor(label = "utf-8", options = undefined) {
    // Section 7.1: a label that names no encoding, or `replacement`, is a RangeError.
    const found = host(OP.DECODE, 0, DECODE_FLAG.LABEL, 0, String(label));
    if (found === undefined) throw new RangeError("'" + label + "' is not a label of an encoding");
    const [index, name] = found.split("\n");
    this.#index = Number(index);
    this.#name = name;
    this.#fatal = !!(options && options.fatal);
    this.#ignoreBOM = !!(options && options.ignoreBOM);
  }
  get encoding() { return this.#name; }
  get fatal() { return this.#fatal; }
  get ignoreBOM() { return this.#ignoreBOM; }
  decode(input = undefined, options = undefined) {
    let bytes = input === undefined ? new Uint8Array(0) : viewOf(input, "decode");
    const stream = !!(options && options.stream);
    const flags = (this.#fatal ? DECODE_FLAG.FATAL : 0) | (this.#ignoreBOM ? DECODE_FLAG.IGNORE_BOM : 0);
    try {
      if (this.#index === 0) return this.#utf8(bytes, stream, flags);
      return this.#other(bytes, stream, flags);
    } catch (error) {
      // Section 7.1: an error ends the decoding; the next begins anew.
      this.#held = null;
      this.#begun = false;
      this.#decoder = 0;
      throw error;
    }
  }
  #utf8(bytes, stream, flags) {
    if (this.#held) {
      const joined = new Uint8Array(this.#held.byteLength + bytes.byteLength);
      joined.set(this.#held, 0);
      joined.set(bytes, this.#held.byteLength);
      bytes = joined;
      this.#held = null;
    }
    if (stream) {
      const unfinished = utf8Unfinished(bytes);
      if (unfinished > 0) {
        this.#held = bytes.slice(bytes.byteLength - unfinished);
        bytes = bytes.subarray(0, bytes.byteLength - unfinished);
      }
    }
    const begun = this.#begun;
    if (stream) {
      if (bytes.byteLength > 0) this.#begun = true;
    } else {
      this.#begun = false;
    }
    if (bytes.byteLength === 0) return "";
    return utf8DecodeWhole(bytes, begun ? flags | DECODE_FLAG.IGNORE_BOM : flags);
  }
  #other(bytes, stream, flags) {
    const most = WEB_BOUND.MAX_HOST_BYTES;
    // A whole decoding that fits one call keeps nothing at the host.
    if (!stream && this.#decoder === 0 && bytes.byteLength <= most) return host(OP.DECODE, this.#index, flags, 0, bytes);
    if (this.#decoder === 0) this.#decoder = host(OP.DECODE, this.#index, flags | DECODE_FLAG.OPEN, 0);
    const decoder = this.#decoder;
    let text = "";
    let from = 0;
    do {
      const to = Math.min(bytes.byteLength, from + most);
      const more = stream || to < bytes.byteLength;
      // The host frees the decoder at the stream's end.
      if (!more) this.#decoder = 0;
      text += host(OP.DECODE, this.#index, more ? DECODE_FLAG.STREAM : 0, decoder, bytes.subarray(from, to));
      from = to;
    } while (from < bytes.byteLength);
    return text;
  }
}

// Encoding standard section 7.4 and 7.5: the two as transform streams.
class TextDecoderStream {
  #decoder;
  #transform;
  constructor(label = "utf-8", options = undefined) {
    const decoder = new TextDecoder(label, options);
    this.#decoder = decoder;
    this.#transform = new TransformStream({
      transform(chunk, controller) {
        const text = decoder.decode(viewOf(chunk, "a TextDecoderStream"), { stream: true });
        if (text !== "") controller.enqueue(text);
      },
      flush(controller) {
        const text = decoder.decode();
        if (text !== "") controller.enqueue(text);
      },
    });
  }
  get encoding() { return this.#decoder.encoding; }
  get fatal() { return this.#decoder.fatal; }
  get ignoreBOM() { return this.#decoder.ignoreBOM; }
  get readable() { return this.#transform.readable; }
  get writable() { return this.#transform.writable; }
}

class TextEncoderStream {
  #transform;
  constructor() {
    // A high surrogate at a chunk's end waits for its low half (section 7.5).
    let pending = "";
    this.#transform = new TransformStream({
      transform(chunk, controller) {
        let text = pending + String(chunk);
        pending = "";
        const last = text.charCodeAt(text.length - 1);
        if (last >= 0xd800 && last <= 0xdbff) {
          pending = text.slice(-1);
          text = text.slice(0, -1);
        }
        if (text !== "") controller.enqueue(utf8Encode(usv(text)));
      },
      flush(controller) {
        if (pending !== "") controller.enqueue(utf8Encode(usv(pending)));
      },
    });
  }
  get encoding() { return "utf-8"; }
  get readable() { return this.#transform.readable; }
  get writable() { return this.#transform.writable; }
}

// URL (URL standard) -----------------------------------------------------------

const SCRIPT_URL = host(OP.INFO, INFO.SCRIPT_URL);

function utf8Bytes(text) {
  let bytes = 0;
  for (let index = 0; index < text.length; index++) {
    const unit = text.charCodeAt(index);
    if (unit < 0x80) bytes += 1;
    else if (unit < 0x800) bytes += 2;
    else if (unit >= 0xd800 && unit <= 0xdbff) { bytes += 4; index++; }
    else bytes += 3;
  }
  return bytes;
}

// What the host says of an address: its serialisation, its origin, and
// what the API gives for each of its components, a line each.
const URL_FIELD_NAMES = ["href", "origin", "protocol", "username", "password", "host", "hostname", "port", "pathname", "search", "hash"];
function urlRecord(reply) {
  if (reply === undefined) return null;
  const fields = reply.split("\n");
  const record = {};
  for (let index = 0; index < WEB_BOUND.URL_FIELDS; index++) record[URL_FIELD_NAMES[index]] = fields[index] || "";
  // The address without its fragment: a `#` in a serialised address is
  // its fragment's and nothing else's.
  const cut = record.href.indexOf("#");
  record.bare = cut < 0 ? record.href : record.href.slice(0, cut);
  record.scheme = record.protocol.slice(0, -1);
  return record;
}

// The basic URL parser (URL standard section 4.4) on `input` against the
// serialised `base`, by the host: its record, or null for failure.
function parseUrl(input, base) {
  input = usv(input);
  if (base === undefined) return urlRecord(host(OP.URL_PARSE, -1, input));
  return urlRecord(host(OP.URL_PARSE, utf8Bytes(input), input + base));
}

const urlInternals = { record: null, setQuery: null };

// application/x-www-form-urlencoded (URL standard section 5).
function formPercentDecode(text) {
  if (text.indexOf("%") < 0 && !/[^\x00-\x7f]/.test(text)) return text;
  const bytes = utf8Encode(text);
  const out = new Uint8Array(bytes.byteLength);
  let length = 0;
  const hex = (byte) => (byte >= 0x30 && byte <= 0x39 ? byte - 0x30 : byte >= 0x41 && byte <= 0x46 ? byte - 0x37 : byte >= 0x61 && byte <= 0x66 ? byte - 0x57 : -1);
  for (let index = 0; index < bytes.byteLength; index++) {
    const byte = bytes[index];
    if (byte === 0x25 && index + 2 < bytes.byteLength && hex(bytes[index + 1]) >= 0 && hex(bytes[index + 2]) >= 0) {
      out[length++] = hex(bytes[index + 1]) * 16 + hex(bytes[index + 2]);
      index += 2;
    } else {
      out[length++] = byte;
    }
  }
  return utf8DecodeWithoutBom(out.subarray(0, length));
}

function formParse(text) {
  const list = [];
  for (const part of text.split("&")) {
    if (part === "") continue;
    const at = part.indexOf("=");
    const name = at < 0 ? part : part.slice(0, at);
    const value = at < 0 ? "" : part.slice(at + 1);
    list.push([formPercentDecode(name.replaceAll("+", " ")), formPercentDecode(value.replaceAll("+", " "))]);
  }
  return list;
}

// The application/x-www-form-urlencoded percent-encode set leaves only
// ASCII alphanumerics and * - . _ (section 5.2); a space is a plus.
function formEncode(text) {
  return encodeURIComponent(text).replace(/%20|[!'()~]/g, (found) => (found === "%20" ? "+" : "%" + found.charCodeAt(0).toString(16).toUpperCase()));
}

function formSerialize(list) {
  let out = "";
  for (const [name, value] of list) out += (out === "" ? "" : "&") + formEncode(name) + "=" + formEncode(value);
  return out;
}

const paramsInternals = { list: null, attach: null, replace: null };

class URLSearchParams {
  #list = [];
  #url = null;
  constructor(init = "") {
    if (init === undefined || init === null) return;
    if (typeof init === "object" || typeof init === "function") {
      const iterate = init[Symbol.iterator];
      if (iterate !== undefined && iterate !== null) {
        if (typeof iterate !== "function") throw new TypeError("the initialiser's iterator is not a function");
        for (const pair of init) {
          const items = pair === null || typeof pair !== "object" ? null : Array.from(pair);
          if (!items || items.length !== 2) throw new TypeError("each initialiser pair has a name and a value");
          this.#list.push([usv(items[0]), usv(items[1])]);
        }
      } else {
        // A record: later keys that are the same string replace earlier ones.
        const seen = new Map();
        for (const key of Reflect.ownKeys(init)) {
          if (typeof key !== "string") continue;
          const own = Reflect.getOwnPropertyDescriptor(init, key);
          if (!own || !own.enumerable) continue;
          const name = usv(key);
          const value = usv(init[key]);
          if (seen.has(name)) this.#list[seen.get(name)][1] = value;
          else { seen.set(name, this.#list.length); this.#list.push([name, value]); }
        }
      }
      return;
    }
    let text = usv(init);
    if (text.startsWith("?")) text = text.slice(1);
    this.#list = formParse(text);
  }
  // Section 5.2, "update": the object's URL takes the serialisation as its query.
  #update() {
    if (!this.#url) return;
    const text = formSerialize(this.#list);
    urlInternals.setQuery(this.#url, text);
  }
  get size() { return this.#list.length; }
  append(name, value) {
    if (arguments.length < 2) throw new TypeError("append takes a name and a value");
    this.#list.push([usv(name), usv(value)]);
    this.#update();
  }
  delete(name, value = undefined) {
    if (arguments.length < 1) throw new TypeError("delete takes a name");
    name = usv(name);
    if (value !== undefined) {
      value = usv(value);
      this.#list = this.#list.filter((pair) => !(pair[0] === name && pair[1] === value));
    } else {
      this.#list = this.#list.filter((pair) => pair[0] !== name);
    }
    this.#update();
  }
  get(name) {
    if (arguments.length < 1) throw new TypeError("get takes a name");
    name = usv(name);
    const found = this.#list.find((pair) => pair[0] === name);
    return found ? found[1] : null;
  }
  getAll(name) {
    if (arguments.length < 1) throw new TypeError("getAll takes a name");
    name = usv(name);
    return this.#list.filter((pair) => pair[0] === name).map((pair) => pair[1]);
  }
  has(name, value = undefined) {
    if (arguments.length < 1) throw new TypeError("has takes a name");
    name = usv(name);
    if (value !== undefined) {
      value = usv(value);
      return this.#list.some((pair) => pair[0] === name && pair[1] === value);
    }
    return this.#list.some((pair) => pair[0] === name);
  }
  set(name, value) {
    if (arguments.length < 2) throw new TypeError("set takes a name and a value");
    name = usv(name);
    value = usv(value);
    const first = this.#list.findIndex((pair) => pair[0] === name);
    if (first < 0) {
      this.#list.push([name, value]);
    } else {
      this.#list[first][1] = value;
      this.#list = this.#list.filter((pair, index) => index <= first || pair[0] !== name);
    }
    this.#update();
  }
  // By code units, keeping the order of equal names (section 5.2, sort).
  sort() {
    this.#list.sort((left, right) => (left[0] < right[0] ? -1 : left[0] > right[0] ? 1 : 0));
    this.#update();
  }
  forEach(callback, thisArg = undefined) {
    if (typeof callback !== "function") throw new TypeError("forEach takes a function");
    for (let index = 0; index < this.#list.length; index++) {
      const pair = this.#list[index];
      callback.call(thisArg, pair[1], pair[0], this);
    }
  }
  *entries() { for (let index = 0; index < this.#list.length; index++) yield [this.#list[index][0], this.#list[index][1]]; }
  *keys() { for (let index = 0; index < this.#list.length; index++) yield this.#list[index][0]; }
  *values() { for (let index = 0; index < this.#list.length; index++) yield this.#list[index][1]; }
  [Symbol.iterator]() { return this.entries(); }
  toString() { return formSerialize(this.#list); }
  get [Symbol.toStringTag]() { return "URLSearchParams"; }
  static {
    paramsInternals.list = (params) => params.#list;
    paramsInternals.attach = (params, url) => { params.#url = url; };
    paramsInternals.replace = (params, text) => { params.#list = formParse(text); };
  }
}

class URL {
  #record;
  #params = null;
  constructor(url, base = undefined) {
    if (arguments.length < 1) throw new TypeError("a URL takes an address");
    let against;
    if (base !== undefined) {
      const parsed = parseUrl(base, undefined);
      if (!parsed) throw new TypeError("'" + base + "' is not a valid base address");
      against = parsed.href;
    }
    const record = parseUrl(url, against);
    if (!record) throw new TypeError("'" + url + "' is not a valid address");
    this.#record = record;
  }
  static canParse(url, base = undefined) {
    try {
      new URL(url, base);
      return true;
    } catch (error) {
      return false;
    }
  }
  static parse(url, base = undefined) {
    try {
      return new URL(url, base);
    } catch (error) {
      return null;
    }
  }
  #set(part, value) {
    const href = this.#record.href;
    this.#record = urlRecord(host(OP.URL_SET, part, href.length, href + usv(value)));
  }
  // Section 6.1: the query object follows the address's query.
  #refresh() {
    if (this.#params) paramsInternals.replace(this.#params, this.#record.search.slice(1));
  }
  get href() { return this.#record.href; }
  set href(value) {
    const record = parseUrl(value, undefined);
    if (!record) throw new TypeError("'" + value + "' is not a valid address");
    this.#record = record;
    this.#refresh();
  }
  get origin() { return this.#record.origin; }
  get protocol() { return this.#record.protocol; }
  set protocol(value) { this.#set(URL_PART.PROTOCOL, value); }
  get username() { return this.#record.username; }
  set username(value) { this.#set(URL_PART.USERNAME, value); }
  get password() { return this.#record.password; }
  set password(value) { this.#set(URL_PART.PASSWORD, value); }
  get host() { return this.#record.host; }
  set host(value) { this.#set(URL_PART.HOST, value); }
  get hostname() { return this.#record.hostname; }
  set hostname(value) { this.#set(URL_PART.HOSTNAME, value); }
  get port() { return this.#record.port; }
  set port(value) { this.#set(URL_PART.PORT, value); }
  get pathname() { return this.#record.pathname; }
  set pathname(value) { this.#set(URL_PART.PATHNAME, value); }
  get search() { return this.#record.search; }
  set search(value) {
    this.#set(URL_PART.SEARCH, value);
    this.#refresh();
  }
  get searchParams() {
    if (!this.#params) {
      this.#params = new URLSearchParams(this.#record.search.slice(1));
      paramsInternals.attach(this.#params, this);
    }
    return this.#params;
  }
  get hash() { return this.#record.hash; }
  set hash(value) { this.#set(URL_PART.HASH, value); }
  toString() { return this.#record.href; }
  toJSON() { return this.#record.href; }
  get [Symbol.toStringTag]() { return "URL"; }
  static {
    urlInternals.record = (url) => url.#record;
    // An empty serialisation removes the query (section 5.2, "update").
    urlInternals.setQuery = (url, text) => { url.#set(URL_PART.SEARCH, text); };
  }
}

for (const [name, value] of Object.entries({
  ReadableStream, ReadableStreamDefaultReader, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableByteStreamController,
  ReadableStreamDefaultController, WritableStream, WritableStreamDefaultWriter, WritableStreamDefaultController, TransformStream,
  TransformStreamDefaultController, ByteLengthQueuingStrategy, CountQueuingStrategy,
  TextEncoder, TextDecoder, TextEncoderStream, TextDecoderStream, URL, URLSearchParams,
})) {
  define(name, value);
}
