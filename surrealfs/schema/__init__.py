"""The SurrealFS table definitions, and a helper to apply them."""

from __future__ import annotations

from pathlib import Path
from typing import Any

__all__ = ["FILE_SCHEMA", "RECORD_AUTH_SCHEMA", "apply_schema", "schema_sql"]

_DIR = Path(__file__).resolve().parent

FILE_SCHEMA: str = (_DIR / "file.surql").read_text(encoding="utf-8")
"""DDL for the ``file`` table: fields, computed path, events, and indexes."""

RECORD_AUTH_SCHEMA: str = (_DIR / "record_auth.surql").read_text(encoding="utf-8")
"""Optional ``user`` table and record access, so the database enforces too."""


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
