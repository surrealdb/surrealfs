# AGENTS.md

Guidance for agents working on this repository.

## What this is

`surrealfs` is a pure-Python library: a SurrealDB `file` table schema plus three
ways to expose it to an AI agent. There is no Rust here any more — the crate,
the REPL, and the `surrealfs_py` pyo3 extension were removed in the 0.2 refactor
(see commit history for the last version that had them).

## Layout

```
surrealfs/
  fs.py             SurrealFs — the async core. Everything else wraps this.
  models.py         FileEntry, SearchHit
  errors.py         exception hierarchy (dual-inherits builtin OSErrors)
  paths.py          normalisation + glob->regex
  schema/           file.surql, user.surql, apply_schema()
  tools/            args.py, handlers.py, registry, docs/*.md
  integrations/     pydantic_ai.py, json_tools.py
examples/           chat_agent.py, anthropic_loop.py, semantic_search.py
tests/
```

## Build / test

```bash
just db      # SurrealDB 3.x on :8000 (in-memory)
just test    # pytest; starts its own throwaway server if needed
just check   # ruff + pytest
```

`SURREALFS_TEST_URL` reuses an already-running server.

## Non-obvious things that will bite you

**A SurrealDB 3.x server is required, and `mem://` will not do.** `COMPUTED`
fields and `FULLTEXT` indexes do not exist in 2.x. The embedded engine in
`surrealdb[embedded]` 3.0.0a4 does parse them, but its 3.0.0-alpha core returns
`null` for computed fields on any **indexed** read — so `path` and `is_folder`
come back empty from `ls`, path resolution, and full-text search, while a plain
table scan is fine. Isolated to the engine: the same SDK against a 3.2.x server
is correct. Tests use a `surreal` subprocess; re-check `mem://` when a newer
`surrealdb-embedded` ships.

**`SurrealFs._query` goes through `query_raw()`, not `query()`.** The envelope is
what carries each statement's error *kind*, which is how we tell a retryable
transaction conflict from a real failure. It is also version-tolerant: on
surrealdb-py 2.x, `query()` returned only the *first* statement's result and
silently swallowed later failures (`LET $x = 1; THROW 'boom'` returned `None`).
On 3.x `query()` returns one entry per statement and raises — so anything
outside the library reading `query()` results must index per statement first,
which is what bit `test_parent_key_follows_a_move`.

**`path` is COMPUTED, so it cannot be indexed.** `WHERE path = $p` is a table
scan that recomputes every row's ancestry. Resolve paths by walking segments
through the `(parent, filename)` index instead — `SurrealFs._resolve` does this
in one round trip with chained `LET`s.

**Lookups go through `parent_key`, not `parent`.** A composite UNIQUE index
skips any row where one indexed field is NONE, so indexing `(parent, filename)`
would leave every root-level file unprotected — two `/notes.md` would both be
accepted. `parent_key` is a stored `VALUE` field mirroring `parent` with the root
spelled `'root'`. It cannot be COMPUTED: SurrealDB rejects indexes on computed
fields outright ("Computed fields cannot be indexed"). `_parent_key()` in
`fs.py` must keep matching the schema's `VALUE` clause.

**`child_unique` is ordered `(parent_key, filename)` on purpose.** The reverse
order still enforces uniqueness but cannot serve `WHERE parent_key = $k`, which
is the hottest query here (`ls`, path resolution, recursive `rm`/`cp`). Verified
with `EXPLAIN`; do not "tidy" the field order.

**In `_resolve`, a missing intermediate segment must dead-end.** The descent
chains `LET $k = <string>$p`, and `<string>NONE` is the literal `"NONE"`, which
matches no row. Substituting `'root'` as a fallback would make `/ghost/a.md`
resolve to `/a.md`.

**FTS scoring is broken upstream.** On 3.2.x `search::score` returns `0.0`,
`search::highlight` returns the content unchanged, and `search::offsets` returns
`null`. The match operator itself works and is index-backed, so `search_text`
matches in SurrealQL and then ranks and snippets in Python. Revisit if a later
release fixes the functions.

**HNSW needs `<|k,ef|>` with literal integers.** `<|k,COSINE|>` compiles to a
brute-force `KnnTopK` over a table scan and ignores the index. And `k`/`ef` must
be *literals*: binding them as parameters looks fine — it parses and returns the
right rows — but the planner silently drops to a brute-force scan. Confirmed with
`EXPLAIN` on both engines. They are interpolated after an `int()` cast.

**`touch` must write `content = ""`, not NONE.** `is_folder` is computed as "no
content, no bytes, no symlink", so a NONE-content row comes back as a directory.

**`rm` is a hard DELETE.** Soft-deleting via `deleted_at` cannot work: the
UNIQUE index on `(parent, filename)` means a tombstone permanently blocks
recreating a file with that name.

**Embedding staleness keys on `embedded_hash`, not `embedded_at`.** `updated_at`
has `VALUE time::now()`, so it is bumped by *every* write including the
indexer's own — a timestamp comparison marks every row stale forever and
`reindex_embeddings` never terminates.

**Transaction conflicts are expected and retried.** The `evt_file_hash` event
updates a row shortly after its content changes, so writing twice in quick
succession races it. `_query` retries anything the store marks retryable.

## Conventions

- Async throughout. The library never owns a connection — the caller passes a
  connected `AsyncSurreal` handle.
- No working directory. Every path is absolute; `cd`/`pwd` do not exist.
- The core returns dataclasses; only `tools/handlers.py` formats strings for a
  model.
- Adding a tool means one entry in `surrealfs/tools/__init__.py`, an args model,
  a handler, and a `docs/<kebab-name>.md`. Both integration surfaces pick it up
  automatically, and `tests/test_tools.py` asserts they stay in sync.
- Tool descriptions live in markdown, not docstrings. They are prompt text —
  write them for a model that has never seen this filesystem.
- Bind every value with parameters. The only interpolation is the table name
  (validated as an identifier) and KNN's `k`/`ef` (cast to `int`).
