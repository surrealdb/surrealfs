"""Guards on the file browser: the XSS-relevant settings, the chat transcript
round trip, and the signin payload each auth level produces."""

from datetime import datetime
from pathlib import Path

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

PAGE = Path(browser.__file__).with_name("page.html")


def test_server_markdown_reaches_the_dom_only_through_the_scrubber():
    # Rendered markdown carries agent output and file contents. `setMarkdown`
    # strips event handlers and script-y URLs; a raw sink bypasses all of it.
    source = PAGE.read_text()
    assert "innerHTML" not in source
    assert "setMarkdown(" in source


def test_the_markdown_renderer_still_escapes_raw_html():
    assert browser.MD.render("<img src=x onerror=alert(1)>").strip() == (
        "<p>&lt;img src=x onerror=alert(1)&gt;</p>"
    )


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
    assert bubbles[0] == {"who": "you", "text": "hi", "html": ""}
    assert bubbles[1]["tools"] == ["ls"]
    assert bubbles[1]["html"].strip() == "<p><strong>done</strong></p>"


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
    ],
)
def test_the_signin_payload_matches_the_auth_level(monkeypatch, level, expected):
    monkeypatch.delenv("SURREALDB_AUTH_LEVEL", raising=False)
    if level is not None:
        monkeypatch.setenv("SURREALDB_AUTH_LEVEL", level)
    assert set(_connect._credentials()) == expected


def test_an_unknown_auth_level_is_rejected_by_name(monkeypatch):
    monkeypatch.setenv("SURREALDB_AUTH_LEVEL", "record")
    with pytest.raises(ValueError, match="root, namespace, database"):
        _connect._credentials()


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
