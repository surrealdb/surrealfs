---
name: surrealdb-python
description: "Using SurrealDB with the Python SDK, covering both client/server mode (WebSocket) and embedded mode (in-memory or file-based). Use when connecting to SurrealDB from Python, using the surrealdb Python package, running SurrealDB embedded without a server, or performing CRUD operations from Python code. Triggers: surrealdb Python, Surreal(), AsyncSurreal(), Python SDK, embedded SurrealDB, mem://, file://."
metadata:
  author: surrealdb
  version: "0.1.0"
---

# SurrealDB Python SDK

## Running SurrealDB (Server Mode)

Persist data with RocksDB:

```bash
surreal start -u root -p root rocksdb:database
```

In-memory:

```bash
surreal start -u root -p root
```

## Client/Server Mode

Connect via WebSocket using the `Surreal` context manager:

```python
from surrealdb import Surreal

with Surreal("ws://localhost:8000/rpc") as db:
    db.signin({"username": "root", "password": "root"})
    db.use("namespace_test", "database_test")

    db.create(
        "person",
        {
            "user": "me",
            "password": "safe",
            "marketing": True,
            "tags": ["python", "documentation"],
        },
    )

    print(db.select("person"))

    print(db.update("person", {
        "user": "you",
        "password": "very_safe",
        "marketing": False,
        "tags": ["Awesome"],
    }))

    print(db.delete("person"))

    db.query("""
    insert into person {
        user: 'me',
        password: 'very_safe',
        tags: ['python', 'documentation']
    };
    """)

    print(db.query("select * from person"))

    print(db.query("""
    update person content {
        user: 'you',
        password: 'more_safe',
        tags: ['awesome']
    };
    """))

    print(db.query("delete person"))
```

## Embedded Mode

Run SurrealDB directly inside your Python process — no server required.

- **In-memory**: `Surreal("mem://")` / `AsyncSurreal("mem://")`
- **File-based persistence**: `Surreal(f"file://{db_path}")` / `AsyncSurreal(f"file://{db_path}")`

See [references/embedded.md](references/embedded.md) for a complete async
example with file-based persistence.
