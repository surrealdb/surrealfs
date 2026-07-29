---
title: Computed & Derived Fields
---

# Computed & Derived Fields

Recomputing the same derived value on every read is wasteful. SurrealDB offers
three ways to compute values once and read them cheaply: stored values
(`VALUE`), computed fields (`COMPUTED`), precomputed views
(`DEFINE TABLE ... AS SELECT`), and event-maintained denormalized fields
(`DEFINE EVENT`). Choose based on whether the value should be stored on write or
recomputed on read.

## Computed fields with `VALUE`

A `DEFINE FIELD ... VALUE` expression is evaluated and stored whenever the record
is written, so reads are plain field reads:

```surql
DEFINE FIELD full_name ON TABLE person
  VALUE string::concat(first_name, ' ', last_name);

DEFINE FIELD updated_at ON TABLE person TYPE datetime
  VALUE time::now();
```

Use this for values derived from the record's own fields.

## Computed fields (`COMPUTED`) — compute on read

A `COMPUTED` field is evaluated when the record is **read**, recomputing against
the current state each time. Use it when the value depends on related records
that change independently and must always be current:

```surql
DEFINE FIELD rating ON TABLE product
  COMPUTED { math::mean(SELECT VALUE stars FROM review WHERE product = $parent.id) };
```

`COMPUTED` replaces the legacy `VALUE <future> { … }` form, which is deprecated
since SurrealDB 3.0. Trade-off: it moves cost to read time. For hot read paths
over expensive computations, prefer a stored value maintained by an event
(below).

## Precomputed views with `DEFINE TABLE ... AS SELECT`

A view table stores the result of an aggregation and is maintained incrementally
as the underlying records change — ideal for dashboards and rollups that are read
far more often than the base data changes:

```surql
DEFINE TABLE product_stats AS
  SELECT
    product,
    count() AS reviews,
    math::mean(stars) AS avg_rating
  FROM review
  GROUP BY product;
```

Query `product_stats` directly instead of re-aggregating `review` on every
request.

## Event-maintained denormalized fields

When you want a **stored** value (cheap reads) that depends on other records, keep
it up to date with a `DEFINE EVENT`. This denormalizes derived data onto a record
at write time:

```surql
DEFINE EVENT update_review_count ON TABLE review
  WHEN $event = 'CREATE' OR $event = 'DELETE'
  THEN {
    UPDATE product:[$after.product ?? $before.product] SET
      review_count = count(SELECT id FROM review WHERE product = $after.product);
  };
```

## Choosing an approach

| Approach | Computed | Read cost | Use when |
| --- | --- | --- | --- |
| `VALUE` field | on write | cheap (stored) | Derived from the record's own fields |
| `COMPUTED` field | on read | recomputed each read | Must always reflect current related data; reads are infrequent |
| `AS SELECT` view | incrementally | cheap (stored) | Aggregations/rollups read more than written |
| `DEFINE EVENT` | on write | cheap (stored) | Denormalized counts/derived data depending on other records |

Rule of thumb: **read-heavy → store it** (`VALUE`, view, or event);
**read-rarely but must be live → compute it** (`COMPUTED`).
