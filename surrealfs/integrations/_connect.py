"""One connection, opened from the environment, for surfaces that host themselves.

Most of this library never connects: the caller owns the handle and passes it in.
The exceptions are the surfaces that have no caller to hand them one — the Hermes
plugin and memory provider, which Hermes gives a sync callback and no lifecycle
hook, and the file browser, which is a program rather than a library.

    async with connected() as db:        # short-lived, one call
        await SurrealFs(db, user=agent_user()).ls("/")

    db = await connect()                 # long-lived, survives an idle socket
    try:
        await SurrealFs(db, user=agent_user()).ls("/")
    finally:
        await db.close()

Connection details come from the same ``SURREALDB_*`` variables the rest of the
project uses, defaulting to a local server.
"""

from __future__ import annotations

import asyncio
import getpass
import os
import re
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any

from surrealdb import AsyncSurreal
from surrealdb.errors import ConnectionUnavailableError
from websockets.exceptions import ConnectionClosed

from ..schema import apply_schema

__all__ = ["agent_user", "connect", "connected"]

# Which kind of user the credentials belong to. All three are *system* users, so
# they bypass table permissions entirely. That is deliberate, and it is why
# SurrealFS enforces its own: the identity a `SurrealFs` acts as comes from
# `agent_user()` below, not from the database. See `docs/permissions.md`.
AUTH_LEVELS = ("root", "namespace", "database")

_schema_applied = False

# No dot: a name is a single path segment, and `hermes/../martin` must not
# slug into anything containing `..`.
_UNSAFE_IN_NAME = re.compile(r"[^A-Za-z0-9_-]+")


def _machine_user() -> str:
    """The unix account this process runs as, or ``unknown``.

    `getpass.getuser` walks LOGNAME/USER/LNAME/USERNAME and then the password
    database, and raises only when every one of them fails -- a container with no
    passwd entry, some cron environments.
    """
    try:
        return _slug(getpass.getuser())
    except Exception:
        return "unknown"


def _slug(name: str) -> str:
    return _UNSAFE_IN_NAME.sub("-", name).strip("-") or "unknown"


def agent_user() -> str:
    """The user an agent surface acts as, and whose home it owns.

    The agent, not the human: the notes skill hands the agent a home at
    ``/home/<name>/``, so two agents owned by two different people collide there
    unless the name distinguishes them.

    Defaults to the machine username, because on a VM or sandbox the agent has an
    account of its own and the SurrealFS path should match it. Where it does not --
    an agent sharing a laptop account with its human, or two agents under one
    account -- ``SURREALFS_AGENT_USER`` names it (``hermes-martin``), and
    ``hermes memory setup`` writes that into the profile's own ``.env``.

    This is now an access-control boundary, not just a path convention: whatever
    it returns is the only home this process can read or write.
    """
    name = os.environ.get("SURREALFS_AGENT_USER", "").strip()
    return _slug(name) if name else _machine_user()


def _namespace() -> str:
    return os.environ.get("SURREALDB_NAMESPACE", "surrealfs")


def _database() -> str:
    return os.environ.get("SURREALDB_DATABASE", "demo")


def _credentials() -> dict[str, str]:
    """The signin payload for the configured auth level.

    The server infers the level from which keys are present -- the SDK forwards
    this dict to the signin RPC untouched -- so a root credential must *not*
    carry a namespace, and a database user must carry both.
    """
    level = os.environ.get("SURREALDB_AUTH_LEVEL", "root").strip().lower()
    if level not in AUTH_LEVELS:
        raise ValueError(
            f"SURREALDB_AUTH_LEVEL must be one of {', '.join(AUTH_LEVELS)}: {level!r}"
        )
    creds = {
        "username": os.environ.get("SURREALDB_USER", "root"),
        "password": os.environ.get("SURREALDB_PASS", "root"),
    }
    if level in ("namespace", "database"):
        creds["namespace"] = _namespace()
    if level == "database":
        creds["database"] = _database()
    return creds


async def _authenticate(db: AsyncSurreal) -> None:
    await db.signin(_credentials())
    await db.use(_namespace(), _database())


class _Reconnecting:
    """A handle that reopens the WebSocket once it has been dropped.

    An idle connection eventually dies -- "keepalive ping timeout" after the
    server stops answering pings, or the machine sleeping -- and surrealdb-py
    does not heal it: its ``connect()`` returns early while the dead socket is
    still attached, so every later query fails with ``ConnectionClosed`` until
    the process restarts. Only ``query_raw`` is wrapped because that is the one
    call SurrealFs makes; everything else passes straight through.
    """

    def __init__(self, db: AsyncSurreal) -> None:
        self._db = db
        self._lock = asyncio.Lock()

    def __getattr__(self, name: str) -> Any:
        return getattr(self._db, name)

    async def query_raw(self, *args: Any, **kwargs: Any) -> Any:
        try:
            return await self._db.query_raw(*args, **kwargs)
        except (ConnectionClosed, ConnectionUnavailableError):
            await self._reopen(self._db.socket)
            return await self._db.query_raw(*args, **kwargs)

    async def _reopen(self, dead: Any) -> None:
        async with self._lock:
            # Another request that failed on the same socket got here first;
            # reconnecting again would drop the connection it just opened.
            if self._db.socket is not dead:
                return
            await self._db.close()  # clears the dead socket so connect() reopens
            await _authenticate(self._db)


async def connect() -> AsyncSurreal:
    """Connect and authenticate, for a process that keeps the handle.

    The schema is *not* applied: a long-running program pointed at a shared
    database usually finds the `file` table already there, and may hold a
    credential with no right to define one. Callers that need it apply it
    themselves and decide whether a failure is fatal.

    The connection must be opened on the loop that will use it -- a SurrealDB
    WebSocket belongs to the loop it was opened on -- so `await` this from inside
    the server's own loop, not from a separate `asyncio.run`.
    """
    db = AsyncSurreal(os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"))
    await _authenticate(db)
    return _Reconnecting(db)


@asynccontextmanager
async def connected() -> AsyncIterator[AsyncSurreal]:
    """Connect, authenticate, ensure the schema, and close again afterwards.

    One connection per call. Hermes offers no lifecycle hook to hang a long-lived
    one on, and caching would be wrong anyway: its `_run_async` bridge picks a loop
    per call site (persistent in the CLI, per-thread for parallel tools, throwaway
    inside a gateway), and a SurrealDB WebSocket belongs to the loop it was opened
    on. Calls arrive at LLM pace, so the connect cost hides in the noise.
    """
    global _schema_applied

    db = AsyncSurreal(os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"))
    try:
        await _authenticate(db)
        if not _schema_applied:
            # Once per process, so a first call works against a bare database
            # without the user running `python -m surrealfs.schema` first.
            await apply_schema(db)
            _schema_applied = True
        yield db
    finally:
        await db.close()
