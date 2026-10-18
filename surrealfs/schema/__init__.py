"""The SurrealFS table definitions, and a helper to apply them."""

from __future__ import annotations

from pathlib import Path
from typing import Any

__all__ = [
    "FILE_SCHEMA",
    "RECORD_AUTH_SCHEMA",
    "apply_schema",
    "gc_chunks",
    "schema_sql",
]

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


async def gc_chunks(db: Any, *, grace: int = 3600) -> int:
    """Delete chunks that no file points at, and return how many went.

    A chunked write stages its chunks before the one transaction that attaches
    them, so a process killed in between leaves chunks no ``file.blob`` names.
    ``grace`` is the age in seconds below which a chunk is left alone: a
    chunk newer than that may belong to an upload still running, and deleting
    it would make that upload commit a file with its content missing. Needs a
    system credential, which the table's permissions do not constrain.
    """
    from ..fs import raise_for_status

    raw = await db.query_raw(
        "LET $live = (SELECT VALUE blob FROM file WHERE blob IS NOT NONE);"
        "LET $dead = (SELECT VALUE id FROM file_chunk WHERE blob NOT IN $live "
        "AND created_at < time::now() - duration::from_secs($grace));"
        "DELETE $dead RETURN NONE;"
        "RETURN array::len($dead);",
        {"grace": grace},
    )
    return int(raise_for_status(raw)[-1])
