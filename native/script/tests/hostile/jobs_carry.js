/*! nt {"api":1} */
// A promise chain that never ends but yields.
function spin() { Promise.resolve().then(spin); } spin(); setInterval(() => {}, 0);
