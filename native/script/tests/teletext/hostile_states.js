// A page that asks the receiver for more animation state than it has: every control given a look and every sprite a frame and a cycle, on every tick, with sprites made and removed between. The engine holds 32 states for a page's sprites and controls and lets 16 cycle; it skips an operation it has no room for without a word to the page. The host must refuse those by name instead, so that what the script believes is what is shown.
const tt = nt.teletext;
const refusals = {};
const attempt = (fn) => { try { return fn(); } catch (error) { if (!(error instanceof RangeError)) throw error; const bound = error.message.split(":")[0]; refusals[bound] = (refusals[bound] || 0) + 1; return null; } };
const a = tt.block(1, 1).cell(0, 0, { char: "a" }), b = tt.block(1, 1).cell(0, 0, { char: "b" }), c = tt.block(1, 1).cell(0, 0, { char: "c" });
const controls = []; for (let n = 0; n < 8; n++) controls.push(tt.control(1 + (n >> 3), (n & 7) * 5, 4));
const sprites = []; for (let n = 0; n < 28; n++) sprites.push(tt.sprite({ frames: [a, b, c], row: 6 + (n >> 3), column: (n & 7) * 5 }));
nt.addEventListener("tick", (e) => {
  for (const [n, one] of sprites.entries()) { attempt(() => one.moveBy(0, (e.tick + n) % 2 ? 1 : -1)); attempt(() => { if ((e.tick + n) % 3 !== 2) one.cycle(0, 2, 2 + (n % 5), { once: n % 2 === 0 }); else one.frame = 1 + ((e.tick + n) % 2); }); }
  for (const [n, one] of controls.entries()) attempt(() => { one.look = (e.tick + n) % 4 === 0 ? "hover" : (e.tick + n) % 4 === 1 ? "pressed" : null; });
  if (e.tick % 5 === 4) { const gone = sprites.shift(); gone.remove(); sprites.push(tt.sprite({ frames: [a, b, c], row: 12 + (e.tick % 10), column: e.tick % 38 })); }
  console.log(JSON.stringify(refusals));
});
