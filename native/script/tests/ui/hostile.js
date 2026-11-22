// A page that asks for more of everything a component has, on every tick: components without end, menus of too many items, options too long, the same bytes held again and again, controls and triplets past the page's, places off the grid, and options that are not what they say. The host must refuse each by name and hold no more than its bounds.
const ui = nt.ui;
const refusals = {};
const attempt = (fn) => { try { return fn(); } catch (error) { if (!(error instanceof RangeError || error instanceof TypeError)) throw error; const bound = error instanceof RangeError ? error.message.split(":")[0] : "TypeError"; refusals[bound] = (refusals[bound] || 0) + 1; return null; } };
const big = "x".repeat(12);
const items = (count) => { const list = []; for (let n = 0; n < count; n++) list.push({ text: big, value: big, target: big }); return list; };
nt.addEventListener("tick", (e) => {
  attempt(() => ui.menu({ at: { row: 1, column: 20, columns: 20 }, items: new Array(257).fill({ text: "a" }) }));
  attempt(() => ui.text({ at: { row: 1, column: 20, columns: 20 }, text: "y".repeat(17000), fit: "cut" }));
  attempt(() => ui.text({ at: { row: 24, column: 0, columns: 4 }, text: "x" }));
  attempt(() => ui.text({ at: { row: 1, column: 38, columns: 4 }, text: "x" }));
  attempt(() => ui.text({ at: { row: 1, column: 20, columns: 4 }, text: { toString() { return "x"; } } }));
  attempt(() => ui.menu({ at: { row: 1, column: 20, columns: 4 }, items: [[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]] }));
  for (let n = 0; n < 40; n++) attempt(() => ui.button({ at: { row: 1 + (n % 23), column: (n > 22 ? 9 : 0), columns: 8 }, text: "b" + n }));
  for (let n = 0; n < 4; n++) attempt(() => ui.band({ at: { row: 1, column: 20 + n * 4, columns: 2, rows: 23 }, text: "", paper: n + 1 }));
  for (let n = 0; n < 80; n++) attempt(() => ui.menu({ at: { row: 1 + (n % 23), column: 18, columns: 18, rows: 1 }, items: items(n < 20 ? 200 : 1), selected: n, disabled: true }));
  const budget = nt.budget();
  console.log("components " + budget.components.used + " nodes " + budget.uiNodes.used + " controls " + budget.controls.used + " triplets " + budget.triplets.used + " " + JSON.stringify(refusals));
});
