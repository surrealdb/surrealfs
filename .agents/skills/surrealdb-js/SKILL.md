---
name: surrealdb-js
description: "Using SurrealDB from JavaScript and TypeScript with the official surrealdb SDK, covering connecting (WebSocket/HTTP and embedded engines), authentication, CRUD, parameterized queries, and live queries. Use when connecting to SurrealDB from Node, Deno, Bun, or the browser, using the surrealdb npm package, or performing CRUD and real-time operations from JS/TS code. Triggers: surrealdb JS, surrealdb.js, new Surreal(), db.query, db.create, db.live, TypeScript SDK, npm i surrealdb."
metadata:
  author: surrealdb
  version: "0.2.0"
---

# SurrealDB JavaScript SDK

The official SDK (`surrealdb` on npm) works in Node.js, Deno, Bun, and the
browser. It connects to a remote SurrealDB instance over WebSocket/HTTP, or runs
an embedded engine in-process. Target the **latest** stable `surrealdb` release;
install without pinning (`npm i surrealdb`) unless the user requires a specific
version.

## Installation

```bash
npm i surrealdb
# or: pnpm i surrealdb / yarn add surrealdb / bun add surrealdb
```

## Connect & select namespace/database

Always `connect`, then `use` a namespace + database, then authenticate. Close
the connection when finished.

```ts
import { Surreal } from "surrealdb";

const db = new Surreal();
await db.connect("ws://127.0.0.1:8000/rpc");
await db.use({ namespace: "test", database: "test" });
await db.signin({ username: "root", password: "root" });

// ... work ...

await db.close();
```

Run a local server with `surreal start -u root -p root rocksdb:mydb` (or
in-memory with `surreal start -u root -p root`). To run SurrealDB in-process
with no server, see [references/embedded.md](references/embedded.md).

## Authentication

```ts
// Root / namespace / database users
await db.signin({ username: "root", password: "root" });
await db.signin({ namespace: "test", username: "ns_user", password: "..." });
await db.signin({ namespace: "test", database: "test", username: "db_user", password: "..." });

// Record (scope) access — sign in / sign up against a DEFINE ACCESS method
const token = await db.signin({
  namespace: "test",
  database: "test",
  access: "user",          // name of the access method
  variables: { email: "a@b.com", pass: "secret" },
});

await db.signup({
  namespace: "test",
  database: "test",
  access: "user",
  variables: { email: "a@b.com", pass: "secret" },
});

await db.authenticate(token); // re-auth with a stored JWT
await db.invalidate();        // log out the current session
const me = await db.info();   // info about the authenticated record user
```

## CRUD

Pass a table as a string (`"person"`) or a `RecordId` for a specific record.
See [references/data-types.md](references/data-types.md) for `RecordId`, `Table`,
`Duration`, `Decimal`, and other value classes.

```ts
import { RecordId } from "surrealdb";

interface Person { id: RecordId; name: string; age?: number; }

// Create (random id, or a specific RecordId)
await db.create<Person>("person", { name: "Tobie" });
await db.create<Person>(new RecordId("person", "tobie"), { name: "Tobie" });

// Select all from a table, or a single record
const people = await db.select<Person>("person");
const tobie = await db.select<Person>(new RecordId("person", "tobie"));

// Replace the whole record(s)
await db.update<Person>(new RecordId("person", "tobie"), { name: "Tobie", age: 30 });

// Insert or update (upsert)
await db.upsert<Person>(new RecordId("person", "tobie"), { name: "Tobie" });

// Merge partial data into record(s)
await db.merge<Person>("person", { active: true });

// JSON Patch a record
await db.patch(new RecordId("person", "tobie"), [
  { op: "replace", path: "/name", value: "Tobie M H" },
]);

// Bulk insert
await db.insert<Person>("person", [{ name: "A" }, { name: "B" }]);

// Graph relation:  person:tobie ->wrote-> article:surreal
await db.relate(new RecordId("person", "tobie"), "wrote", new RecordId("article", "surreal"));

// Delete record(s)
await db.delete("person");
await db.delete(new RecordId("person", "tobie"));
```

## Queries

`db.query<T>(sql, vars)` runs raw SurrealQL and returns an **array with one entry
per statement**. Always pass user input via bound parameters, never string
interpolation.

```ts
const [people, count] = await db.query<[Person[], number]>(
  `SELECT * FROM person WHERE age > $min;
   SELECT count() FROM person GROUP ALL;`,
  { min: 18 },
);

// Bind values and RecordIds as parameters
const [found] = await db.query<[Person[]]>(
  "SELECT * FROM person WHERE id = $who",
  { who: new RecordId("person", "tobie") },
);
```

Other helpers: `db.let(name, value)` / `db.unset(name)` set session parameters
usable as `$name` in later queries; `db.run(fnName, version, args)` invokes a
defined function; `db.queryRaw()` returns the unparsed RPC response (status +
timing per statement).

## Live queries

Subscribe to real-time changes on a table with `db.live()`. Requires a
WebSocket (or embedded) connection — not HTTP. See
[references/live-queries.md](references/live-queries.md).

```ts
const queryUuid = await db.live<Person>("person", (action, result) => {
  // action: "CREATE" | "UPDATE" | "DELETE" | "CLOSE"
  console.log(action, result);
});

await db.kill(queryUuid); // stop the subscription
```

## References

- [references/data-types.md](references/data-types.md) — `RecordId`, `Table`,
  `Duration`, `Decimal`, `Uuid`, geometry, and CBOR mapping.
- [references/embedded.md](references/embedded.md) — embedded engines via
  `@surrealdb/node` (RocksDB / SurrealKV / in-memory) and `@surrealdb/wasm`
  (in-memory / IndexedDB).
- [references/live-queries.md](references/live-queries.md) — `live`,
  `subscribeLive`, and `kill` patterns.

## Resources

- [SDK documentation](https://surrealdb.com/docs/sdk/javascript)
- [GitHub repository](https://github.com/surrealdb/surrealdb.js)
