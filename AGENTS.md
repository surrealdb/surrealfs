# AGENTS.md

Guidance for agents working on this repository.

## What this is

`surrealfs` is a pure-Python library: a SurrealDB `file` table schema plus five
ways to expose it to an AI agent — four tool surfaces and a Hermes memory provider.

## Layout

```
surrealfs/
  fs.py             SurrealFs — the async core. Everything else wraps this.
  models.py         FileEntry, SearchHit
  errors.py         exception hierarchy (dual-inherits builtin OSErrors)
  paths.py          normalisation + glob->regex
  embed.py          OpenAI embedder + `python -m surrealfs.embed` indexer daemon
  schema/           file.surql, record_auth.surql, apply_schema()
  users.py          `python -m surrealfs.users` — record-auth provisioning
  tools/            args.py, handlers.py, registry, docs/*.md
  browser/          the `surrealfs-browser` web UI — page.html, the Starlette
                    app, its chat agent, and the CLI in __main__.py
  integrations/     _connect.py (env connection, shared by the browser and the
                    Hermes surfaces), and one dir per integration, each with its
                    own README.md: pydantic_ai/, json_tools/, hermes/ (plugin +
                    bundled skill), hermes_memory/ (memory provider)
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

**The Hermes plugin's tool names are prefixed, and the prefix is load-bearing.**
Hermes has one flat tool namespace, and `tools/registry.py` rejects a name
already owned by another toolset by *logging an error and returning* — no
exception. Registering `write_file` would therefore lose to Hermes' built-in
disk one, silently, and the model would go on calling that instead. `PREFIX` in
`integrations/hermes/__init__.py` keeps all fifteen and disambiguates the two
filesystems for the model; `_describe` rewrites the cross-references inside the
shared `docs/*.md` text to match, since `cat.md` telling the model to use
`read_bytes` is a dead end when the tool is `surrealfs_read_bytes`.

**That plugin registers `is_async=True` rather than bridging to async itself.**
Hermes calls sync handlers straight from `registry.dispatch`, and the gateways
(Telegram, Discord) dispatch from inside a running loop — an `asyncio.run` in the
handler would raise there and break every call. Hermes' own `_run_async` handles
all three cases, and picks a *different* loop depending on the call site, which
is also why the connection cannot be cached across calls: a SurrealDB WebSocket
belongs to the loop it was opened on.

**The bundled `surrealfs:notes` skill is only reachable because of the
`pre_llm_call` hook.** Hermes does not advertise plugin skills: `register_skill`'s
own docstring says they never enter the `~/.hermes/skills/` tree and are absent from
the system prompt's `<available_skills>` index, and `skills_list()` builds its answer
by scanning that directory, so it does not list them either. `skill_view` resolves
them only by exact qualified name — which nothing would ever tell the agent. Hence
the hook, which injects that name on the first turn of a session. Delete it and the
skill is dead weight, with nothing failing to say so.

**Never let `integrations/hermes/__init__.py` mention `MemoryProvider`.**
`hermes_cli/plugins.py` decides what kind of plugin a directory is by *text-scanning*
the first 8KB of its `__init__.py`: find `MemoryProvider` or
`register_memory_provider` and it coerces the plugin to `kind="exclusive"`, routing it
to memory-provider discovery instead of the plugin manager. A mention in a mere
comment would therefore stop the tools and the skill from loading for anyone who
installs by symlink, silently. That is why the memory provider lives in its own
`hermes_memory/` directory, and why `tests/test_memory_provider.py` guards the string.

**Memory providers are found by directory, never by entry point.** Hermes scans
`$HERMES_HOME/plugins/<name>/` and matches the directory name against
`memory.provider` in `config.yaml`, so `pip install surrealfs` cannot deliver one —
it has to be symlinked, and the tool plugin and the provider install separately. Note
`PluginContext` has no `register_memory_provider` despite what the plugin docs claim;
the provider's `register()` is called with the memory loader's own collector, which
also has no `register_skill` — hence two independent `register()` functions rather
than one shared entry point.

**Hermes cannot host the embedding indexer; a script-only cron job does.** There is
no plugin-declared background process: `PluginContext` registers tools, hooks, skills,
CLI commands, middleware and providers, the manifest has no field for it, and
`tools/process_registry.py` only backs `terminal(background=true)` — session-scoped
and killed on reset. `register_auxiliary_task` sounds like it and is LLM side-jobs.
So `docs/hermes-indexer.md` — one copy, linked from both Hermes READMEs since either
plugin can be installed alone — tells users to schedule `python -m surrealfs.embed
--once` with `hermes cron create 'every 5m' --no-agent --script …`, which skips
inference entirely.
Two constraints the recipe exists to satisfy: the script must resolve inside
`$HERMES_HOME/scripts/`, and cron sanitizes the subprocess env against a
provider-credential blocklist that `OPENAI_API_KEY` is on — so the key comes from a
sourced file, never from the gateway's environment.

**The browser's `.env` must be found with `find_dotenv(usecwd=True)`.** Plain
`load_dotenv()` walks up from the *calling module's* directory, not the working
directory — so `surrealfs-browser` run from anywhere inside a checkout silently
picked up this repo's own `.env` (a cloud instance) instead of the one the user was
standing in, and failed authentication against the server their flags named. The
whole point of the tool is that a person runs it in their own directory.

**`search_text` defaults to `match="any"` (`@1,OR@`), not AND.** With `@1@`,
`'what tone does the user prefer'` finds nothing while `'prefer'` finds the note —
and a caller that hands the model an empty result has cost it a turn, or worse: the
bundled skill tells the agent to search before it writes, so a false empty produces a
duplicate note. Since this surface returns a ranked list of snippets, a weak hit is
cheap to dismiss and an unseen one is not. `match="all"` is there for callers that
want a precise filter and read empty as a real answer.

**Scoring must run on `search::analyze` output, or stemmed matches score zero.**
This was the real defect behind a long detour. The index analyzes with
`snowball(english)`, so a query for `prefer` matches a file saying "Prefers" — but the
Python scorer counted *raw words*, found no literal "prefer", scored the file `0.000`,
and sorted it below files that did not answer the question. Hence
`search::analyze($analyzer, content) AS tokens` in the query, `$terms` for the
analyzed query, and `ANALYZER` in `fs.py` kept in step with `schema/file.surql`.
Ranking is Okapi BM25 (`_bm25`) over those tokens: `df` from the matched rows only,
stopwords dropped from scoring but never from matching (`_scoring_terms`).

Measured on `test_ranking_quality_over_a_realistic_corpus`, which exists to keep this
honest: raw term counts scored **MRR 0.591**, BM25 on analyzer tokens **0.823**.

Three things that look like fixes and measurably were not:

- *Stripping stopwords from the query* changes which rows match, not how they rank —
  byte-identical ordering on the same corpus.
- *BM25 alone*, while still counting raw words, ranked no better than term counts. It
  was the stemming mismatch doing the damage, not the formula.
- *Renaming the memory turn headers* (`## User` → `## Asked`) to stop every turn file
  matching "user" made things **worse** overall, 0.823 → 0.750: it helps a query where
  "user" is incidental and hurts one that is genuinely about what the user said.

What remains, and is inherent to lexical search: a query word that is incidental to
the question still pulls in files dense with it — "what does the user prefer" ranks a
`user`-heavy note above the preferences file, because lexically "user" is the rarer
term. That is what the vector arm is for. Do not tune BM25 to paper over it; a test
asserting the opposite was deleted rather than have the scorer chase it.

Related ceiling: `limit` is applied after Python ranking, so `search_text` fetches
the content of every matching row and now asks the server to tokenise each one.
Fine for notes; a corpus where one common term matches thousands of files needs
server-side scoring before a SQL `LIMIT` can be trusted not to wreck the order.

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

**Permission checks are in `SurrealFs`, and a missed one in a *bulk* query is a
disclosure.** `_require` covers the ancestry of anything resolved by path, so
single-path operations are safe by construction. The four queries that return
many rows without resolving them — `ls`, `glob`, `search_text`,
`search_semantic` — each have to append `fn::sfs_can_read` via `self._readable`,
and `search` returns file *content*, so forgetting one hands another user's
notes to a model. `tests/test_permissions.py` asserts all four.

**A row with no `mode` must stay readable-by-root, never error and never open.**
`NONE % 2` raises, and `gate` is recomputed whenever a row is rewritten, so the
natural behaviour takes down every read *and* `docs/migrate_permissions.surql`
itself. `fn::sfs_gate`, `fn::sfs_bits` and `FileEntry.from_row` all coalesce a
missing owner to `root` and a missing mode to no bits. Keep the three in step.
Delete all three once no pre-permissions database is left to migrate.

**`FOR $x IN (SELECT ...)` is rejected** with "Cannot execute statement using
value: <first row>". Bind the subquery with `LET` first. Bit us in the
migration.

**The `file` table's PERMISSIONS clause says `fn::sfs_gate(parent)`, and
"tidying" it to `gate` opens the whole tree — silently.** A table permission is
evaluated against the raw record *before* computed fields run, so a same-table
COMPUTED field reads NONE there, and a NONE `gate` means "no closed ancestor",
i.e. readable. Verified on 3.2.4. Traversing to a *linked* record is fine — a
permission predicate reads links with permissions off and does compute their
fields — which is why the indirection exists. The clause is inert under a system
credential, so only `tests/test_record_auth.py` can catch a regression here.

**`gate` is COMPUTED, so it cannot be indexed — and a COMPUTED field reading
NULL through an index would silently disable filtering rather than error.** That
is why the permission tests exercise the FULLTEXT and HNSW paths specifically,
and why `EXPLAIN` was checked (`idx_file_content` still drives a `FullTextScan`,
with the predicate applied above it). Verified on 3.2.4; re-check it on a server
upgrade.

**HNSW applies its `k` cut before the permission filter.** `search_semantic`
therefore over-fetches by `_KNN_OVERFETCH` for a non-root user and trims in
Python, or a filtered vector search would quietly return fewer hits than asked
for. Root skips the predicate entirely and keeps the original plan.

**A home directory is owned by the user it is named after, not by whoever
created it** (`default_owner`). Root seeds `/home/<x>` often — the indexer, the
browser, test fixtures — and a home is 0700, so crediting the creator would lock
that user out permanently. There is no `chown`.

**`rm` checks the parent folder, not the file.** That is unix, it is what agents
expect, and it is easy to "fix" into a bug.

**SurrealQL has no bitwise operators.** The mode-bit arithmetic in
`fn::sfs_bits` and the `gate` field uses `math::floor` and `%`. Do not reach for
`&`.

## Conventions

- Async throughout. The library never owns a connection — the caller passes a
  connected `AsyncSurreal` handle. The one exception is the Hermes plugin, which
  has no caller to pass it one: Hermes hands a plugin sync handlers and no
  lifecycle hooks, so it connects per tool call from the `SURREALDB_*` env vars.
- No working directory. Every path is absolute; `cd`/`pwd` do not exist.
- Permissions are unix ones, enforced in `SurrealFs` and nowhere else. `user` is
  a required constructor argument; `ROOT` bypasses every check. Record auth is
  opt-in and adds a database-level backstop. Read `docs/permissions.md` before
  touching `owner`, `mode`, `gate` or any `fn::sfs_*`.
- The core returns dataclasses; only `tools/handlers.py` formats strings for a
  model.
- Adding a tool means one entry in `surrealfs/tools/__init__.py`, an args model,
  a handler, and a `docs/<kebab-name>.md`. All three integration surfaces pick it
  up automatically — the only hand-written list is `provides_tools` in
  `integrations/hermes/plugin.yaml`. `tests/test_tools.py` asserts every surface,
  that manifest included, stays in sync.
- Tool descriptions live in markdown, not docstrings. They are prompt text —
  write them for a model that has never seen this filesystem.
- Bind every value with parameters. The only interpolation is the table name
  (validated as an identifier) and KNN's `k`/`ef` (cast to `int`).
