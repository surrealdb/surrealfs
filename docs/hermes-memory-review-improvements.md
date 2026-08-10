# Hermes memory provider — what can be improved

Working behaviour in `surrealfs/integrations/hermes_memory/` with a known ceiling,
in rough value order. See also [missing](hermes-memory-review-missing.md) and
[decisions](hermes-memory-decisions.md).

## 1. `prefetch` runs the whole retrieval inline, and the cost is understated

The existing `ponytail:` note argues the fan-out isn't worth caching because
"Hermes already threads this call and gives it 8s, and a local search answers in
milliseconds". Two things it doesn't account for:

- With `SURREALFS_SEMANTIC=1`, `make_embedder()(query)` puts a third-party API
  round-trip on the critical path of every turn, on top of the WebSocket connect,
  the once-per-process `apply_schema`, and the search itself. "Milliseconds" holds
  only for the non-semantic path.
- `MemoryManager._prefetch_provider` records the daemon thread per provider and
  **skips recall entirely on the next turn** if the previous call is still running
  ("prefetch is still running; skipping this turn"). So one slow turn costs recall
  on the following turn too, silently.

The ABC's own shape — `queue_prefetch` after each turn, serve the cache in
`prefetch` — costs about ten lines and removes both. Blocking in `sync_turn`
remains fine and the class docstring's reasoning there is correct: `sync_all`
dispatches to a serialized daemon worker precisely so a provider can block
(`memory_manager.py:638`).

## 2. Recall passes the raw prompt to `match="any"`

`search_text` builds `content @1,OR@ $query`, so every term is an OR branch and a
chatty prompt matches nearly every file in the tree — then pulls all of their
`content` into Python for BM25. `fs.py:586` already flags that ceiling for the
search tool; recall hits it on every single turn, with a query far longer than
anything a human types into search. Strip stopwords (`_scoring_terms` already
knows them) or cap the term count before searching.

## 3. Recall reads the memory tree, which is also where it writes

Self-reinforcing loop: after a few sessions the top-5 is dominated by verbatim past
transcripts rather than the curated `/preferences/` and `/projects/` notes the
README calls the whole point. BM25 makes it worse, not better — one turn per file
means short documents, which it favours. Worst case is the current session's own
first turn outranking everything on the query that produced it.

Excluding the current session's files is the cheap version; down-weighting the
memory tree relative to hand-written notes is the real one. `_is_mine` is where both
belong — it already filters recall hits by path.

## 4. `sync_turn` gates on the user side only

`is_trivial_prompt(user_content)` drops the whole exchange, so "go ahead" followed
by a 2000-word assistant design document is never filed. Gate on the assistant
content's length as well — trivial prompt *and* short answer.

## 5. `messages` is accepted and ignored

The parameter is in the signature (which is what
`_provider_sync_accepts_messages` inspects, so Hermes does pass it), but only
`user_content` and `assistant_content` are written. Tool calls and tool results —
often the substance of a turn — never reach memory. Cheap partial win: append tool
names, or the file paths touched, to the turn file.

Worth noting the doc's caution here applies only to cloud providers: "Cloud
providers should document what parts of `messages` are sent off-device." This
writes to the user's own SurrealDB, so there is nothing to disclose — but the
README could say so explicitly, since "memory provider" reads as "cloud" to most
people now.

## 6. `_TRIVIAL_RE` duplicates a regex upstream calls its single source of truth

`agent/memory_provider.py` says of `TRIVIAL_PROMPT_RE`: "Single source of truth
shared by the core per-turn prefetch gate and provider-side classifiers so the two
can never drift apart." The fallback copy only runs outside Hermes (tests, linting,
a plain `pip install`), and the trailing-punctuation class differs — `[\W_]*`
versus an explicit set — though I found no input where they disagree. Known drift,
acceptable; the alternative is skipping the gate entirely when the import fails.

## 7. Provider name is coupled to the install directory name

`PROVIDER_NAME = "surrealfs-memory"` must match whatever the user names the
symlink, because Hermes matches `memory.provider` against the *directory* name
while `MemoryManager` reports the *property*. Both READMEs say so, which is the
right mitigation, but a mismatch produces a confusing failure. Nothing to fix in
code — noted so the next reader doesn't try to make the name dynamic.

## Things that are right and shouldn't be "improved"

Recorded so a later pass doesn't undo them:

- **One connection per call** (`_connect.connected`). The loop-affinity reasoning
  holds: `prefetch` runs on a fresh daemon thread per turn, `sync_turn` on the
  memory worker, and a SurrealDB WebSocket belongs to the loop that opened it.
- **`get_tool_schemas()` returning `[]`.** A second toolset would collide in
  Hermes' flat namespace, and `add_provider` reserves core tool names anyway.
- **The `MemoryProvider`-string guard** in `tests/test_memory_provider.py`. Verified
  against `hermes_cli/plugins.py:1749-1767` — the text scan and the coercion to
  `kind="exclusive"` are real, and a mention in a comment really would unload the
  fourteen tools.
- **"Memory providers are found by directory, never by entry point"** and
  "`PluginContext` has no `register_memory_provider`". Both verified;
  `plugins/memory/__init__.py:_ProviderCollector` is what calls `register()`.
