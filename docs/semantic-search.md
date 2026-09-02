# Semantic search

SurrealFS never calls an embedding model. You provide the function, so you pick
the provider:

```python
count = await fs.reindex_embeddings(embed, version="openai:text-embedding-3-small")
hits = await fs.search("how do I get paid", vector=await embed("how do I get paid"))
```

`reindex_embeddings` is root only: it reads every text file in the tree to embed
it, so it is the one bulk read that cannot be permission-filtered and still be an
index of the whole tree. Searching what root indexed is filtered as usual, per
caller. See [permissions.md](permissions.md).

`fs.search` runs both arms and fuses them by rank — full-text scores are BM25 and
vector scores are distances, so there is nothing sane to normalise. Without a
`vector` it is full-text only. `search_text` and `search_semantic` remain
available if you want one arm on its own.

All three are thin wrappers. The queries themselves are custom SurrealQL
functions in `surrealfs/schema/file.surql`, so any client gets them — see
[From SurrealQL](#from-surrealql) below.

The full-text arm matches a file that shares **any** term with the query, so
`"how do I get paid"` returns candidates even without embeddings — the vector arm is
what makes it rank the invoicing note by _meaning_ rather than by shared words. That
default favours recall because the caller sees a ranked list of snippets and can
dismiss a weak hit at a glance, but cannot dismiss one it never saw.

Ranking is Okapi BM25, computed by SurrealDB over the FULLTEXT index
(`search::score`). Rare words count for more than common ones, repetition
saturates, and long files are not rewarded for sprawl.

One thing to know about the numbers: SurrealDB uses the unsmoothed BM25 IDF,
clamped at zero, so **a term held by more than about half your files scores 0.0
for everything** and `path` decides the order. That is BM25 working as specified,
not a bug — a term that common cannot discriminate between documents. On a real
filesystem it rarely bites; on a three-file test corpus it is every term.

What it cannot do is read intent: a query word that is beside the point still pulls
in files dense with it. That is the vector arm's job.

Pass `match="all"` when you want a precise filter and an empty result is a real
answer:

```python
hits = await fs.search("invoice 2026", match="all")  # both terms must appear
```

## From SurrealQL

The retrieval is not Python's. It lives in the schema as three functions, so a TS
client, a `surreal sql` session or a live query gets exactly what `fs.search`
gets — ranked, fused and permission-filtered:

```surql
-- full text; $match is 'any' (default) or 'all'
fn::sfs_search_text($query, $limit, $match, $me);
-- vectors; $qvec is a 1536-dim embedding
fn::sfs_search_semantic($qvec, $limit, $me);
-- both, fused with reciprocal rank fusion into `rrf_score`
fn::sfs_hybrid_search($query, $qvec, $limit, $match, $me);
```

```surql
LET $q = "how do I get paid";
SELECT path, rrf_score FROM fn::sfs_hybrid_search($q, $qvec, 5, NONE, 'alice');
```

`$me` is who is asking, and it is not optional — passing NONE with no record
credential is an error rather than an empty result, because a silent empty is the
expensive failure for an agent. Two cases:

- **A system credential** (root, or any namespace/database user) has no identity
  the database can read, so `$me` is it. This is the same arrangement `SurrealFs`
  uses, and it is trusted for the same reason: whoever holds that credential can
  read the table directly anyway.
- **A record credential** ([record auth](permissions.md)) *is* the identity, and
  it wins — `$me` is ignored entirely, so it cannot be used to search as somebody
  else.

Every row comes back with the full file, `content` included, and no snippet:
`search::highlight` and `search::offsets` are broken on SurrealDB 3.2.x, so cut
your own window on the client. `fn::sfs_search_text` also returns each row's BM25
`score`, and `fn::sfs_search_semantic` its cosine `distance`; only `rrf_score`
from the hybrid function is comparable across the whole result set.

Vectors still have to come from somewhere — the functions do not embed either.
Keep them current with `just embed`, then pass a query embedding from the same
model.

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
