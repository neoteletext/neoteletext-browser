/*! nt
{ "api": 1, "renderer": "naplps", "text": true, "pointer": "press",
  "keys": ["Tab", "Enter", " ", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown", "Escape", "Backspace"] }
*/
// The thirteen first kinds of component on one NAPLPS page (script-api.md section 8; work package U3): the page of work package U1, on NAPLPS, with what NAPLPS adds: a colour of the page's own behind the band and a heading drawn narrower to fit. Every event a component reports is logged, and two listeners show a default action cancelled and one replaced.
const ui = nt.ui;
const dusk = nt.naplps.colours.define("dusk", "#324");
ui.frame({ id: "frame", at: { row: 1, column: 0, columns: 40, rows: 23 }, title: "HARBOUR CONTROL", hint: "Tab moves, Enter chooses", prompts: ["News", "Tides", "Wind", "Help"] });
ui.text({ id: "heading", at: { row: 3, column: 0, columns: 40 }, text: "Thirteen kinds of component on one NAPLPS page, drawn narrower to fit", fit: "shrink", ink: "green" });
ui.band({ id: "band", at: { row: 4, column: 0, columns: 40 }, text: "STATUS", paper: dusk });
const story = ui.paragraph({ id: "story", at: { row: 5, column: 0, columns: 20, rows: 3 }, text: "The lifeboat was launched at dawn and stood by the trawler until the tide turned and the harbour pilot could bring her in past the bar." });
ui.clock({ id: "clock", at: { row: 5, column: 30, columns: 9 }, format: "HH:MM:SS", zone: "utc", ink: "yellow" });
const loading = ui.progress({ id: "loading", at: { row: 6, column: 22, columns: 18, rows: 2 }, text: "Loading charts", value: 40 });
const menu = ui.menu({ id: "menu", at: { row: 9, column: 0, columns: 19, rows: 4 }, items: [
  { text: "Shipping forecast", value: "ship" }, { text: "Tide tables", target: "103" }, { text: "Closed for winter", disabled: true },
  { text: "Wind and swell" }, { text: "Notices to mariners", value: 5 }, { text: "Harbour webcam", value: "cam" } ], label: "Sections" });
const table = ui.table({ id: "tides", at: { row: 9, column: 20, columns: 20, rows: 4 }, selectable: true,
  columns: [{ title: "Port", width: 11 }, { title: "High", width: 8, align: "right" }],
  rows: [["Dover", "11:02"], ["Harwich", "11:48"], ["Lowestoft", "09:17"], ["Whitby", "03:55"], ["Leith", "02:31"]] });
const link = ui.link({ id: "link", at: { row: 14, column: 0, columns: 30 }, text: "Shipping forecast in full", target: "102" });
const refresh = ui.button({ id: "refresh", at: { row: 15, column: 0, columns: 12 }, text: "Refresh" });
const sound = ui.toggle({ id: "sound", at: { row: 15, column: 14, columns: 14 }, text: "Sound" });
const volume = ui.slider({ id: "volume", at: { row: 17, column: 0, columns: 21 }, min: 0, max: 10, value: 4 });
const name = ui.field({ id: "name", at: { row: 19, column: 0, columns: 20 }, placeholder: "Your vessel", longest: 12 });
for (const type of ["move", "activate", "change", "page", "submit", "cancel"])
  nt.addEventListener(type, (e) => console.log(type + " " + e.component.id + " " + JSON.stringify(e.value) + (e.defaultPrevented ? " cancelled" : "")));
// A default cancelled: the volume never passes 6. A default replaced: Refresh fills the progress bar.
volume.addEventListener("change", (e) => { if (e.value > 6) e.preventDefault(); });
refresh.addEventListener("activate", () => loading.set({ value: 100, text: "Charts loaded" }));
nt.addEventListener("keydown", (e) => { if (e.key === "Escape" && e.target !== name) { console.log("escape at " + (e.target === nt ? "nt" : e.target.id)); } });
const used = nt.budget();
console.log("made " + used.components.used + " components, " + used.actors.used + " objects, " + used.symbols.used + " symbols; focus on " + story.id);
