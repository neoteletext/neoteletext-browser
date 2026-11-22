/*! nt {"api": 1, "renderer": "teletext", "keys": ["Enter"], "tick": 10} */
// The runner's own kept run: a page that uses the clock, the seeded
// random numbers, a timer, a key and a synthesizer, so that a change to
// any of them changes a kept file.
console.log("beacon on page", nt.page.number, "of", nt.channel.title, "at", nt.channel.url);
let presses = 0;
nt.addEventListener("tick", (event) => console.log("tick", event.tick, "random", Math.random().toFixed(9)));
nt.addEventListener("keydown", (event) => {
  presses += 1;
  const press = presses;
  nt.synth.start(0, "synthd");
  nt.synth.note(0, 57 + press, { ticks: 12 });
  setTimeout(() => console.log("a quarter of a second after press", press, "at", new Date().toISOString()), 250);
  event.preventDefault();
});
nt.addEventListener("close", () => console.log("closed after", presses, "presses; budget", JSON.stringify(nt.budget().timers)));
