"""Test fixtures.

These tests need a real SurrealDB server, so we start a throwaway in-memory one.

The embedded engine (``mem://``, via ``surrealdb[embedded]``) is *not* usable,
even though as of surrealdb-py 3.0.0a4 it does parse the schema. The engine it
bundles is 3.0.0-alpha.4, which returns ``null`` for COMPUTED fields whenever a
row is reached through an index — and `path` and `is_folder` are both computed.
Every indexed read (`ls`, path resolution, full-text search) would silently come
back with no path. Verified against the same SDK talking to a 3.2.x server, where
the identical query is correct, so it is an engine bug rather than an SDK one.
Re-test when a newer ``surrealdb-embedded`` ships.

Set ``SURREALFS_TEST_URL`` to reuse a server you already have running.
"""

from __future__ import annotations

import contextlib
import os
import shutil
import socket
import subprocess
import time

import pytest
from surrealdb import AsyncSurreal

from surrealfs import SurrealFs, apply_schema
from surrealfs.tools import ToolContext

ROOT = {"username": "root", "password": "root"}


def _free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


@pytest.fixture(autouse=True)
def _clean_surrealdb_env(monkeypatch):
    """Tests talk to the throwaway server, never to whatever ``.env`` configures.

    ``just test`` sets ``dotenv-load``, so without this a developer's real
    ``SURREALDB_USER``/``PASS``/``AUTH_LEVEL`` reach ``integrations._connect``,
    which reads the environment, and signin against the local root-only server
    fails. Each test sets the variables it needs itself.
    """
    for name in [n for n in os.environ if n.startswith("SURREALDB_")]:
        monkeypatch.delenv(name)


@pytest.fixture(scope="session")
def surreal_url() -> str:
    """URL of a running SurrealDB 3.x server, starting one if needed."""
    if url := os.environ.get("SURREALFS_TEST_URL"):
        yield url
        return

    binary = shutil.which("surreal")
    if binary is None:
        pytest.skip(
            "the `surreal` binary is not on PATH; install SurrealDB 3.x or set "
            "SURREALFS_TEST_URL to point at a running server"
        )

    port = _free_port()
    process = subprocess.Popen(
        [
            binary,
            "start",
            "--allow-all",
            "-u",
            "root",
            "-p",
            "root",
            "--bind",
            f"127.0.0.1:{port}",
            "memory",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            pytest.fail("surreal server exited during startup")
        with (
            contextlib.suppress(OSError),
            socket.create_connection(("127.0.0.1", port), timeout=0.5),
        ):
            break
        time.sleep(0.1)
    else:
        process.kill()
        pytest.fail("surreal server did not start within 30s")

    yield f"ws://127.0.0.1:{port}/rpc"

    process.terminate()
    with contextlib.suppress(subprocess.TimeoutExpired):
        process.wait(timeout=10)


@pytest.fixture
async def db(surreal_url, request):
    """A connection to a namespace unique to this test, with the schema applied."""
    connection = AsyncSurreal(surreal_url)
    await connection.signin(ROOT)
    # A fresh namespace per test keeps them isolated and order-independent.
    name = "t" + str(abs(hash(request.node.nodeid)))[:12]
    await connection.query_raw(
        f"REMOVE NAMESPACE IF EXISTS {name}; DEFINE NAMESPACE {name};"
    )
    await connection.use(name, "test")
    await apply_schema(connection)
    yield connection
    with contextlib.suppress(Exception):
        await connection.query_raw(f"REMOVE NAMESPACE IF EXISTS {name}")
    await connection.close()


@pytest.fixture
def fs(db) -> SurrealFs:
    return SurrealFs(db)


@pytest.fixture
def ctx(fs) -> ToolContext:
    return ToolContext(fs=fs)
