---
title: Data Querying Patterns
---

# Data Querying Patterns

Common SurrealQL patterns for querying and mutating data. For full statement
syntax, see the [statements docs](https://surrealdb.com/docs/surrealql/statements).

## SELECT

```surql
-- All records
SELECT * FROM person;

-- Specific fields
SELECT name, age FROM person;

-- By record ID
SELECT * FROM person:john;
```

### Filtering

```surql
SELECT * FROM person WHERE age > 25;

SELECT * FROM person
WHERE age > 18
  AND email IS NOT NONE
  AND address.country = 'NL';

SELECT * FROM person WHERE tags CONTAINS 'developer';
```

### Ordering, Limits, and Pagination

```surql
SELECT * FROM person ORDER BY name ASC LIMIT 10;

SELECT * FROM person ORDER BY created_at DESC LIMIT 20 START AT 40;
```

### Aggregation

```surql
SELECT count() FROM person GROUP BY city;

SELECT count(), math::mean(age) AS avg_age FROM person GROUP BY country;
```

### Subqueries

```surql
SELECT * FROM person WHERE id IN (SELECT VALUE author FROM article);

SELECT
  name,
  (SELECT VALUE count() FROM ->likes) AS like_count
FROM person;
```

### SELECT VALUE

Return flat values instead of records.

```surql
SELECT VALUE name FROM person WHERE age > 21;
```

### SPLIT

Expand array fields into separate rows.

```surql
SELECT * FROM person SPLIT ON tags;
```

### FETCH

Resolve record links inline.

```surql
SELECT * FROM article FETCH author, comments;
```

## CREATE

Create new records. Errors if the record already exists.

```surql
-- Auto-generated ID
CREATE person CONTENT { name: 'Alice', age: 30 };

-- Specific record ID
CREATE person:alice CONTENT { name: 'Alice', age: 30 };

-- Using SET syntax
CREATE person SET name = 'Alice', age = 30;

-- Return control
CREATE person:bob CONTENT { name: 'Bob' } RETURN AFTER;
```

## INSERT

Insert one or more records. Supports `ON DUPLICATE KEY UPDATE` for upsert
behaviour based on unique indexes.

```surql
-- Single record
INSERT INTO person { name: 'Alice', age: 30 };

-- Bulk insert
INSERT INTO person [
  { name: 'Alice', age: 30 },
  { name: 'Bob', age: 25 },
];

-- Upsert on duplicate key
INSERT INTO person { name: 'Alice', age: 31 }
ON DUPLICATE KEY UPDATE
  age = $input.age;

-- Insert graph edges
INSERT RELATION INTO knows {
  in: person:alice,
  out: person:bob,
};
```

## UPDATE

Update existing records. No-op if the record doesn't exist (use UPSERT to
create-if-missing).

```surql
-- Replace entire content
UPDATE person:alice CONTENT { name: 'Alice', age: 31, city: 'Amsterdam' };

-- Merge with existing data
UPDATE person:alice MERGE { age: 31, city: 'Amsterdam' };

-- Set individual fields
UPDATE person:alice SET age = 31, city = 'Amsterdam';

-- Unset fields
UPDATE person:alice UNSET city;

-- JSON Patch
UPDATE person:alice PATCH [
  { op: 'replace', path: '/age', value: 31 },
];

-- Conditional bulk update
UPDATE person SET age += 1 WHERE age < 30;

-- Return control
UPDATE person:alice SET age = 31 RETURN DIFF;
```

## UPSERT

Insert a record if it doesn't exist, update it if it does.

```surql
UPSERT person:alice SET name = 'Alice', age = 31;

UPSERT person:alice MERGE { settings: { theme: 'dark' } };

-- With WHERE (only updates if condition matches)
UPSERT person:alice SET name = 'Alice' WHERE active = true;
```

## DELETE

```surql
-- Specific record
DELETE person:alice;

-- All records in a table
DELETE person;

-- Conditional
DELETE person WHERE active = false;

-- Return deleted records
DELETE person:alice RETURN BEFORE;

-- Delete graph edges
DELETE person:alice->knows WHERE out = person:bob;
```

## RELATE

Create graph edges between records. Edge tables can hold additional data.

```surql
-- Simple relationship
RELATE person:alice -> knows -> person:bob;

-- With edge data
RELATE person:alice -> wrote -> article:intro CONTENT {
  date: d'2024-01-15',
  word_count: 1500,
};

-- Using SET
RELATE person:alice -> likes -> post:123 SET
  created_at = time::now(),
  reaction = 'love';
```

## Graph Traversal

```surql
-- Outbound: people that alice knows
SELECT ->knows->person FROM person:alice;

-- Inbound: people who know bob
SELECT <-knows<-person FROM person:bob;

-- Chained traversal
SELECT ->wrote->article->has->category.name FROM person:alice;

-- Filter on traversal
SELECT * FROM person WHERE ->knows->person.age > 30;
```

## LIVE SELECT

Stream real-time changes. Returns a UUID identifying the live query.

```surql
LIVE SELECT * FROM person;

LIVE SELECT * FROM person WHERE active = true;

-- Receive diffs instead of full records
LIVE SELECT DIFF FROM person;
```

Cancel with `KILL <uuid>`.

## Transactions

Wrap multiple statements in a transaction for atomicity.

```surql
BEGIN TRANSACTION;
  UPDATE account:alice SET balance -= 100;
  UPDATE account:bob SET balance += 100;
  RELATE account:alice -> transfer -> account:bob SET
    amount = 100,
    timestamp = time::now();
COMMIT TRANSACTION;

-- Cancel instead of committing
BEGIN TRANSACTION;
  UPDATE account:alice SET balance -= 100;
  IF $error {
    CANCEL TRANSACTION;
  };
COMMIT TRANSACTION;
```

## FOR Loops

```surql
FOR $item IN $items {
  CREATE item SET
    name = $item.name,
    value = $item.value;
};

-- With BREAK / CONTINUE
FOR $user IN (SELECT * FROM user) {
  IF $user.role = 'bot' {
    CONTINUE;
  };
  CREATE notification SET target = $user.id;
};
```

## RETURN

```surql
-- Return a literal
RETURN 42;

-- Return a query result
RETURN SELECT * FROM person WHERE age > 18;

-- Return from a block
{
  LET $x = 10;
  LET $y = 20;
  RETURN $x + $y;
};
```

## Parameterized Queries

Use parameters to prevent injection and improve readability.

```surql
LET $min_age = 18;
SELECT * FROM person WHERE age > $min_age;

-- Safely parameterize table names
SELECT * FROM type::table($table);
```
