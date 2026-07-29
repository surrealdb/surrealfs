---
title: Record ID & Key Structuring
---

# Record ID & Key Structuring

Record IDs in SurrealDB are not just identifiers — they are the primary physical
key, stored in **sorted order**. Designing IDs around your access patterns is the
single highest-leverage performance decision, because it controls data locality
and whether range queries can avoid full-table scans. See the
[Record IDs docs](https://surrealdb.com/docs/surrealql/datamodel/ids).

## A record ID is `table:identifier`

```surql
person:surrealdb          -- string identifier
person:17493              -- integer identifier (64-bit)
person:⟨complex id⟩       -- use ⟨…⟩ (or backticks) to escape unusual identifiers
```

IDs sort in a natural order, so records that share a prefix are stored near each
other. Lookups by full ID are direct key reads — the cheapest possible access.

## ID generation strategies

| Generator | Syntax | When to use |
| --- | --- | --- |
| Random (default) | `CREATE temperature:rand()` | No ordering needed; avoids hot keys |
| ULID | `CREATE temperature:ulid()` | Time-sortable, insert-ordered, good locality by time |
| UUID v7 | `CREATE temperature:uuid()` | Time-ordered UUIDs for interop |
| Numeric / string | `CREATE temperature:17493` | Natural keys or externally supplied IDs |
| Sequence | see below | Strict monotonic counters (invoice numbers, etc.) |

Time-sortable IDs (ULID, UUID v7) keep recently-created records together, which
helps time-range scans and pagination. Purely random IDs spread writes out,
avoiding write hot spots but giving no useful range locality.

## Compound (array) IDs for locality and range scans

Array IDs are the key tool for high-performance range queries. Order components
from **coarsest to finest** so the prefix you filter on comes first:

```surql
-- Group readings by city, then time: all London readings sit together,
-- ordered by timestamp.
CREATE weather:['London', d'2025-02-13T05:00:00Z'] SET temperature = 5.7;
CREATE weather:['London', d'2025-02-13T06:00:00Z'] SET temperature = 6.1;
```

## Record ranges

Range queries use ID ordering directly instead of scanning and filtering:

```surql
-- Inclusive numeric range
SELECT * FROM person:1..=1000;

-- All London readings within a time window (prefix + sub-range)
SELECT * FROM weather:['London', d'2025-02-13T00:00:00Z']
                  ..['London', d'2025-02-14T00:00:00Z'];

-- Open-ended bounds
SELECT * FROM person:1000..;
SELECT * FROM person:..=1000;
```

Prefer a record range over `SELECT ... WHERE id > …`: the range walks the sorted
key space, while a `WHERE` filter on a scanned set does not benefit from ID
ordering.

## Avoiding hot keys

Monotonic IDs concentrate every new write at the "end" of the key space, which
can become a write bottleneck under heavy ingest. Trade-off:

- **Need range/time locality** → ULID / UUID v7 / time-prefixed compound IDs.
- **Need maximum write throughput, no range needs** → `rand()`, or put a
  higher-cardinality component (e.g. a shard or tenant id) first in a compound ID
  to spread writes.

## Sequences for monotonic counters

When you need a strict, gap-aware incrementing number (invoice numbers, order
numbers), use a sequence rather than counting rows:

```surql
DEFINE SEQUENCE invoice_number;

CREATE invoice SET number = sequence::nextval('invoice_number');
```

Sequences are designed for concurrent use and avoid the race conditions of
`SELECT count()`-based numbering.
