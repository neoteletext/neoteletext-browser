/*! nt {"api":1} */
const names = Object.getOwnPropertyNames(globalThis).sort().join(",");
for (let from = 0; from < names.length; from += 380) nt.log("globals " + names.slice(from, from + 380));
setTimeout(() => {
  const a = Date.now(), p0 = performance.now();
  let x = 0; for (let i = 0; i < 2000; i++) x += i;
  const b = Date.now(), p1 = performance.now();
  nt.log("date " + (b - a) + " perf " + (p1 - p0) + " now " + nt.now());
  try { const sab = new SharedArrayBuffer(8); const ia = new Int32Array(sab); nt.log("atomics.wait: " + Atomics.wait(ia, 0, 0, 10)); } catch (e) { nt.log("atomics: " + e.name); }
}, 0);
