"""Guards on the two XSS-relevant settings in the demo browser."""

import sys
from pathlib import Path

import pytest
from websockets.exceptions import ConnectionClosedError

EXAMPLES = Path(__file__).parent.parent / "examples"
PAGE = EXAMPLES / "file_browser.html"

# file_browser.py imports its sibling `chat_agent` the way a script would.
sys.path.insert(0, str(EXAMPLES))


def test_server_markdown_reaches_the_dom_only_through_the_scrubber():
    # Rendered markdown carries agent output and file contents. `setMarkdown`
    # strips event handlers and script-y URLs; a raw sink bypasses all of it.
    source = PAGE.read_text()
    assert "innerHTML" not in source
    assert "setMarkdown(" in source


def test_the_markdown_renderer_still_escapes_raw_html():
    from examples.file_browser import MD

    assert MD.render("<img src=x onerror=alert(1)>").strip() == (
        "<p>&lt;img src=x onerror=alert(1)&gt;</p>"
    )


@pytest.mark.asyncio
async def test_a_dropped_socket_is_reopened_and_the_query_retried(monkeypatch):
    from examples import chat_agent

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

    monkeypatch.setattr(chat_agent, "_authenticate", _reopen_fake)
    db = FakeDb()
    handle = chat_agent._Reconnecting(db)

    assert await handle.query_raw("RETURN 1", {}) == "RETURN 1"
    assert db.calls == 2  # failed once, reconnected, succeeded


async def _reopen_fake(db):
    db.socket = "live"
