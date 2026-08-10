# Hermes memory provider — what can be improved

Working behaviour in `surrealfs/integrations/hermes_memory/` with a known ceiling,
in rough value order. Everything here is open; things already settled — including the
ones deliberately left alone — are in [decisions](hermes-memory-decisions.md).

## 1. Recall passes the raw prompt to `match="any"`

`search_text` builds `content @1,OR@ $query`, so every term is an OR branch and a
chatty prompt matches nearly every file in the tree — then pulls all of their
`content` into Python for BM25. `fs.py:586` already flags that ceiling for the
search tool; recall hits it on every single turn, with a query far longer than
anything a human types into search. Strip stopwords (`_scoring_terms` already
knows them) or cap the term count before searching.

## 2. Recall reads the memory tree, which is also where it writes

Self-reinforcing loop: after a few sessions the top-5 is dominated by verbatim past
transcripts rather than the curated `/preferences/` and `/projects/` notes the
README calls the whole point. BM25 makes it worse, not better — one turn per file
means short documents, which it favours. Worst case is the current session's own
first turn outranking everything on the query that produced it.

Excluding the current session's files is the cheap version; down-weighting the
memory tree relative to hand-written notes is the real one. `_is_mine` is where both
belong — it already filters recall hits by path.

## 3. `sync_turn` gates on the user side only

`is_trivial_prompt(user_content)` drops the whole exchange, so "go ahead" followed
by a 2000-word assistant design document is never filed. Gate on the assistant
content's length as well — trivial prompt *and* short answer.

## 4. `sync_turn` accepts `messages` and ignores it

The parameter is in the signature (which is what
`_provider_sync_accepts_messages` inspects, so Hermes does pass it), but only
`user_content` and `assistant_content` are written. Tool calls and tool results —
often the substance of a turn — never reach memory. Cheap partial win: append tool
names, or the file paths touched, to the turn file.

`on_pre_compress` already renders a full message list, tool calls included, through
`_render`, so the rendering half of this exists — `sync_turn` just doesn't use it.

Worth noting the doc's caution here applies only to cloud providers: "Cloud
providers should document what parts of `messages` are sent off-device." This
writes to the user's own SurrealDB, so there is nothing to disclose — but the
README could say so explicitly, since "memory provider" reads as "cloud" to most
people now.

## 5. `_TRIVIAL_RE` duplicates a regex upstream calls its single source of truth

`agent/memory_provider.py` says of `TRIVIAL_PROMPT_RE`: "Single source of truth
shared by the core per-turn prefetch gate and provider-side classifiers so the two
can never drift apart." The fallback copy only runs outside Hermes (tests, linting,
a plain `pip install`), and the trailing-punctuation class differs — `[\W_]*`
versus an explicit set — though I found no input where they disagree. Known drift,
acceptable; the alternative is skipping the gate entirely when the import fails.
