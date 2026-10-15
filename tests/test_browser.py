"""Guards on the connection helper (`surrealfs.integrations._connect`) and
`SurrealFs.from_server` client RPC transport."""

import pytest
from websockets.exceptions import ConnectionClosedError

from surrealfs import SurrealFs
from surrealfs.fs import _ServerRpcClient
from surrealfs.integrations import _connect


@pytest.mark.parametrize(
    "level,expected",
    [
        # The server infers the auth level from which keys are present, so a root
        # credential must not carry a namespace and a database user must carry both.
        (None, {"username", "password"}),
        ("root", {"username", "password"}),
        ("namespace", {"username", "password", "namespace"}),
        ("database", {"username", "password", "namespace", "database"}),
        ("DATABASE", {"username", "password", "namespace", "database"}),
        # Record signin is a different shape: an access method, and the
        # credentials as `variables` matching the SIGNIN clause's $user/$pass.
        ("record", {"namespace", "database", "access", "variables"}),
    ],
)
def test_the_signin_payload_matches_the_auth_level(monkeypatch, level, expected):
    monkeypatch.delenv("SURREALDB_AUTH_LEVEL", raising=False)
    if level is not None:
        monkeypatch.setenv("SURREALDB_AUTH_LEVEL", level)
    assert set(_connect._credentials()) == expected


def test_an_unknown_auth_level_is_rejected_by_name(monkeypatch):
    monkeypatch.setenv("SURREALDB_AUTH_LEVEL", "wizard")
    with pytest.raises(ValueError, match="root, namespace, database, record"):
        _connect._credentials()


def test_the_record_payload_carries_the_access_method(monkeypatch):
    monkeypatch.setenv("SURREALDB_AUTH_LEVEL", "record")
    monkeypatch.setenv("SURREALDB_USER", "alice")
    monkeypatch.setenv("SURREALDB_PASS", "hunter2")
    creds = _connect._credentials()
    assert creds["access"] == _connect.RECORD_ACCESS
    assert creds["variables"] == {"user": "alice", "pass": "hunter2"}


@pytest.mark.asyncio
async def test_a_dropped_socket_is_reopened_and_the_query_retried(monkeypatch):
    class FakeDb:
        def __init__(self):
            self.socket = "dead"
            self.calls = 0

        async def query_raw(self, sql, variables):
            self.calls += 1
            if self.socket == "dead":
                raise ConnectionClosedError(None, None)
            return sql

        async def close(self):
            self.socket = None

    monkeypatch.setattr(_connect, "_authenticate", _reopen_fake)
    db = FakeDb()
    handle = _connect._Reconnecting(db)

    assert await handle.query_raw("RETURN 1", {}) == "RETURN 1"
    assert db.calls == 2  # failed once, reconnected, succeeded


async def _reopen_fake(db):
    db.socket = "live"


@pytest.mark.asyncio
async def test_connect_uses_server_rpc_when_surrealfs_server_url_is_set(monkeypatch):
    monkeypatch.setenv("SURREALFS_SERVER_URL", "https://fs.example.com")
    monkeypatch.setenv("SURREALFS_TOKEN", "tok-123")
    monkeypatch.setenv("SURREALFS_AGENT_USER", "alice")

    handle = await _connect.connect()
    assert isinstance(handle, _ServerRpcClient)
    assert handle.rpc_url == "https://fs.example.com/api/rpc"
    assert handle.token == "tok-123"
    assert handle.user == "alice"

    fs = SurrealFs.from_server("sfs://127.0.0.1:8080", user="alice", token="tok-456")
    assert isinstance(fs.db, _ServerRpcClient)
    assert fs.db.rpc_url == "http://127.0.0.1:8080/api/rpc"
    assert fs.user == "alice"
