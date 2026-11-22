/*! nt {"api":1} */
const a = new Uint8Array(40 * 1024 * 1024);
setTimeout(() => { for (;;) { a.copyWithin(0, 1); } }, 0);
