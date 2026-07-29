---
title: Live Queries
---

# Live Queries

Live queries push changes to the client as records are created, updated, or
deleted. They require a **WebSocket** (`ws://` / `wss://`) or an embedded engine
connection — they do **not** work over plain HTTP.

## `db.live` — subscribe to a table

`db.live<T>(table, callback?, diff?)` starts a live query on a table and returns
a `Uuid` handle. The callback fires once per change.

```ts
import { Surreal, RecordId } from "surrealdb";

interface Person { id: RecordId; name: string; }

const queryUuid = await db.live<Person>("person", (action, result) => {
  switch (action) {
    case "CREATE": console.log("created", result); break;
    case "UPDATE": console.log("updated", result); break;
    case "DELETE": console.log("deleted", result); break;
    case "CLOSE":  console.log("subscription closed"); break;
  }
});
```

Pass `diff = true` to receive JSON Patch diffs instead of full records:

```ts
const uuid = await db.live<Person>("person", (action, patches) => {
  // `patches` is an array of JSON Patch operations
}, true);
```

## `db.subscribeLive` — attach to an existing live query

When you start a live query inside a raw query (`LIVE SELECT ...`), the result is
a `Uuid`. Attach a handler to it with `subscribeLive`. You can register multiple
handlers for the same handle.

```ts
const [uuid] = await db.query<[string]>(
  "LIVE SELECT * FROM person WHERE age >= $min",
  { min: 18 },
);

db.subscribeLive<Person>(uuid, (action, result) => {
  console.log(action, result);
});
```

## `db.kill` — stop a live query

Always kill subscriptions you no longer need (and before closing in long-lived
apps) to free server resources.

```ts
await db.kill(queryUuid);
```

## Notes

- Each live query handle is tied to the current connection; reconnecting
  requires re-subscribing.
- Permissions apply: a record-authenticated user only receives changes to rows
  they are allowed to see.
- Use bound parameters (`$min` above) rather than interpolating values into the
  `LIVE SELECT` string.
