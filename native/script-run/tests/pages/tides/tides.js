/*! nt {"api": 1, "renderer": "teletext", "network": true} */
// The runner's kept run of the web APIs: a fetch answered from the mock
// table, a fetch the table has no entry for, and an event stream, so
// that a change to how a request leaves or an answer arrives changes the
// kept log.
fetch("tides.json")
  .then((response) => {
    console.log("head", response.status, response.statusText, response.type, response.headers.get("content-type"), response.url);
    return response.json();
  })
  .then((tides) => console.log("height", tides.height, "at", nt.now()), (error) => console.log("failed", error.name));
fetch("missing.json").then(() => console.log("answered"), (error) => console.log("no entry in the table:", error.constructor.name, error.message));
const feed = new EventSource("feed");
feed.onopen = () => console.log("feed open at", nt.now());
feed.onmessage = (event) => {
  console.log("feed", event.data, "id", event.lastEventId, "from", event.origin, "at", nt.now());
  if (event.lastEventId === "7") feed.close();
};
nt.addEventListener("close", () => console.log("closed; feed", feed.readyState, "requests", JSON.stringify(nt.budget().requests)));
