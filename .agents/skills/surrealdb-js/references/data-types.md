---
title: SDK Data Types
---

# SDK Data Types

The SDK exports value classes that map 1:1 to SurrealDB's native types. They
serialize to/from the database via CBOR, so prefer them over plain strings or
numbers when a value is really a record link, duration, decimal, etc. Import any
of them from `surrealdb`.

```ts
import {
  RecordId, StringRecordId, RecordIdRange,
  Table, Duration, Decimal, Uuid, Future,
  GeometryPoint, GeometryLine, GeometryPolygon,
  Range, BoundIncluded, BoundExcluded,
} from "surrealdb";
```

## Record references

```ts
// RecordId("table", id) — id can be a string, number, array, or object
const tobie = new RecordId("person", "tobie");   // person:tobie
const item  = new RecordId("item", 42);          // item:42
const compound = new RecordId("temp", ["London", "2024-01-01"]);

tobie.tb; // "person"
tobie.id; // "tobie"
tobie.toString(); // "person:tobie"

// Parse a record id that is already in string form
const ref = new StringRecordId("person:tobie");

// A range of record ids: person:alice..=person:tobie
const range = new RecordIdRange(
  "person",
  new BoundIncluded("alice"),
  new BoundIncluded("tobie"),
);
```

Use a `RecordId` anywhere CRUD methods accept a "thing", and bind it as a query
parameter rather than interpolating `person:tobie` into the SurrealQL string.

## Table

Wraps a table name as a distinct type (vs. a plain string used as a value).

```ts
const person = new Table("person");
await db.select(person);
```

## Numbers, durations, time, ids

```ts
// Arbitrary-precision decimal (avoids float rounding)
const price = new Decimal("99.99");

// Duration — accepts a SurrealQL duration string or compact form
const ttl = new Duration("1w2d6h");

// UUID
const id = new Uuid("0193e3a1-0000-7000-8000-000000000000");

// DateTime values round-trip as native JS Date objects
const created = new Date();
```

## Geometry

```ts
const point   = new GeometryPoint([-0.118, 51.509]); // [lng, lat]
const line    = new GeometryLine([point, new GeometryPoint([2.349, 48.864])]);
const polygon = new GeometryPolygon([line]);
```

## Ranges & deferred values

```ts
// Numeric/value range: 1..=10
const r = new Range(new BoundIncluded(1), new BoundIncluded(10));

// Future — a value computed by the database when read
const f = new Future("time::now()");
```

## CBOR mapping

The SDK uses CBOR (not JSON) on the wire, which preserves these types exactly:
- `RecordId` ⇄ record link, `Table` ⇄ table name
- `Decimal` ⇄ `decimal`, `Duration` ⇄ `duration`, `Uuid` ⇄ `uuid`
- JS `Date` ⇄ `datetime`, geometry classes ⇄ `geometry`

When you read records back, these fields are returned as the corresponding class
instances — check with `instanceof` and call `.toString()` / `.toJSON()` as
needed.
