---
title: Indexing
---

# Indexing

Indexes turn full-table scans into targeted lookups, but each one adds work to
every write. Index the fields you filter, sort, or join on — and verify the index
is actually used. See the
[DEFINE INDEX docs](https://surrealdb.com/docs/surrealql/statements/define/index).

## Index types

| Type | Definition | Use for |
| --- | --- | --- |
| Standard | `DEFINE INDEX i ON t FIELDS f` | Equality / range filters, sorting, joins |
| Unique | `DEFINE INDEX i ON t FIELDS f UNIQUE` | Enforce uniqueness + fast lookup |
| Full-text | `… FULLTEXT ANALYZER a BM25 …` | Text search with relevance scoring |
| Vector (HNSW/MTREE) | `… HNSW DIMENSION n …` | KNN / similarity search on embeddings |

```surql
-- Standard index on a filter field
DEFINE INDEX idx_email ON TABLE user FIELDS email;

-- Unique constraint (also a fast lookup index)
DEFINE INDEX idx_email_unique ON TABLE user FIELDS email UNIQUE;

-- Full-text search index (FULLTEXT; SEARCH ANALYZER is deprecated since 3.0)
DEFINE INDEX idx_title ON TABLE article
  FIELDS title
  FULLTEXT ANALYZER my_analyzer BM25(1.2, 0.75);

-- Vector index for KNN search
DEFINE INDEX idx_embedding ON TABLE document
  FIELDS embedding
  HNSW DIMENSION 384 DIST COSINE TYPE F32;
```

For full-text analyzers and BM25 see the `surrealql` skill's
[schema reference](../../surrealql/references/schema.md). For vector index tuning
(EFC, M, M0, distance functions) see the **surrealdb-vector** skill.

## Composite indexes and field order

A composite index covers queries that filter on a **leading prefix** of its
fields. Order fields from most to least selective and to match how you query:

```surql
-- Supports: filter by status; filter by status + created_at; sort within status
DEFINE INDEX idx_status_created ON TABLE order FIELDS status, created_at;
```

A query filtering only on `created_at` (the non-leading field) cannot use this
index — define a separate index if that access pattern matters.

## Verify index usage with EXPLAIN

Never assume an index is used. Confirm it:

```surql
-- Show the query plan and which index (if any) is chosen
SELECT * FROM user WHERE email = 'a@b.com' EXPLAIN;

-- Include actual execution detail
SELECT * FROM user WHERE email = 'a@b.com' EXPLAIN FULL;
```

Look for an iterator that references your index rather than a full table scan. If
the plan scans the table, the index field order, type, or the query shape (e.g.
a function wrapped around the field) is preventing index use.

## The cost of over-indexing

Every index must be updated on insert, update, and delete. Symptoms of too many
indexes: slow writes, large storage footprint, and indexes that `EXPLAIN` never
selects. Keep only indexes that back real query patterns, and remove unused ones:

```surql
REMOVE INDEX idx_unused ON TABLE user;
```

## Rebuilding indexes

Rebuild after changing analyzer settings or to recover an index:

```surql
REBUILD INDEX idx_title ON TABLE article;
```

## Checklist

1. Identify the exact filter, sort, and join fields per query.
2. Define one index per access pattern; order composite fields to match.
3. Run `EXPLAIN` to confirm the index is selected.
4. Remove indexes that no query plan uses.
