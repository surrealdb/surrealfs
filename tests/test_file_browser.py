"""Guards on the two XSS-relevant settings in the demo browser."""

import sys
from pathlib import Path

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
