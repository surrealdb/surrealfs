"""Tests for optimistic concurrency control (OCC), append, head, and mkdir exist_ok."""

from __future__ import annotations

import pytest

from surrealfs import AlreadyExists, ConflictError, NotFound, SurrealFs
from surrealfs.tools import ToolContext
from surrealfs.tools.args import AppendArgs, HeadArgs, MkdirArgs
from surrealfs.tools.handlers import append_file, head, mkdir


async def test_write_text_occ(db):
    fs = SurrealFs(db, user="root")
    entry1 = await fs.write_text("/doc.txt", "Initial text")
    assert entry1.generation == 1

    # Write with expected generation matching
    entry2 = await fs.write_text("/doc.txt", "Second text", if_generation=1)
    assert entry2.generation == 2
    assert await fs.read_text("/doc.txt") == "Second text"

    # Write with stale generation raises ConflictError
    with pytest.raises(ConflictError):
        await fs.write_text("/doc.txt", "Stale text", if_generation=1)

    # Database still has generation 2
    assert (await fs.stat("/doc.txt")).generation == 2
    assert await fs.read_text("/doc.txt") == "Second text"


async def test_write_bytes_occ(db):
    fs = SurrealFs(db, user="root")
    entry1 = await fs.write_bytes("/blob.bin", b"\x00\x01\x02")
    assert entry1.generation == 1

    entry2 = await fs.write_bytes("/blob.bin", b"\x00\x01\x02\x03", if_generation=1)
    assert entry2.generation == 2

    with pytest.raises(ConflictError):
        await fs.write_bytes("/blob.bin", b"\xff", if_generation=1)

    assert await fs.read_bytes("/blob.bin") == b"\x00\x01\x02\x03"


async def test_edit_occ(db):
    fs = SurrealFs(db, user="root")
    entry1 = await fs.write_text("/config.yaml", "env: staging\ndebug: true\n")
    assert entry1.generation == 1

    diff = await fs.edit("/config.yaml", "staging", "production", if_generation=1)
    assert "-env: staging" in diff
    assert "+env: production" in diff
    stat = await fs.stat("/config.yaml")
    assert stat.generation == 2

    # Stale generation edit fails
    with pytest.raises(ConflictError):
        await fs.edit("/config.yaml", "debug: true", "debug: false", if_generation=1)

    assert "debug: true" in (await fs.read_text("/config.yaml"))


async def test_append_text(db):
    fs = SurrealFs(db, user="root")
    entry1 = await fs.write_text("/app.log", "2026-10-01 Start\n")
    assert entry1.generation == 1

    # Append without if_generation
    entry2 = await fs.append_text("/app.log", "2026-10-02 Event\n")
    assert entry2.generation == 2
    assert await fs.read_text("/app.log") == "2026-10-01 Start\n2026-10-02 Event\n"

    # Append with matching if_generation
    entry3 = await fs.append_text("/app.log", "2026-10-03 Done\n", if_generation=2)
    assert entry3.generation == 3

    # Append with stale if_generation
    with pytest.raises(ConflictError):
        await fs.append_text("/app.log", "2026-10-04 Late\n", if_generation=2)

    # Append to nonexistent file raises NotFound
    with pytest.raises(NotFound):
        await fs.append_text("/missing.log", "Content")


async def test_head(db):
    fs = SurrealFs(db, user="root")
    lines = [f"line {i}" for i in range(1, 26)]
    await fs.write_text("/lines.txt", "\n".join(lines))

    # Default n=10
    head_10 = await fs.head("/lines.txt")
    assert head_10.splitlines() == lines[:10]

    # Custom n=3
    head_3 = await fs.head("/lines.txt", n=3)
    assert head_3.splitlines() == lines[:3]

    # n larger than file
    head_all = await fs.head("/lines.txt", n=50)
    assert head_all.splitlines() == lines

    # Empty file
    await fs.write_text("/empty.txt", "")
    assert await fs.head("/empty.txt") == ""


async def test_mkdir_exist_ok(db):
    fs = SurrealFs(db, user="root")
    d1 = await fs.mkdir("/data")
    assert d1.path == "/data"
    assert d1.is_folder is True

    # Recreating with exist_ok=False raises AlreadyExists
    with pytest.raises(AlreadyExists):
        await fs.mkdir("/data", exist_ok=False)

    # Recreating with exist_ok=True succeeds
    d2 = await fs.mkdir("/data", exist_ok=True)
    assert d2.path == "/data"
    assert d2.is_folder is True

    # Nested with parents=True and exist_ok=True
    nested = await fs.mkdir("/data/a/b/c", parents=True, exist_ok=True)
    assert nested.path == "/data/a/b/c"

    # Repeated nested mkdir
    nested2 = await fs.mkdir("/data/a/b/c", parents=True, exist_ok=True)
    assert nested2.path == "/data/a/b/c"


async def test_sniff_content_type(db):
    from surrealfs.fs import _sniff_content_type

    # Extension sniffing
    assert _sniff_content_type("query.sql", "SELECT 1;") == "text/x-sql"
    assert _sniff_content_type("run.sh", "#!/bin/bash") == "text/x-shellscript"
    assert _sniff_content_type("data.tsv", "a\tb") == "text/tab-separated-values"
    assert _sniff_content_type("style.css", "body {}") == "text/css"

    # Content-based sniffing without extension
    assert (
        _sniff_content_type("payload", '{"key": "value", "count": 42}')
        == "application/json"
    )
    assert (
        _sniff_content_type("page", "<!DOCTYPE html><html><body>Hi</body></html>")
        == "text/html"
    )
    assert (
        _sniff_content_type("feed", '<?xml version="1.0"?><rss></rss>')
        == "application/xml"
    )
    assert _sniff_content_type("notes", "Just plain notes") == "text/markdown"


async def test_head_and_append_tool_handlers(db):
    fs = SurrealFs(db, user="root")
    ctx = ToolContext(fs=fs)

    # Write initial file
    await fs.write_text("/journal.md", "Day 1: Started\nDay 2: Built\n")

    # Call head handler
    head_res = await head(ctx, HeadArgs(path="/journal.md", n=1))
    assert head_res == "Day 1: Started"

    # Call append_file handler
    append_res = await append_file(
        ctx, AppendArgs(path="/journal.md", content="Day 3: Shipped\n")
    )
    assert "/journal.md" in append_res
    assert "gen 2" in append_res

    content = await fs.read_text("/journal.md")
    assert content == "Day 1: Started\nDay 2: Built\nDay 3: Shipped\n"

    # Mkdir tool handler with exist_ok
    mkdir_res1 = await mkdir(ctx, MkdirArgs(path="/logs", exist_ok=False))
    assert "/logs" in mkdir_res1
    mkdir_res2 = await mkdir(ctx, MkdirArgs(path="/logs", exist_ok=True))
    assert "/logs" in mkdir_res2
