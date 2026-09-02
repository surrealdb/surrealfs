"""Guards on the file browser: the JSON API the page is written against, the
chat transcript round trip, and the signin payload each auth level produces."""

from datetime import datetime

import httpx
import pytest
from pydantic_ai.messages import (
    FunctionToolCallEvent,
    ModelMessagesTypeAdapter,
    ModelRequest,
    ModelResponse,
    PartDeltaEvent,
    PartStartEvent,
    TextPart,
    TextPartDelta,
    ToolCallPart,
    UserPromptPart,
)
from websockets.exceptions import ConnectionClosedError

from surrealfs import browser
from surrealfs.integrations import _connect


def client(fs) -> httpx.AsyncClient:
    """Talk to the app in this test's event loop.

    Not starlette's `TestClient`: it drives the app from a thread with its own
    loop, and a SurrealDB WebSocket belongs to the loop it was opened on -- the
    same reason `serve()` runs uvicorn by hand instead of `uvicorn.run()`.
    """
    return httpx.AsyncClient(
        transport=httpx.ASGITransport(app=browser.build_app(fs)),
        base_url="http://browser",
    )


async def test_the_json_api_the_page_is_written_against(fs):
    """The page has no other contract with the server than these shapes."""
    async with client(fs) as http:
        made = await http.post(
            "/api/file", json={"path": "/notes/a.md", "folder": False}
        )
        assert made.status_code == 200, made.text
        assert made.json().keys() == {
            "path",
            "filename",
            "is_folder",
            "content_type",
            "size",
            "updated_at",
        }
        assert made.json()["content_type"] == "text/markdown"

        # The tree is flat and recursive: the page builds the hierarchy itself,
        # including the `/notes` folder, which was created implicitly.
        tree = (await http.get("/api/tree")).json()
        assert {"/notes", "/notes/a.md"} <= {e["path"] for e in tree}
        assert [e["is_folder"] for e in tree if e["path"] == "/notes"] == [True]

        again = await http.post(
            "/api/file", json={"path": "/notes/a.md", "folder": False}
        )
        assert again.status_code == 409
        assert "error" in again.json()

        saved = await http.put(
            "/api/file", json={"path": "/notes/a.md", "content": "# hi\n\nrefactor"}
        )
        assert saved.status_code == 200, saved.text
        assert saved.json()["size"] == len("# hi\n\nrefactor")

        assert (await http.get("/raw", params={"path": "/notes/a.md"})).text == (
            "# hi\n\nrefactor"
        )

        found = (await http.get("/api/search", params={"q": "refactor"})).json()
        assert found["hybrid"] is False  # no embedder was passed
        assert [hit["path"] for hit in found["results"]] == ["/notes/a.md"]
        assert "refactor" in found["results"][0]["snippet"]

        moved = await http.post(
            "/api/move", json={"src": "/notes/a.md", "dst": "/b.md"}
        )
        assert moved.json()["path"] == "/b.md"

        gone = await http.request("DELETE", "/api/file", params={"path": "/b.md"})
        assert gone.json() == {"removed": 1}


async def test_a_non_empty_folder_needs_the_recursive_flag(fs):
    """The 409 is what drives the page's second confirmation."""
    async with client(fs) as http:
        await http.post("/api/file", json={"path": "/x/y.md", "folder": False})
        refused = await http.request("DELETE", "/api/file", params={"path": "/x"})
        assert refused.status_code == 409

        forced = await http.request(
            "DELETE", "/api/file", params={"path": "/x", "recursive": "1"}
        )
        assert forced.json() == {"removed": 2}


def test_the_page_is_the_built_react_bundle():
    """`static/` is gitignored, so a fresh clone has no UI until `just ui` runs.

    Nothing here can build it; assert the path the app serves, which is what
    `serve()` checks before it binds a port.
    """
    assert browser.INDEX.name == "index.html"
    assert browser.INDEX.parent == browser.STATIC


def test_a_denial_is_a_403_not_a_server_error():
    """`PermissionDenied` is an `OSError`, not a `ValueError`, so `on_error`'s
    fallback turned every denial into a 500. Newly reachable: `--user` lets the
    browser run as somebody other than root, and then any click into another
    user's home takes this path."""
    from surrealfs import NotFound, PermissionDenied

    assert browser.on_error(None, PermissionDenied("nope")).status_code == 403
    assert browser.on_error(None, NotFound("gone")).status_code == 404


def test_stream_events_become_what_the_page_renders():
    delta = PartDeltaEvent(index=0, delta=TextPartDelta(content_delta="hi"))
    assert browser._stream_event(delta) == {"delta": "hi"}
    # The opening text of a part arrives here, not as a delta -- skip this and
    # every sentence loses its first word or two. A run that stops to call tools
    # resumes in a *new* part, so keep the paragraph break.
    start = PartStartEvent(index=0, part=TextPart(content="hi"))
    assert browser._stream_event(start) == {"delta": "\n\nhi"}
    call = FunctionToolCallEvent(ToolCallPart(tool_name="write_file"))
    assert browser._stream_event(call) == {"tool": "write_file"}
    assert browser._stream_event(object()) is None


def test_a_stored_conversation_replays_two_responses_as_one_bubble():
    history = [
        ModelRequest(
            parts=[
                UserPromptPart(
                    content="[The user has /a.md open in the file browser.]\n\nhi"
                )
            ]
        ),
        ModelResponse(parts=[ToolCallPart(tool_name="ls", args={})]),
        ModelResponse(parts=[TextPart(content="**done**")]),
    ]
    stored = ModelMessagesTypeAdapter.dump_json(history)
    bubbles = browser._transcript(ModelMessagesTypeAdapter.validate_json(stored))

    assert [b["who"] for b in bubbles] == ["you", "agent"], bubbles
    # The note about the open file rides on the message; a reloaded transcript
    # shows what the user actually typed.
    assert bubbles[0] == {"who": "you", "text": "hi"}
    assert bubbles[1]["tools"] == ["ls"]
    # Markdown, not HTML: the page renders the agent's reply itself.
    assert bubbles[1]["text"] == "**done**"


def test_session_paths_are_slugged_and_dated():
    assert browser._session_path("Fix the README!").endswith("-fix-the-readme.json")
    assert browser._session_path("¿?").endswith("-chat.json")
    assert browser._session_path("x").startswith(
        f"{browser.SESSIONS}/{datetime.now():%Y-%m-%d}/"
    )


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
