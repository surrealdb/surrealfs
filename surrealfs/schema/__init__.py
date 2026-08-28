"""The SurrealFS table definitions, and a helper to apply them."""

from __future__ import annotations

from pathlib import Path
from typing import Any

__all__ = [
    "FILE_SCHEMA",
    "MIGRATE_PERMISSIONS",
    "RECORD_AUTH_SCHEMA",
    "apply_schema",
    "migrate",
    "schema_sql",
]

_DIR = Path(__file__).resolve().parent

FILE_SCHEMA: str = (_DIR / "file.surql").read_text(encoding="utf-8")
"""DDL for the ``file`` table: fields, computed path, events, and indexes."""

RECORD_AUTH_SCHEMA: str = (_DIR / "record_auth.surql").read_text(encoding="utf-8")
"""Optional ``user`` table and record access, so the database enforces too."""

MIGRATE_PERMISSIONS: str = (_DIR / "migrate_permissions.surql").read_text(
    encoding="utf-8"
)
"""One-off backfill for databases holding rows older than ``owner``/``mode``."""


def schema_sql(*, record_auth: bool = False) -> str:
    """The DDL that :func:`apply_schema` executes."""
    return f"{FILE_SCHEMA}\n{RECORD_AUTH_SCHEMA}" if record_auth else FILE_SCHEMA


async def apply_schema(db: Any, *, record_auth: bool = False) -> None:
    """Define the SurrealFS tables on an already-connected database.

    ``db`` is an ``AsyncSurreal`` handle that is signed in and has selected a
    namespace and database. Every statement uses ``OVERWRITE`` or ``IF NOT
    EXISTS``, so this is safe to run repeatedly.
    """
    # Deliberately not db.query(): the SDK returns only the first statement's
    # result and silently swallows per-statement errors, so a typo in the DDL
    # would apply partially and report success.
    from ..fs import raise_for_status

    raise_for_status(await db.query_raw(schema_sql(record_auth=record_auth)))


async def migrate(db: Any) -> None:
    """Backfill an existing database for the permissions schema.

    Run once, after :func:`apply_schema`, on a database that holds rows written
    before ``owner`` and ``mode`` existed. Until it has run, every read of those
    rows fails: ``gate`` is computed from the parent's mode, and they have none.

    A fresh database does not need this, and running it there does nothing.
    It is safe to re-run; see ``migrate_permissions.surql`` for what each step
    guards against.
    """
    from ..fs import raise_for_status

    raise_for_status(await db.query_raw(MIGRATE_PERMISSIONS))
