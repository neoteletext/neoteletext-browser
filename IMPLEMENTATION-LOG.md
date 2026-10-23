# Implementation log

The questions met while implementing without the owner, and the answer taken, as `~/devel/neoteletext-common/DEVELOPMENT.md` section 3.3 asks: number, question, decision, reason. A row is amended in place or superseded by a later row that says so. Behaviour the standard leaves open is a decision (`docs/architecture.md` section 2), not a row here.

## The round of 23 October 2026: the whole to-do list, the owner away

| Row | Question | Decision | Reason |
|---|---|---|---|
| 1 | Removing fonts from channels: does the browser hold any code for them? | None to remove: the fetcher never asked for a font and no font helper was built; only the documents named them. The requirements are withdrawn (decision 66), and the engine decides its own channel-font requirements by its own process. | The owner's word was to remove them completely; what existed was the promise in the documents. |
