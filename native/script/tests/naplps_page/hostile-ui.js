// A page that makes, changes and removes components without end (section
// 10.1 rule 7, for the components on NAPLPS): more buttons than the page
// has objects, menus of every row, the longest tables, colours that are
// no entry, changes on every tick. Every call past a bound must throw
// with the bound's name; nothing here may stop the page or make the host
// hold or send more than its bounds.
const ui = nt.ui;
let made = [];
const refused = new Set();
function attempt(fn) {
  try { return fn(); } catch (error) {
    if (!(error instanceof RangeError) && !(error instanceof TypeError)) throw error;
    refused.add(String(error.message).split(":")[0]);
  }
}
const items = [];
for (let index = 0; index < 256; index++) items.push({ text: "Item " + index });
const rows = [];
for (let index = 0; index < 256; index++) rows.push(["Port " + index, "12:00"]);
function flood() {
  for (const one of made.splice(0, 10)) attempt(() => one.remove());
  attempt(() => ui.text({ at: { row: 1, column: 0, columns: 4 }, text: "too long to fit", fit: "refuse" }));
  attempt(() => ui.band({ at: { row: 2, column: 0, columns: 40 }, text: "x", paper: 16 }));
  for (let index = 0; index < 70; index++)
    attempt(() => made.push(ui.button({ at: { row: 1 + (index % 23), column: (index % 3) * 13, columns: 12 }, text: "Button " + index, label: "x".repeat(64) })));
  for (let index = 0; index < 20; index++) attempt(() => made.push(ui.clock({ at: { row: 23, column: index * 2, columns: 2 } })));
  attempt(() => made.push(ui.menu({ at: { row: 1, column: 0, columns: 40, rows: 23 }, items })));
  attempt(() => made.push(ui.table({ at: { row: 1, column: 0, columns: 40, rows: 23 }, selectable: true, columns: [{ title: "Port", width: 30 }, { title: "High", width: 10 }], rows })));
  for (const one of made) attempt(() => one.set({ text: "Changed " + nt.now() }));
  for (const one of made.slice(0, 4)) attempt(() => one.focus());
  console.log("flood " + made.length + " " + Array.from(refused).sort().join(" "));
}
flood();
nt.addEventListener("tick", flood);
