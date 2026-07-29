---
title: Schema Management
---

# Schema Management

Patterns for defining and managing schemas in SurrealDB. For full syntax, see the
[DEFINE docs](https://surrealdb.com/docs/surrealql/statements/define).

## Namespace & Database

SurrealDB organises data into namespaces and databases. Switch between them
with `USE`.

```surql
DEFINE NAMESPACE production;
DEFINE DATABASE app;

USE NS production DB app;
```

## Tables

```surql
-- Schemaless (default): accepts any fields
DEFINE TABLE post SCHEMALESS;

-- Schemafull: only explicitly defined fields are allowed
DEFINE TABLE person SCHEMAFULL;

-- Table as a view (pre-computed aggregation)
DEFINE TABLE post_stats AS SELECT
  product,
  count() AS total,
  math::mean(rating) AS avg_rating
FROM review
GROUP BY product;
```

### Permissions

```surql
DEFINE TABLE person SCHEMAFULL
  PERMISSIONS
    FOR select, create FULL
    FOR update WHERE $auth.id = id
    FOR delete WHERE $auth.role = 'admin';
```

## Fields

```surql
DEFINE FIELD name ON TABLE person TYPE string;

DEFINE FIELD age ON TABLE person TYPE int;

DEFINE FIELD email ON TABLE person TYPE string
  ASSERT string::is::email($value);

DEFINE FIELD created_at ON TABLE person TYPE datetime
  VALUE time::now()
  DEFAULT time::now();

DEFINE FIELD tags ON TABLE post TYPE option<array<string>>
  DEFAULT [];
```

## Indexes

```surql
-- Unique index
DEFINE INDEX idx_email ON TABLE person FIELDS email UNIQUE;

-- Full-text search index
DEFINE INDEX idx_title ON TABLE article
  FIELDS title
  FULLTEXT ANALYZER my_analyzer BM25(1.2, 0.75);

-- Vector index (HNSW)
DEFINE INDEX idx_embedding ON TABLE document
  FIELDS embedding
  HNSW DIMENSION 384 DIST COSINE TYPE F32;
```

### Rebuilding Indexes

Force an index to be rebuilt (e.g. after changing analyzer settings).

```surql
REBUILD INDEX idx_title ON TABLE article;
```

## Events

```surql
DEFINE EVENT log_create ON TABLE person
  WHEN $event = 'CREATE'
  THEN {
    CREATE log SET
      table = 'person',
      action = $event,
      record = $after.id,
      time = time::now();
  };
```

## Analyzers

```surql
DEFINE ANALYZER my_analyzer
  TOKENIZERS blank, class
  FILTERS lowercase, snowball(english);
```

## Functions

```surql
DEFINE FUNCTION fn::greet($name: string) {
  RETURN 'Hello, ' + $name + '!';
};

-- With typed parameters and return type
DEFINE FUNCTION fn::calculate($base: float, $multiplier: float) -> float {
  RETURN math::round($base * $multiplier, 2);
};
```

## Access (Authentication)

```surql
DEFINE ACCESS account ON DATABASE
  TYPE RECORD
  SIGNUP (
    CREATE user SET
      name = $name,
      email = $email,
      password = crypto::argon2::generate($password)
  )
  SIGNIN (
    SELECT * FROM user
    WHERE email = $email
      AND crypto::argon2::compare(password, $password)
  )
  DURATION FOR TOKEN 15m, FOR SESSION 12h;
```

## Users

```surql
DEFINE USER admin ON DATABASE PASSWORD 'secret' ROLES OWNER;
```

## Parameters

```surql
DEFINE PARAM $default_limit VALUE 50;
```

## Sequences

Auto-incrementing sequences for generating sequential IDs.

```surql
DEFINE SEQUENCE invoice_number;
```

## ALTER

Modify an existing definition without dropping and recreating it.

```surql
-- Add a comment to an existing table
ALTER TABLE person COMMENT 'Main user table';

-- Change a field type
ALTER FIELD age ON TABLE person TYPE float;
```

## Removing Definitions

```surql
REMOVE TABLE person;
REMOVE FIELD email ON TABLE person;
REMOVE INDEX idx_email ON TABLE person;
REMOVE EVENT log_create ON TABLE person;
REMOVE FUNCTION fn::greet;
REMOVE ANALYZER my_analyzer;
REMOVE ACCESS account ON DATABASE;
REMOVE USER admin ON DATABASE;
REMOVE PARAM $default_limit;
REMOVE SEQUENCE invoice_number;
```

## Inspecting Definitions

```surql
INFO FOR ROOT;
INFO FOR NS;
INFO FOR DB;
INFO FOR TABLE person;
```

## Changefeed

View recent changes to a table or database using `SHOW`.

```surql
SHOW CHANGES FOR TABLE person SINCE d'2024-01-01T00:00:00Z';
```
