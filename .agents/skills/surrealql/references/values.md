---
title: SurrealQL Values
---

# SurrealQL Values

In SurrealDB, values represent the data that can be stored in records. These values encompass a wide variety of types allowing for rich data modeling and flexible expressions within SurrealQL queries. Understanding the different value types and their semantics is crucial for designing schemas, constructing queries, and ensuring accurate data representation in SurrealDB.

The syntax used to represent values in SurrealQL is known as "SurrealQL Object Notation" - or "SQON" for short.

## Data types

### None

`none` represents the explicit absence of a value. It is distinct from `null` and is used to indicate that a field has no value at all. Responses typically omit `none`-valued fields entirely rather than including them.

**Example SQON:**  
```
NONE
```

### Null

`null` represents an unknown or undefined value. While semantically similar to `none`, `null` conveys "value is unknown" rather than "value is absent".

**Example SQON:**  
```
NULL
```

### Bool

A boolean value: `true` or `false`.

**Example SQON:**  
```
true
false
```

### Number

SurrealDB supports three numeric subtypes, all of which fall under the umbrella `number` type:

| Subtype   | Storage                    | Range / precision                                   | SQON syntax           |
|-----------|----------------------------|-----------------------------------------------------|-----------------------|
| `int`     | 64-bit signed integer      | −9,223,372,036,854,775,808 to 9,223,372,036,854,775,807 | `42`                  |
| `float`   | 64-bit IEEE 754 double     | ≈15–17 significant decimal digits                   | `3.14` or `3.14f`     |
| `decimal` | 128-bit decimal floating point | Arbitrary precision, no IEEE 754 rounding        | `3.14dec`             |

A numeric literal without a decimal point and within the `int` range is stored as an `int`. A literal with a decimal point or outside the `int` range is stored as a `float`.

Underscores in numeric literals are ignored and can be used for readability (e.g. `1_000_000`).

**Example SQON:**  
```
42                  -- Int
3.14f               -- Float
3.14159265358979dec -- Decimal
```

### Duration

A non-negative time span with nanosecond precision. Durations are composed of one or more unit segments:

| Unit   | Meaning       |
|--------|--------------|
| `ns`   | Nanoseconds  |
| `us` / `µs` | Microseconds  |
| `ms`   | Milliseconds |
| `s`    | Seconds      |
| `m`    | Minutes      |
| `h`    | Hours        |
| `d`    | Days         |
| `w`    | Weeks        |
| `y`    | Years        |

Units can be combined in a single literal: `1y2w3d4h5m6s7ms8us9ns`. A duration can be zero (`0ns`) but cannot be negative.

**Example SQON:**  
```
1h30m
2w3d
100ms
```

### String

A UTF-8 encoded text value of arbitrary length. Strings can contain Unicode characters, emojis, tabs, and line breaks. You can
prefix strings with certain characters to indicate their type (e.g. datetime, uuid, file).

**Example SQON:**  
```
'hello'
"hello"
```

### Datetime

An RFC 3339 / ISO 8601 timestamp with nanosecond precision. Datetimes are stored internally as UTC; a timezone offset in the input is converted to UTC on storage.

**Example SQON:**  
```
d"2024-01-15T09:30:00Z"
```

### UUID

A universally unique identifier conforming to RFC 4122. SurrealDB supports UUID v4 (random) and v7 (time-ordered).

**Example SQON:**  
```
u"01924b3c-f1a2-7e3d-a001-2f4b8c9d0e1f"
```

### Array

An ordered, indexed collection of values. Arrays may contain values of any type, including nested arrays and objects. Individual elements are accessed by zero-based index. An optional element type and length constraint can be specified in schema definitions (e.g. `array<string, 5>`).

**Example SQON:**  
```
[1, 2, 3]
```

### Set

An ordered, automatically deduplicated collection of values. Sets differ from arrays in two ways: duplicate values are removed, and values are sorted. Sets support the same element type and length constraints as arrays in schema definitions.

**Example SQON:**  
```
{,}       -- Empty set (comma is required)
{1,}      -- Set with one element (trailing comma is required)
{1, 2, 3} -- Set with many elements
```

### Object

An unordered key-value map with string keys and values of any type. Objects may be nested and can contain any other value type.
The quoting of keys is optional and only required if the key contains special characters or is a reserved word.

**Example SQON:**  
```
{ name: 'Jane', age: 30 }
```

### Geometry

A geospatial value conforming to RFC 7946 (GeoJSON). SurrealDB supports the following geometry subtypes:

| Subtype             | Description                                            |
|---------------------|-------------------------------------------------------|
| `Point`             | A single position (longitude, latitude)               |
| `LineString`        | An ordered sequence of positions                      |
| `Polygon`           | A closed shape with an exterior ring and optional interior rings (holes) |
| `MultiPoint`        | A collection of points                                |
| `MultiLineString`   | A collection of line strings                          |
| `MultiPolygon`      | A collection of polygons                              |
| `GeometryCollection`| A heterogeneous collection of geometry objects        |

**Example SQON:**  
```
{ type: "Point", coordinates: [-122.4194, 37.7749] } -- Regular GeoJSON object
(-122.4194, 37.7749)                                 -- Special shorthand notation for Point
```

### Bytes

Raw binary data. Bytes are typically displayed in hexadecimal encoding.

**Example SQON:**  
```
b"48656C6C6F"
```

### Record ID

A record ID uniquely identifies a single record within a table. It is composed of two parts: a **table name** and an **identifier**.

The identifier can take several forms:

| Form                   | Example (SQON)                       |
|------------------------|--------------------------------------|
| Text                   | `user:tobie`                         |
| Numeric (64-bit int)   | `user:42`                            |
| UUID                   | `user:u"01924b3c-f1a2-7e3d-a001-2f4b8c9d0e1f"` |
| Array (composite key)  | `temperature:['London', d'2025-02-13']` |
| Object (structured key)| `user:{ name: 'john', age: 30 }`     |
| Generated              | `user:rand()`, `user:ulid()`, `user:uuid()` |
| Range                  | `temperature:['London', '2022-08-29T08:03:39']..['London', '2022-08-29T08:09:31']` |

Record IDs are immutable and double as record links — holding a record ID is sufficient to traverse to another record's data.

**Example SQON:**  
```
user:abc123
user:42
user:u"01924b3c-f1a2-7e3d-a001-2f4b8c9d0e1f"
user:{ name: 'john', age: 30 }
temperature:['London', d'2025-02-13']
```

### File

A reference to a file in a storage bucket.

**Example SQON:**  
```
f"bucket:/path/to/file.txt"
```

### Range

A bounded or unbounded range of values. Ranges are composed of the `..` operator with optional lower and upper bounds:

| Syntax      | Meaning                                 |
|-------------|-----------------------------------------|
| `a..b`      | From `a` (inclusive) to `b` (exclusive) |
| `a..=b`     | From `a` (inclusive) to `b` (inclusive) |
| `a>..b`     | From `a` (exclusive) to `b` (exclusive) |
| `a>..=b`    | From `a` (exclusive) to `b` (inclusive) |
| `a..`       | From `a` (inclusive), unbounded above   |
| `..b`       | Unbounded below, to `b` (exclusive)     |
| `..`        | Fully unbounded (infinite range)        |

Ranges can be constructed from any value type supporting comparison.

**Example SQON:**  
```
0..10 -- From 0 (inclusive) to 10 (exclusive)
0..=10 -- From 0 (inclusive) to 10 (inclusive)
0>..10 -- From 0 (exclusive) to 10 (exclusive)
0>..=10 -- From 0 (exclusive) to 10 (inclusive)
0.. -- From 0 (inclusive), unbounded above
..10 -- Unbounded below, to 10 (exclusive)
.. -- Fully unbounded (infinite range)
```
