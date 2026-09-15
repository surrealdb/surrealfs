"""The SurrealFS table definitions, and a helper to apply them."""

from __future__ import annotations

from pathlib import Path
from typing import Any

__all__ = [
    "FILE_SCHEMA",
    "RECORD_AUTH_SCHEMA",
    "SSO_SCHEMA",
    "apply_schema",
    "apply_sso_access",
    "schema_sql",
]

_DIR = Path(__file__).resolve().parent

FILE_SCHEMA: str = (_DIR / "file.surql").read_text(encoding="utf-8")
"""DDL for the ``file`` table: fields, computed path, events, and indexes."""

RECORD_AUTH_SCHEMA: str = (_DIR / "record_auth.surql").read_text(encoding="utf-8")
"""Optional ``user`` table and record access, so the database enforces too."""

SSO_SCHEMA: str = (_DIR / "sso.surql").read_text(encoding="utf-8")
"""Optional ``sso`` record access, for a JWT minted from an SSO assertion.

Not part of :func:`schema_sql`: it needs a signing key, so it is applied on its
own by :func:`apply_sso_access` rather than bundled into DDL anyone may print.
"""

# RFC 7518 3.2: an HMAC key should be at least the hash output length, which for
# SHA-512 is 64 bytes. PyJWT warns below that; refusing is better than warning,
# because the whole trust chain hangs off this one string.
SSO_SECRET_BYTES = 64


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


async def apply_sso_access(db: Any, secret: str) -> None:
    """Define the ``sso`` record access, signed with ``secret``.

    Separate from :func:`apply_schema` because it takes a key: the DDL binds it
    as ``$secret`` so the deployment's signing secret never lands in a file.
    Requires the ``user`` table, so apply with ``record_auth=True`` first.
    """
    from ..fs import raise_for_status

    if len(secret.encode("utf-8")) < SSO_SECRET_BYTES:
        raise ValueError(
            f"the SSO signing secret must be at least {SSO_SECRET_BYTES} bytes "
            "(HMAC-SHA512's block size); generate one with "
            "`openssl rand -base64 48`"
        )
    raise_for_status(await db.query_raw(SSO_SCHEMA, {"secret": secret}))
