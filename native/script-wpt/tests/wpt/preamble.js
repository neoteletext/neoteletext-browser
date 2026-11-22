// What a web-platform-tests file written for a worker's scope finds
// before testharness.js: the names wptserve's `.any.worker.js` wrapper
// and a worker's global give it. `__WPT` is filled in by the harness.
globalThis.self = globalThis;
self.GLOBAL = { isWindow() { return false; }, isWorker() { return true; }, isShadowRealm() { return false; } };
(function () {
  const url = new URL(__WPT.url);
  self.location = Object.freeze({
    href: url.href, origin: url.origin, protocol: url.protocol, host: url.host, hostname: url.hostname, port: url.port,
    pathname: url.pathname, search: url.search, hash: url.hash, toString() { return url.href; },
  });
})();
// There is no synchronous loader here: a script the test's META names is
// written in before the test by the harness, and a later call is refused.
self.importScripts = function () { throw new Error("importScripts is not provided to a page's script"); };
