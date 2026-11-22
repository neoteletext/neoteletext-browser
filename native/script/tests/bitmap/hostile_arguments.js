// A page that gives the bitmap every argument it should not take, on every tick: whole numbers at the ends of their range, numbers that are not whole, objects that run code when read, arrays of the wrong kind and of the largest size, lines and blits as long as the bounds allow, as many as a tick has room for. Every one must be a RangeError or a TypeError, or do bounded work; nothing may be staged for it, and the host must hold no more.
const bm = nt.teletext.bitmap;
const ends = [0, 1, -1, 79, 80, 68, 69, 1024, 1025, -1024, -1025, 2147483647, -2147483648, 4294967296, 1e300, 0.5, NaN, Infinity];
let ran = 0;
const evil = { valueOf() { ran++; return 1; }, toString() { ran++; return "1"; } };
let other = 0, calls = 0;
const attempt = (work) => { calls++; try { work(); } catch (error) { if (!(error instanceof RangeError || error instanceof TypeError)) other++; } };
const big = new Uint8Array(1024 * 1024);
nt.addEventListener("tick", () => {
  for (const a of ends) for (const b of ends) {
    attempt(() => bm.set(a, b, 1));
    attempt(() => bm.get(a, b));
    attempt(() => bm.line(a, b, b, a, 2));
    attempt(() => bm.rect(a, b, b, a, 3));
    attempt(() => bm.rect(a, b, 5, 5, a));
    attempt(() => bm.blit(a, b, 2, 2, new Uint8Array([1, 2, 3, 4])));
    attempt(() => bm.blit(0, 0, a, b, big));
    attempt(() => bm.text(a, b, "text", { ink: b, paper: a }));
    attempt(() => bm.rows(a, b));
    attempt(() => bm.clear(a));
  }
  for (const wrong of [evil, null, "1", [1], {}, true, 1n, Symbol.iterator, () => 1]) {
    attempt(() => bm.set(wrong, 0, 1));
    attempt(() => bm.set(0, 0, wrong));
    attempt(() => bm.line(0, 0, wrong, 5, 1));
    attempt(() => bm.rect(0, 0, 5, 5, 1, wrong));
    attempt(() => bm.blit(0, 0, 1, 1, wrong));
    attempt(() => bm.blit(0, 0, 1, 1, new Uint8Array(1), { transparent: wrong }));
    attempt(() => bm.text(1, 0, wrong));
    attempt(() => bm.text(1, 0, "x", wrong));
    attempt(() => bm.rows(wrong, wrong));
    attempt(() => bm.clear(wrong));
  }
  // The longest lines and the largest blit the bounds allow, many times.
  for (let n = 0; n < 200; n++) bm.line(-1024, -1024 + n, 1024, 1024 - n, 1 + (n % 7));
  for (let n = 0; n < 4; n++) attempt(() => bm.blit(-512, -512, 1024, 256, big.subarray(0, 262144)));
  bm.release();
  console.log("calls " + calls + " other " + other + " ran " + ran);
});
