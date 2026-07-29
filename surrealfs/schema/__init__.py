"""The SurrealFS table definitions, and a helper to apply them."""

from __future__ import annotations

from pathlib import Path
from typing import Any

__all__ = ["FILE_SCHEMA", "USER_SCHEMA", "apply_schema", "schema_sql"]

_DIR = Path(__file__).resolve().parent

FILE_SCHEMA: str = (_DIR / "file.surql").read_text(encoding="utf-8")
"""DDL for the ``file`` table: fields, computed path, events, and indexes."""

USER_SCHEMA: str = (_DIR / "user.surql").read_text(encoding="utf-8")
"""Optional ``user`` table + record access, for the ``owner`` permission model."""


def schema_sql(*, include_user: bool = False) -> str:
    """The DDL that :func:`apply_schema` executes."""
    return f"{USER_SCHEMA}\n{FILE_SCHEMA}" if include_user else FILE_SCHEMA


async def apply_schema(db: Any, *, include_user: bool = False) -> None:
    """Define the SurrealFS tables on an already-connected database.

    ``db`` is an ``AsyncSurreal`` handle that is signed in and has selected a
    namespace and database. Every statement uses ``OVERWRITE`` or ``IF NOT
    EXISTS``, so this is safe to run repeatedly.

    Pass ``include_user=True`` to also define the ``user`` table and record
    access that ``file``'s ``owner = $auth.id`` permissions rely on.
    """
    # Deliberately not db.query(): the SDK returns only the first statement's
    # result and silently swallows per-statement errors, so a typo in the DDL
    # would apply partially and report success.
    from ..fs import raise_for_status

    raise_for_status(await db.query_raw(schema_sql(include_user=include_user)))
