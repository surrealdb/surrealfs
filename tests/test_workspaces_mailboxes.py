"""Tests for Workspaces, Agent Actor Mailboxes, and Watchers."""

from __future__ import annotations

import asyncio

import pytest

from surrealfs import ConflictError, NotFound, SurrealFs


@pytest.mark.asyncio
async def test_workspace_fork_diff_and_discard(fs: SurrealFs) -> None:
    # Setup files in base main filesystem
    await fs.write_text("/src/index.ts", "console.log('main');\n")
    await fs.write_text("/src/utils.ts", "export const add = (a, b) => a + b;\n")

    # Fork workspace
    fork_res = await fs.fork_workspace("agent-branch-1")
    assert fork_res["workspace"] == "agent-branch-1"
    assert fork_res["count"] == 2

    # List workspaces
    workspaces = await fs.list_workspaces()
    assert any(ws.name == "agent-branch-1" for ws in workspaces)

    # Check diff
    diff = await fs.diff_workspace("agent-branch-1")
    assert len(diff) == 2
    assert all(d["modified"] is False for d in diff)
    assert all(d["conflict"] is False for d in diff)

    # Discard workspace
    discard_res = await fs.discard_workspace("agent-branch-1")
    assert discard_res["discarded"] == "agent-branch-1"

    workspaces_after = await fs.list_workspaces()
    assert not any(ws.name == "agent-branch-1" for ws in workspaces_after)


@pytest.mark.asyncio
async def test_agent_mailbox_fifo_claim_and_complete(fs: SurrealFs) -> None:
    agent_id = "agent-alpha"

    # Send messages
    msg1 = await fs.send_message(agent_id, "01-task.json", '{"task": 1}')
    msg2 = await fs.send_message(agent_id, "02-task.json", '{"task": 2}')
    assert msg1.path == f"/agents/{agent_id}/inbox/01-task.json"
    assert msg2.path == f"/agents/{agent_id}/inbox/02-task.json"

    # Receive messages
    pending = await fs.receive_messages(agent_id)
    assert len(pending) == 2
    assert pending[0].filename == "01-task.json"
    assert pending[1].filename == "02-task.json"

    # Claim message 1 by worker-1
    claimed = await fs.claim_message(agent_id, "01-task.json", "worker-1")
    assert claimed.path == f"/agents/{agent_id}/inbox/.claimed/worker-1/01-task.json"

    # Inbox now only has message 2
    pending_after = await fs.receive_messages(agent_id)
    assert len(pending_after) == 1
    assert pending_after[0].filename == "02-task.json"

    # Attempting to re-claim message 1 raises NotFound
    with pytest.raises(NotFound):
        await fs.claim_message(agent_id, "01-task.json", "worker-2")

    # Complete message 1
    await fs.complete_message(agent_id, "01-task.json", "worker-1")
    assert not await fs.exists(claimed.path)


@pytest.mark.asyncio
async def test_live_watch_stream(fs: SurrealFs) -> None:
    received_events = []

    async def run_watcher():
        async for event in fs.watch("/watch_test/*"):
            received_events.append(event)
            if len(received_events) >= 1:
                break

    watch_task = asyncio.create_task(run_watcher())

    # Wait briefly for live subscription to register
    await asyncio.sleep(0.1)

    # Trigger a file change matching pattern
    await fs.write_text("/watch_test/event.txt", "live event trigger")

    # Wait for watcher to receive event
    try:
        await asyncio.wait_for(watch_task, timeout=2.0)
    except TimeoutError:
        watch_task.cancel()

    assert len(received_events) >= 1
    assert received_events[0].path == "/watch_test/event.txt"
    assert received_events[0].entry is not None
    assert received_events[0].entry.filename == "event.txt"


@pytest.mark.asyncio
async def test_workspace_merge_clean(fs: SurrealFs) -> None:
    # 1. Base file
    await fs.write_text("/doc.md", "Base content\n")

    # 2. Fork workspace
    await fs.fork_workspace("feature-docs")

    # 3. Edit in workspace branch
    await fs.write_workspace_text("feature-docs", "/doc.md", "Updated in workspace\n")

    # 4. Check diff
    diff = await fs.diff_workspace("feature-docs")
    assert len(diff) == 1
    assert diff[0]["modified"] is True
    assert diff[0]["conflict"] is False

    # 5. Merge workspace
    merge_res = await fs.merge_workspace("feature-docs")
    assert merge_res["status"] == "ok"

    # Base file should now reflect updated content
    assert await fs.read_text("/doc.md") == "Updated in workspace\n"


@pytest.mark.asyncio
async def test_workspace_merge_conflict(fs: SurrealFs) -> None:
    # 1. Base file on main
    await fs.write_text("/shared.txt", "Initial content\n")

    # 2. Fork workspace
    await fs.fork_workspace("concurrent-worker")

    # 3. Concurrent modification on main (advances generation)
    await fs.write_text("/shared.txt", "Modified on main\n")

    # 4. Modify in workspace
    await fs.write_workspace_text(
        "concurrent-worker", "/shared.txt", "Modified in workspace\n"
    )

    # 5. Check diff
    diff = await fs.diff_workspace("concurrent-worker")
    assert len(diff) == 1
    assert diff[0]["conflict"] is True

    # 6. Merge workspace should fail due to conflict
    with pytest.raises(Exception) as exc_info:
        await fs.merge_workspace("concurrent-worker")
    assert "conflict" in str(exc_info.value).lower()


@pytest.mark.asyncio
async def test_lease_context_manager(db) -> None:
    fs_alice = SurrealFs(db, user="alice")
    fs_bob = SurrealFs(db, user="bob")
    path = "/projects/acme.md"
    await fs_alice.write_text(path, "# Acme Corp")

    async with fs_alice.lease(path, ttl=60, reason="Editing spec") as lock:
        assert lock.get("holder") == "alice"
        # Bob cannot acquire while Alice holds it
        with pytest.raises(ConflictError):
            await fs_bob.acquire_lock(path, ttl_seconds=30)

    # After exiting context manager, lease is automatically released
    bob_lease = await fs_bob.acquire_lock(path, ttl_seconds=30)
    assert bob_lease.get("holder") == "bob"
    await fs_bob.release_lock(path)


@pytest.mark.asyncio
async def test_sandbox_context_manager_clean_merge(fs: SurrealFs) -> None:
    await fs.write_text("/src/model.py", "def run(): return 1\n")

    async with fs.sandbox(branch="agent-experiment") as sandbox_fs:
        await sandbox_fs.write_text("/src/model.py", "def run(): return 2\n")
        assert await sandbox_fs.read_text("/src/model.py") == "def run(): return 2\n"
        diff = await sandbox_fs.diff()
        assert len(diff) == 1
        assert diff[0]["modified"] is True
        await sandbox_fs.merge()

    # After clean merge, main branch reflects the change
    assert await fs.read_text("/src/model.py") == "def run(): return 2\n"


@pytest.mark.asyncio
async def test_sandbox_context_manager_auto_discard_on_failure(fs: SurrealFs) -> None:
    await fs.write_text("/src/stable.py", "stable_val = 42\n")

    with pytest.raises(ValueError):
        async with fs.sandbox(branch="failing-agent") as sandbox_fs:
            await sandbox_fs.write_text("/src/stable.py", "corrupted\n")
            raise ValueError("agent validation failed!")

    # Workspace should be auto-discarded; main file preserved
    assert await fs.read_text("/src/stable.py") == "stable_val = 42\n"
    workspaces = await fs.list_workspaces()
    assert not any(ws.name == "failing-agent" for ws in workspaces)
