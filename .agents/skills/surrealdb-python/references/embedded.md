---
title: Embedded SurrealDB in Python
---

## File-Based Persistent Embedded Example

```python
import asyncio
import tempfile
from pathlib import Path

from surrealdb import AsyncSurreal


async def main() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "mydb"
        db_url = f"file://{db_path}"

        # First connection: create data
        async with AsyncSurreal(db_url) as db:
            await db.use("test", "test")

            await db.create(
                "company",
                {"name": "Acme Corp", "founded": 2020, "employees": 100},
            )

            await db.create(
                "company",
                {"name": "TechStart Inc", "founded": 2021, "employees": 50},
            )

            companies = await db.select("company")
            print(f"Created companies: {companies}")

        # Second connection: verify persistence
        async with AsyncSurreal(db_url) as db:
            await db.use("test", "test")

            companies = await db.select("company")
            print(f"Loaded companies from disk: {companies}")

            updated = await db.query("""
                UPDATE company SET employees = employees + 10
                WHERE name = "Acme Corp"
            """)
            print(f"Updated company: {updated}")

            all_companies = await db.select("company")
            print(f"All companies after update: {all_companies}")


if __name__ == "__main__":
    asyncio.run(main())
```
