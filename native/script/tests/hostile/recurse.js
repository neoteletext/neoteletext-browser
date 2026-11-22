/*! nt {"api":1} */
function f(n) { return f(n + 1) + 1; } setTimeout(() => { f(0); }, 0);
