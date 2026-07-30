---
title: Built-in Function Catalog
---

# Built-in Function Catalog

Every built-in SurrealQL function belongs to a namespace. Use this catalog to
jump to authoritative docs for a namespace; use the language server (see
[SKILL.md](../SKILL.md)) for exact, version-current signatures and completions.

Docs follow the pattern
`https://surrealdb.com/docs/surrealql/functions/database/<namespace>`. Treat the
docs site as the source of truth for the **latest** SurrealDB release; older
versions may lack or rename functions.

| Namespace | Covers | Docs |
| --- | --- | --- |
| `api` | Custom API endpoint definition helpers | [api](https://surrealdb.com/docs/surrealql/functions/database/api) |
| `array` | Array manipulation, search, fold/reduce, set-like ops | [array](https://surrealdb.com/docs/surrealql/functions/database/array) |
| `bytes` | Byte-array length and inspection | [bytes](https://surrealdb.com/docs/surrealql/functions/database/bytes) |
| `count` | Counting values and truthy results | [count](https://surrealdb.com/docs/surrealql/functions/database/count) |
| `crypto` | Hashing and password helpers (argon2, bcrypt, scrypt, md5, sha) | [crypto](https://surrealdb.com/docs/surrealql/functions/database/crypto) |
| `duration` | Convert and extract parts of durations | [duration](https://surrealdb.com/docs/surrealql/functions/database/duration) |
| `encoding` | base64 and similar encode/decode | [encoding](https://surrealdb.com/docs/surrealql/functions/database/encoding) |
| `file` | Storage bucket / file operations | [file](https://surrealdb.com/docs/surrealql/functions/database/file) |
| `geo` | Geospatial distance, area, bearing, hashing | [geo](https://surrealdb.com/docs/surrealql/functions/database/geo) |
| `http` | Make HTTP requests (get/post/put/patch/delete) | [http](https://surrealdb.com/docs/surrealql/functions/database/http) |
| `math` | Numeric and statistical functions | [math](https://surrealdb.com/docs/surrealql/functions/database/math) |
| `meta` | Inspect record id / table metadata | [meta](https://surrealdb.com/docs/surrealql/functions/database/meta) |
| `not` | Logical negation of a value's truthiness | [not](https://surrealdb.com/docs/surrealql/functions/database/not) |
| `object` | Build and inspect objects (keys, values, entries) | [object](https://surrealdb.com/docs/surrealql/functions/database/object) |
| `parse` | Parse emails and URLs into parts | [parse](https://surrealdb.com/docs/surrealql/functions/database/parse) |
| `rand` | Random values, ULID/UUID, sampling | [rand](https://surrealdb.com/docs/surrealql/functions/database/rand) |
| `record` | Inspect record ids (id, table, exists) | [record](https://surrealdb.com/docs/surrealql/functions/database/record) |
| `search` | Full-text search scoring and highlighting | [search](https://surrealdb.com/docs/surrealql/functions/database/search) |
| `sequence` | Read/advance defined sequences | [sequence](https://surrealdb.com/docs/surrealql/functions/database/sequence) |
| `session` | Current session/auth/namespace context | [session](https://surrealdb.com/docs/surrealql/functions/database/session) |
| `set` | Set operations (union, intersect, difference) | [set](https://surrealdb.com/docs/surrealql/functions/database/set) |
| `sleep` | Pause execution for a duration | [sleep](https://surrealdb.com/docs/surrealql/functions/database/sleep) |
| `string` | String manipulation, formatting, validation (`string::is::*`) | [string](https://surrealdb.com/docs/surrealql/functions/database/string) |
| `time` | Now, parts, rounding, conversions for datetimes | [time](https://surrealdb.com/docs/surrealql/functions/database/time) |
| `type` | Cast/convert values and construct ids (`type::thing`, `type::table`, …) | [type](https://surrealdb.com/docs/surrealql/functions/database/type) |
| `value` | Generic value inspection helpers | [value](https://surrealdb.com/docs/surrealql/functions/database/value) |
| `vector` | Vector math and distance/similarity functions | [vector](https://surrealdb.com/docs/surrealql/functions/database/vector) |

Notes:

- `type::*` functions and record-id construction have changed across major
  versions — verify the exact form against the installed version via the LSP or
  the docs rather than assuming older syntax.
- Scripting (`function() { … }`) and custom `fn::*` functions are user-defined,
  not built-ins; see the `surrealql` skill's schema reference.
