---
title: Embedded SurrealDB in JavaScript
---

# Embedded SurrealDB in JavaScript

Run SurrealDB directly inside your process — no server required. Register an
embedded engine on the `Surreal` instance, then `connect` to an embedded address
instead of a `ws://`/`http://` URL. Combine with `createRemoteEngines()` if you
also want the same instance to be able to connect to remote servers.

> Both engine packages are ES modules — use `import`, not `require`.

## Node.js — `@surrealdb/node`

```bash
npm i surrealdb @surrealdb/node
```

```ts
import { Surreal, createRemoteEngines } from "surrealdb";
import { createNodeEngines } from "@surrealdb/node";

const db = new Surreal({
  engines: {
    ...createRemoteEngines(),
    ...createNodeEngines(),
  },
});

// In-memory (nothing persisted)
await db.connect("mem://");

// RocksDB persistence
await db.connect("rocksdb://path/to/db");

// SurrealKV persistence
await db.connect("surrealkv://path/to/db");

await db.use({ namespace: "test", database: "test" });

await db.create("person", { name: "Tobie" });
console.log(await db.select("person"));

await db.close();
```

## Browser / Deno — `@surrealdb/wasm`

```bash
npm i surrealdb @surrealdb/wasm
```

```ts
import { Surreal, createRemoteEngines } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";

const db = new Surreal({
  engines: {
    ...createRemoteEngines(),
    ...createWasmEngines(),
  },
});

// In-memory
await db.connect("mem://");

// IndexedDB persistence (browser)
await db.connect("indxdb://my-database");

await db.use({ namespace: "test", database: "test" });
```

## Choosing an engine

| Address                  | Engine             | Persistence            |
| ------------------------ | ------------------ | ---------------------- |
| `mem://`                 | node / wasm        | none (in-memory)       |
| `rocksdb://path`         | `@surrealdb/node`  | on-disk (RocksDB)      |
| `surrealkv://path`       | `@surrealdb/node`  | on-disk (SurrealKV)    |
| `indxdb://name`          | `@surrealdb/wasm`  | browser IndexedDB      |
| `ws://` / `wss://`       | remote (built-in)  | remote server          |
| `http://` / `https://`   | remote (built-in)  | remote server          |

The full SDK API (CRUD, `query`, live queries) is identical across embedded and
remote engines — only the `connect` address and registered engines differ. Live
queries work on embedded and WebSocket connections, but not over plain HTTP.
