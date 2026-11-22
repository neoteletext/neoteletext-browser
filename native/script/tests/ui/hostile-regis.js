// A page that asks on every tick for more of everything a component has on ReGIS: components without end, menus of too many items and of more rows than the page has drawings for, options too long, the same bytes held again and again, colours and labels ReGIS does not have, places off the grid, and options that are not what they say. The host must refuse each by name, hold no more than its bounds, and stage nothing the renderer would refuse.
const ui = nt.ui;
const refusals = {};
const attempt = (fn) => { try { return fn(); } catch (error) { if (!(error instanceof RangeError || error instanceof TypeError)) throw error; const bound = error instanceof RangeError ? error.message.split(":")[0] : "TypeError"; refusals[bound] = (refusals[bound] || 0) + 1; return null; } };
const big = "x".repeat(12);
const plain = (count) => { const list = []; for (let n = 0; n < count; n++) list.push({ text: big }); return list; };
let failed = 0;
addEventListener("error", () => { failed++; });
const buttons = [];
nt.addEventListener("tick", (e) => {
  attempt(() => ui.menu({ at: { row: 1, column: 20, columns: 20 }, items: new Array(257).fill({ text: "a" }) }));
  attempt(() => ui.text({ at: { row: 1, column: 20, columns: 20 }, text: "y".repeat(17000), fit: "cut" }));
  attempt(() => ui.text({ at: { row: 24, column: 0, columns: 4 }, text: "x" }));
  attempt(() => ui.text({ at: { row: 1, column: 38, columns: 4 }, text: "x" }));
  attempt(() => ui.text({ at: { row: 1, column: 20, columns: 4 }, text: { toString() { return "x"; } } }));
  attempt(() => ui.text({ at: { row: 1, column: 20, columns: 4 }, text: "x", ink: 16 }));
  attempt(() => ui.button({ at: { row: 1, column: 20, columns: 4 }, text: "x", label: "l".repeat(65) }));
  // Menus of 256 rows: the page's drawings run out long before its components do.
  for (let n = 0; n < 7; n++) attempt(() => ui.menu({ at: { row: 1 + (n % 23), column: 18, columns: 18, rows: 1 }, items: plain(256), selected: n }));
  for (let n = 0; n < 10 && e.tick === 0; n++) attempt(() => ui.text({ at: { row: 1 + (n % 23), column: 0, columns: 8 }, text: "y".repeat(14000), fit: "cut" }));
  for (let n = 0; n < 60; n++) { const made = attempt(() => ui.button({ at: { row: 1 + (n % 23), column: (n > 22 ? 9 : 0), columns: 8 }, text: "b" + n })); if (made) buttons.push(made); }
  // The same bytes held again and again: a component's options replaced by longer ones.
  for (const button of buttons.slice(0, 6)) attempt(() => button.set({ text: "y".repeat(14000) }));
  const budget = nt.budget();
  console.log("components " + budget.components.used + " nodes " + budget.uiNodes.used + " drawings " + budget.drawings.used + " contents " + budget.contentBytes.used + " failed " + failed + " " + JSON.stringify(refusals));
});
