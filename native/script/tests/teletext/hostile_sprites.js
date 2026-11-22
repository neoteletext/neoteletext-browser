// A page that attacks sprites every way the API has, on every tick (the security review's leads on host memory and on bounds, TODO-7272 and TODO-7336): more sprites than a page has, frames past an object's triplets, sixteen-frame sprites of distinct frames until the Object page is full, churn, operations on dead handles and past every range. The host must refuse each by name, hold no more than the page's sprites and one Object page, and never send the receiver an operation it has no room for.
const tt = nt.teletext;
let serial = 0;
const refusals = {};
const attempt = (fn) => { try { return fn(); } catch (error) { if (!(error instanceof RangeError || error instanceof TypeError)) throw error; const bound = error instanceof RangeError ? error.message.split(":")[0] : "TypeError"; refusals[bound] = (refusals[bound] || 0) + 1; return null; } };
const distinct = (columns, rows) => { const block = tt.block(columns, rows); for (let row = 0; row < rows; row++) for (let column = 0; column < columns; column++) block.cell(row, column, { sixels: 1 + (serial++ % 63), ink: serial % 32, paper: (serial >> 3) % 32, separated: (serial & 1) === 1 }); return block; };
const held = [];
nt.addEventListener("tick", (e) => {
  attempt(() => tt.sprite({ frames: [distinct(40, 23)], row: 1, column: 0 }));
  attempt(() => tt.sprite({ frames: new Array(17).fill(distinct(1, 1)), row: 1, column: 0 }));
  for (let n = 0; n < 40; n++) {
    const frames = []; if (n % 4 === 0) for (let f = 0; f < 16; f++) frames.push(distinct(8 + (n % 5), 3)); else frames.push(distinct(1, 1), distinct(1, 1));
    const made = attempt(() => tt.sprite({ frames, row: (n * 7) % 24, column: (n * 5) % 40, layer: 1 + (n % 3), order: n - 20, detectable: true, wheel: n % 4 !== 1, label: "s" + n }));
    if (made) held.push(made);
  }
  for (const one of held) {
    attempt(() => one.moveBy(1, 1)); attempt(() => one.moveTo(99, 99)); attempt(() => one.moveTo(99, 0)); attempt(() => { one.frame = 15; }); attempt(() => one.cycle(0, one.frames - 1, 1 + (serial++ % 7), { once: (serial & 1) === 0 }));
    attempt(() => { one.layer = 9; }); attempt(() => { one.order = 500; }); attempt(() => { one.shown = !one.shown; });
  }
  if (e.tick % 3 === 2) { for (const one of held.splice(0, 9)) { one.remove(); attempt(() => one.moveBy(0, 1)); attempt(() => one.cycle(0, 1, 1)); } }
  const budget = nt.budget();
  console.log("sprites " + budget.sprites.used + " frames " + budget.spriteFrames.used + " triplets " + budget.objectPageTriplets.used + " " + JSON.stringify(refusals));
});
