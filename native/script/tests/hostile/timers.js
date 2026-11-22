/*! nt {"api":1} */
// Timers that are each busy for a few milliseconds every tick.
for (let i = 0; i < 31; i++) setInterval(() => { let x = 0; for (let j = 0; j < 60000; j++) { x += j; } }, 0);
