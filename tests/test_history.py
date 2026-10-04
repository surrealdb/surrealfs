"""Tests for history, provenance, undo, graph relations, and locks."""

from __future__ import annotations

import pytest

from surrealfs import AlreadyExists, ConflictError, SurrealFs


@pytest.mark.asyncio
async def test_file_version_history_and_diff(fs: SurrealFs) -> None:
    path = "/docs/spec.md"
    await fs.write_text(path, "line 1\nline 2\n")

    # Update content twice to create versions
    await fs.write_text(path, "line 1\nline 2 modified\nline 3\n")
    await fs.write_text(path, "line 1\nline 2 modified\nline 3\nline 4\n")

    history = await fs.history(path)
    # History contains previous versions recorded on update
    assert len(history) >= 2
    assert any(h.generation == 1 for h in history)
    assert any(h.generation == 2 for h in history)

    # Diff generation 1 vs current
    diff_curr = await fs.diff(path, from_generation=1)
    assert "-line 2" in diff_curr
    assert "+line 2 modified" in diff_curr
    assert "+line 4" in diff_curr

    # Diff generation 1 vs generation 2
    diff_1_2 = await fs.diff(path, from_generation=1, to_generation=2)
    assert "-line 2" in diff_1_2
    assert "+line 2 modified" in diff_1_2
    assert "+line 3" in diff_1_2
    assert "+line 4" not in diff_1_2


@pytest.mark.asyncio
async def test_restore_previous_version(fs: SurrealFs) -> None:
    path = "/notes/todo.txt"
    await fs.write_text(path, "original todo list\n")
    await fs.write_text(path, "corrupted todo list\n")

    # Restore generation 1
    restored = await fs.restore(path, generation=1)
    assert restored.generation == 3
    assert await fs.read_text(path) == "original todo list\n"


@pytest.mark.asyncio
async def test_undelete_after_rm(fs: SurrealFs) -> None:
    path = "/notes/important.txt"
    await fs.write_text(path, "vital content that should not be lost\n")

    # Delete the file
    await fs.rm(path)
    assert not await fs.exists(path)

    # Undelete should bring it back
    restored = await fs.undelete(path)
    assert restored.path == path
    assert await fs.read_text(path) == "vital content that should not be lost\n"

    # Cannot undelete if file already exists
    with pytest.raises(AlreadyExists):
        await fs.undelete(path)


@pytest.mark.asyncio
async def test_graph_relations_and_backlinks(fs: SurrealFs) -> None:
    rfc = "/rfcs/001.md"
    code = "/src/auth.py"
    await fs.write_text(rfc, "# Auth RFC\n")
    await fs.write_text(code, "def authenticate(): pass\n")

    # Relate code implements rfc
    await fs.relate(code, "implements", rfc)

    # Outbound from code
    neighbors = await fs.get_neighbors(code)
    assert len(neighbors) == 1
    assert neighbors[0].target_path == rfc
    assert neighbors[0].relation == "implements"

    # Inbound to rfc (backlinks)
    backs = await fs.backlinks(rfc)
    assert len(backs) == 1
    assert backs[0].source_path == code
    assert backs[0].relation == "implements"

    # Invalid relation raises ValueError
    with pytest.raises(ValueError):
        await fs.relate(code, "invalid_rel", rfc)


@pytest.mark.asyncio
async def test_advisory_file_locking(db) -> None:
    fs_alice = SurrealFs(db, user="alice")
    fs_bob = SurrealFs(db, user="bob")

    path = "/shared/document.md"
    await fs_alice.write_text(path, "Shared doc\n")

    # Alice acquires lock
    lease = await fs_alice.acquire_lock(
        path, ttl_seconds=30, reason="editing section 1"
    )
    assert lease.get("holder") == "alice"

    # Bob attempts to acquire lock -> ConflictError
    with pytest.raises(ConflictError):
        await fs_bob.acquire_lock(path, ttl_seconds=30)

    # Alice releases lock
    assert await fs_alice.release_lock(path)

    # Bob can now acquire lock
    lease_bob = await fs_bob.acquire_lock(path, ttl_seconds=30)
    assert lease_bob.get("holder") == "bob"
