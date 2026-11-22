/*! nt {"api":1} */
setTimeout(() => { const e = new Error("x"); Object.defineProperty(e, "stack", { get() { for (;;) {} } }); throw e; }, 0);
