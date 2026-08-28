# Semantic search

SurrealFS never calls an embedding model. You provide the function, so you pick
the provider:

```python
count = await fs.reindex_embeddings(embed, version="openai:text-embedding-3-small")
hits = await fs.search("how do I get paid", vector=await embed("how do I get paid"))
```

`fs.search` runs both arms and fuses them by rank — full-text scores are BM25 and
vector scores are distances, so there is nothing sane to normalise. Without a
`vector` it is full-text only. `search_text` and `search_semantic` remain
available if you want one arm on its own.

The full-text arm matches a file that shares **any** term with the query, so
`"how do I get paid"` returns candidates even without embeddings — the vector arm is
what makes it rank the invoicing note by _meaning_ rather than by shared words. That
default favours recall because the caller sees a ranked list of snippets and can
dismiss a weak hit at a glance, but cannot dismiss one it never saw.

Ranking is Okapi BM25, computed in Python over the tokens SurrealDB's own analyzer
produces for the query and each matching file — the same stemming the index matched
with, so a file saying "Prefers" scores for a query of "prefer" instead of scoring
zero. Rare words count for more than common ones, repetition saturates, and long
files are not rewarded for sprawl. Stopwords still match; they just get no vote.

What it cannot do is read intent: a query word that is beside the point still pulls
in files dense with it. That is the vector arm's job.

Pass `match="all"` when you want a precise filter and an empty result is a real
answer:

```python
hits = await fs.search("invoice 2026", match="all")  # both terms must appear
```

Re-running the indexer is cheap: it skips files whose content has not changed
since they were embedded. Agents get one `search` tool either way; pass the
embedder in the context and opt in to make it hybrid —
`build_fs_toolset(semantic=True)` with `ToolContext(fs=fs, embed=embed)`. See
`examples/semantic_search.py`.

An OpenAI embedder ships in `surrealfs.embed` (`make_embedder`,
`text-embedding-3-small`, the 1536 dimensions the HNSW index expects), along with
a daemon that keeps every vector current no matter what wrote the row:

```bash
just embed              # poll every 5s; --once for a single pass
```

It needs `OPENAI_API_KEY` and an already-applied schema (`just schema`). Under
Hermes there is no daemon to leave running — see [Keeping the vectors current
under Hermes](docs/hermes-indexer.md).
