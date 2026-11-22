/*! nt {"api":1} */
setTimeout(() => { function spin() { return Promise.resolve().then(spin); } spin(); }, 0);
