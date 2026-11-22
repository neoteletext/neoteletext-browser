/*! nt {"api": 1, "renderer": "teletext", "network": true} */
// The runner's page for the live network: it fetches a file of its own
// channel through the real web client and says what came back.
fetch("light.json")
  .then((response) => response.json().then((light) => console.log("light", response.status, response.headers.get("content-type"), light.period, light.colour)))
  .catch((error) => console.log("failed", error.name, error.message));
fetch("no-such-file.json").then((response) => console.log("missing", response.status), (error) => console.log("failed", error.name));
