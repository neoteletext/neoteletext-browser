/*! nt {"api":1} */
// Recursion through native callbacks, which the engine's own stack check may miss.
function g(n) { return [1].map(() => g(n + 1)); } setTimeout(() => { g(0); }, 0);
