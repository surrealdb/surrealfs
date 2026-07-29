---
name: surrealql-performance
description: "Optimize SurrealDB performance through record ID and key design, indexing strategy, and computed/derived fields. Use when queries are slow, when designing record IDs for locality and range scans, choosing between standard/unique/full-text/vector indexes, verifying index usage with EXPLAIN, or deciding whether to precompute values with computed fields, views, or events. Triggers: slow query, performance, record id design, range scan, DEFINE INDEX, EXPLAIN, computed field, FUTURE, DEFINE TABLE AS SELECT."
metadata:
  author: surrealdb
  version: "0.1.0"
---

# SurrealQL Performance

Techniques for making SurrealDB queries fast: structuring record IDs and keys
for data locality, choosing and verifying the right indexes, and precomputing
values with computed fields and views instead of recomputing them on every read.

Target the **latest** stable SurrealDB release. Confirm version-sensitive syntax
(record ranges, `COMPUTED` fields, `FULLTEXT` index options) against
`https://surrealdb.com/docs`, and validate examples with `surreal validate`. See
the `surrealql` skill for version detection.

## When to use this skill

- A query is slow or scans more records than expected
- Designing record IDs to support efficient lookups and range scans
- Choosing between standard, `UNIQUE`, full-text `SEARCH`, or vector indexes
- Confirming whether an index is actually used (`EXPLAIN`)
- Deciding whether to store a derived value vs. compute it on read

## Topic map

| Topic | Reference |
| --- | --- |
| Record ID & key structuring for locality and range scans | [references/keys.md](references/keys.md) |
| Index types, composite order, verifying usage, rebuild cost | [references/indexing.md](references/indexing.md) |
| Computed fields, precomputed views, event-maintained values | [references/computed-fields.md](references/computed-fields.md) |

## Top rules

- **Design IDs for access patterns.** Record IDs are stored in sorted order. Put
  the most selective, range-friendly component first (e.g.
  `weather:['London', d'2025-02-13T05:00Z']`) so related records sit together and
  range queries avoid full-table scans. See [references/keys.md](references/keys.md).
- **Prefer record ranges over `WHERE` on the ID.** `SELECT * FROM person:1..1000`
  uses key ordering directly; filtering with `WHERE` after a full scan does not.
- **Index the fields you filter, sort, or join on — but no more.** Every index
  adds write cost. Order composite index fields from most to least selective and
  to match your query's filter/sort order. See [references/indexing.md](references/indexing.md).
- **Verify, don't assume.** Run `EXPLAIN` (or `EXPLAIN FULL`) to confirm a query
  uses the index you expect before concluding it is optimized.
- **Precompute expensive, read-heavy values.** Use computed fields, a
  `DEFINE TABLE ... AS SELECT` view, or a `DEFINE EVENT` to maintain derived data
  rather than recomputing aggregates on every query. See
  [references/computed-fields.md](references/computed-fields.md).
- **Use bound parameters.** Parameterized queries are safer and let the engine
  reuse query plans.
