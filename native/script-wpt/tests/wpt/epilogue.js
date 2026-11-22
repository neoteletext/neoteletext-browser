// After the test file: what wptserve's wrapper does (`done()`), and the
// results written to the run's log, a few lines a tick, for the harness.
(function () {
  const lines = [];
  // A log line holds 400 characters: a name is cut to fit, in ASCII.
  const ascii = (text, most) => {
    let out = "";
    for (const unit of String(text)) {
      const code = unit.codePointAt(0);
      out += code >= 0x20 && code < 0x7f && unit !== "\\" ? unit : "\\u{" + code.toString(16) + "}";
      if (out.length >= most) break;
    }
    return out.slice(0, most);
  };
  let reported = false;
  const report = (tests, status) => {
    if (reported) return;
    reported = true;
    lines.push("WPT S " + status.status + " " + ascii(status.message || "", 200));
    for (const test of tests) lines.push("WPT T " + test.status + " " + ascii(test.name, 240) + " ||| " + ascii(test.message || "", 100));
    lines.push("WPT END");
  };
  add_completion_callback(report);
  setInterval(() => { for (const line of lines.splice(0, 40)) console.log(line); }, 0);
  // A file whose tests never finish is ended as timed out, so that the
  // cases that did finish are reported.
  setTimeout(() => { if (!reported && typeof timeout === "function") timeout(); }, __WPT.timeout);
  done();
})();
