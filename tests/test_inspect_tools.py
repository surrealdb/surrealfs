"""Tests for read_range, tree, grep, MCP resources, and embed daemon."""

from __future__ import annotations

import pytest

from surrealfs.errors import NotADirectory, NotFound
from surrealfs.integrations.mcp import read_resource
from surrealfs.tools import ToolContext
from surrealfs.tools.args import GrepArgs, ReadRangeArgs, TreeArgs
from surrealfs.tools.handlers import grep, read_range, tree


async def test_read_range(fs):
    content = "\n".join(f"line {i}" for i in range(1, 11))
    await fs.write_text("/lines.txt", content)

    # Basic range
    res = await fs.read_range("/lines.txt", start=2, end=4)
    assert res == "line 2\nline 3\nline 4"

    # End = -1 (to end of file)
    res = await fs.read_range("/lines.txt", start=8, end=-1)
    assert res == "line 8\nline 9\nline 10"

    # Out of bounds end is clamped to total lines
    res = await fs.read_range("/lines.txt", start=9, end=50)
    assert res == "line 9\nline 10"

    # Start past total lines returns empty string
    res = await fs.read_range("/lines.txt", start=20, end=30)
    assert res == ""

    # Line numbers
    res_num = await fs.read_range("/lines.txt", start=9, end=10, numbers=True)
    assert res_num == " 9 | line 9\n10 | line 10"

    # Validation errors
    with pytest.raises(ValueError, match="start line must be >= 1"):
        await fs.read_range("/lines.txt", start=0, end=5)

    with pytest.raises(ValueError, match="end line must be >= start line"):
        await fs.read_range("/lines.txt", start=5, end=3)

    # Tool handler
    ctx = ToolContext(fs=fs)
    handler_res = await read_range(
        ctx, ReadRangeArgs(path="/lines.txt", start=1, end=2, numbers=False)
    )
    assert handler_res == "line 1\nline 2"


async def test_tree(fs):
    await fs.mkdir("/project/src", parents=True)
    await fs.mkdir("/project/tests", parents=True)
    await fs.write_text("/project/src/main.py", "print('hi')")
    await fs.write_text("/project/src/utils.py", "# utils")
    await fs.write_text("/project/tests/test_main.py", "assert True")
    await fs.write_text("/project/README.md", "# Project")

    rendered = await fs.tree("/project", max_depth=3)
    assert "/project" in rendered
    assert "src/" in rendered
    assert "tests/" in rendered
    assert "main.py" in rendered
    assert "README.md" in rendered

    # Error conditions
    with pytest.raises(NotADirectory):
        await fs.tree("/project/README.md")

    with pytest.raises(NotFound):
        await fs.tree("/nonexistent")

    # Tool handler
    ctx = ToolContext(fs=fs)
    handler_res = await tree(ctx, TreeArgs(path="/project", max_depth=2))
    assert "/project" in handler_res
    assert "src/" in handler_res


async def test_grep(fs):
    await fs.mkdir("/src", parents=True)
    await fs.mkdir("/docs", parents=True)
    await fs.write_text(
        "/src/app.py",
        "def start_server():\n    print('listening')\n    return True\n",
    )
    await fs.write_text(
        "/src/test.py",
        "# test file\ndef test_server():\n    assert start_server()\n",
    )
    await fs.write_text(
        "/docs/guide.md",
        "# Guide\nCall start_server() to begin.\nEnjoy!\n",
    )

    # Exact case-sensitive match
    hits = await fs.grep("start_server")
    paths = {h.path for h in hits}
    assert paths == {"/src/app.py", "/src/test.py", "/docs/guide.md"}

    # Path prefix filter
    docs_hits = await fs.grep("start_server", path_prefix="/docs")
    assert len(docs_hits) == 1
    assert docs_hits[0].path == "/docs/guide.md"
    assert docs_hits[0].line_number == 2
    assert "Call start_server() to begin." in docs_hits[0].line

    # Glob filter
    py_hits = await fs.grep("start_server", glob="*.py")
    assert {h.path for h in py_hits} == {"/src/app.py", "/src/test.py"}

    # Regex match
    regex_hits = await fs.grep(r"def\s+\w+\(\):", is_regex=True)
    assert {h.path for h in regex_hits} == {"/src/app.py", "/src/test.py"}

    # Case insensitive match
    case_hits = await fs.grep("ENJOY", case_sensitive=False)
    assert len(case_hits) == 1
    assert case_hits[0].path == "/docs/guide.md"

    # Limit
    limited = await fs.grep("start_server", limit=2)
    assert len(limited) == 2

    # Tool handler
    ctx = ToolContext(fs=fs)
    handler_res = await grep(ctx, GrepArgs(pattern="listening"))
    assert "/src/app.py:2:    print('listening')" in handler_res

    empty_res = await grep(ctx, GrepArgs(pattern="nonexistent_xyz"))
    assert "Nothing matches" in empty_res


async def test_mcp_resources(ctx):
    await ctx.fs.write_text("/data/info.txt", "SurrealFS info content")

    # Read via surrealfs:// URI
    content = await read_resource(ctx, "surrealfs:///data/info.txt")
    assert content == "SurrealFS info content"

    # Relative-style path in URI
    content2 = await read_resource(ctx, "surrealfs://data/info.txt")
    assert content2 == "SurrealFS info content"

    with pytest.raises(ValueError, match="Unsupported URI scheme"):
        await read_resource(ctx, "file:///data/info.txt")


def test_embed_env_loading(tmp_path, monkeypatch):
    from surrealfs.embed import _load_env

    env_file = tmp_path / ".env"
    env_file.write_text("SURREALDB_USER=indexer_custom\n", encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("SURREALDB_USER", raising=False)

    _load_env()
    import os

    assert os.environ.get("SURREALDB_USER") == "indexer_custom"
