"""Cross-Language Conformance Test Suite (Phase 3.4).

Verifies that Python (surrealfs), Rust (surrealfs-cli / surrealfs-core),
and TypeScript (@surrealdb/fs / Bun) maintain byte-for-byte identical state,
generation semantics, permissions, advisory locks, zero-copy workspaces,
and agent mailboxes against the exact same SurrealDB instance.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_CLI_BIN = REPO_ROOT / "target" / "debug" / "surrealfs-rs"


def _ensure_rust_cli():
    if not RUST_CLI_BIN.exists():
        subprocess.run(
            ["cargo", "build", "-p", "surrealfs-cli"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
        )


def run_rust_cli(
    args: list[str], url: str, ns: str, db: str = "test", caller: str = "root"
) -> str:
    _ensure_rust_cli()
    cmd = [
        str(RUST_CLI_BIN),
        "--url",
        url,
        "--ns",
        ns,
        "--db",
        db,
        "--user",
        "root",
        "--pass",
        "root",
        "--caller",
        caller,
        *args,
    ]
    res = subprocess.run(cmd, cwd=REPO_ROOT, capture_output=True, text=True)
    if res.returncode != 0:
        err_msg = f"Rust CLI failed ({res.returncode}): {res.stderr}\n{res.stdout}"
        raise RuntimeError(err_msg)
    return res.stdout


def run_ts_code(ts_code: str, url: str, ns: str, db: str = "test") -> str:
    wrapper = f"""
import {{ Surreal }} from 'surrealdb';
import {{ SurrealFs }} from './packages/sdk-ts/src/index.ts';

const db = new Surreal();
await db.connect('{url}');
await db.signin({{ username: 'root', password: 'root' }});
await db.use({{ namespace: '{ns}', database: '{db}' }});

const fs = new SurrealFs(db, {{ user: 'root' }});

try {{
    {ts_code}
}} finally {{
    await db.close();
}}
"""
    cmd = ["bun", "-e", wrapper]
    res = subprocess.run(cmd, cwd=REPO_ROOT, capture_output=True, text=True)
    if res.returncode != 0:
        err_msg = f"TS script failed ({res.returncode}): {res.stderr}\n{res.stdout}"
        raise RuntimeError(err_msg)
    return res.stdout


@pytest.mark.asyncio
async def test_cross_language_crud_roundtrip(fs, surreal_url, namespace):
    """Verify write in Python -> read in Rust -> append in TS -> read in Python."""
    # 1. Python writes initial file
    path = "/shared/specs.md"
    initial_content = "# System Specifications\nVersion 1.0.0"
    await fs.write_text(path, initial_content)

    entry = await fs.stat(path)
    assert entry.generation == 1

    # 2. Rust CLI reads file
    rust_cat = run_rust_cli(["cat", path], surreal_url, namespace)
    assert rust_cat == initial_content

    # 3. TS SDK stats and appends content
    ts_code = f"""
const stat = await fs.stat('{path}');
if (!stat || stat.generation !== 1) {{
    throw new Error('TS stat generation mismatch: ' + JSON.stringify(stat));
}}
await fs.appendText('{path}', '\\n## Appendix A\\nDetails here.', 1);
console.log('TS_APPEND_OK');
"""
    ts_out = run_ts_code(ts_code, surreal_url, namespace)
    assert "TS_APPEND_OK" in ts_out

    # 4. Python reads back appended content
    updated_content = await fs.read_text(path)
    assert "Version 1.0.0" in updated_content
    assert "## Appendix A" in updated_content

    updated_entry = await fs.stat(path)
    assert updated_entry.generation == 2

    # 5. Rust CLI writes another file in same directory
    notes_path = "/shared/notes.txt"
    run_rust_cli(
        ["write", notes_path, "Meeting notes: sync passed"],
        surreal_url,
        namespace,
    )

    # 6. Python lists directory
    listing = await fs.ls("/shared")
    filenames = [f.filename for f in listing]
    assert "specs.md" in filenames
    assert "notes.txt" in filenames


@pytest.mark.asyncio
async def test_cross_language_advisory_locks(fs, surreal_url, namespace):
    """Verify advisory locking across Rust CLI, TS SDK, and Python."""
    path = "/locked/file.txt"
    await fs.write_text(path, "Critical config")

    # 1. Rust CLI acquires lock
    run_rust_cli(
        ["lock", "acquire", path, "--ttl", "60", "--reason", "rust-maintenance"],
        surreal_url,
        namespace,
    )

    # 2. TS SDK inspects locks
    ts_code = f"""
const locks = await fs.listLocks();
const targetLock = locks.find(l => l.path === '{path}');
if (!targetLock) {{
    throw new Error('Lock not visible to TS SDK');
}}
if (targetLock.reason !== 'rust-maintenance') {{
    throw new Error('Lock reason mismatch: ' + targetLock.reason);
}}
console.log('TS_LOCK_VERIFIED');
"""
    ts_out = run_ts_code(ts_code, surreal_url, namespace)
    assert "TS_LOCK_VERIFIED" in ts_out

    # 3. Rust CLI releases lock
    run_rust_cli(["lock", "release", path], surreal_url, namespace)

    # 4. TS SDK confirms lock is released
    ts_verify_unlocked = f"""
const locks = await fs.listLocks();
if (locks.some(l => l.path === '{path}')) {{
    throw new Error('Lock still active');
}}
console.log('TS_UNLOCKED_OK');
"""
    ts_out2 = run_ts_code(ts_verify_unlocked, surreal_url, namespace)
    assert "TS_UNLOCKED_OK" in ts_out2


@pytest.mark.asyncio
async def test_cross_language_workspaces_and_mailboxes(fs, surreal_url, namespace):
    """Verify zero-copy workspace fork/diff/merge & mailboxes across languages."""
    # 1. Python writes base document
    doc_path = "/workflow/task.md"
    await fs.write_text(doc_path, "# Task\nInitial draft.")

    # 2. Rust CLI forks main -> agent-branch
    run_rust_cli(["fork", "main", "agent-branch"], surreal_url, namespace)

    # 3. TS SDK modifies doc in agent-branch
    ts_code = f"""
await fs.setBranch('agent-branch');
await fs.writeText('{doc_path}', '# Task\\nUpdated by TypeScript agent.');
console.log('TS_BRANCH_WRITE_OK');
"""
    ts_out = run_ts_code(ts_code, surreal_url, namespace)
    assert "TS_BRANCH_WRITE_OK" in ts_out

    # 4. Rust CLI diffs workspace
    diff_out = run_rust_cli(["diff", "agent-branch"], surreal_url, namespace)
    assert "MODIFIED" in diff_out
    assert doc_path in diff_out

    # 5. Rust CLI merges agent-branch into main
    merge_out = run_rust_cli(["merge", "agent-branch", "main"], surreal_url, namespace)
    assert '"status":"ok"' in merge_out or "ok" in merge_out

    # 6. Python verifies merged content on main
    main_content = await fs.read_text(doc_path)
    assert "Updated by TypeScript agent." in main_content

    # 7. Python dispatches task into agent mailbox
    msg = await fs.send_message("agent-ts", "task-01.json", '{"action": "synthesize"}')
    assert msg.path == "/agents/agent-ts/inbox/task-01.json"

    # 8. TS SDK receives task, claims, and completes
    ts_claim_code = """
const messages = await fs.receiveMessages('agent-ts');
if (messages.length !== 1) {
    throw new Error('TS receiveMessages failed: ' + JSON.stringify(messages));
}
if (messages[0].filename !== 'task-01.json') {
    throw new Error('Filename mismatch: ' + messages[0].filename);
}
const claimed = await fs.claimMessage('agent-ts', 'task-01.json', 'worker-ts');
if (!claimed.path.includes('.claimed/worker-ts')) {
    throw new Error('Claim path mismatch: ' + claimed.path);
}
await fs.completeMessage('agent-ts', 'task-01.json', 'worker-ts');
console.log('TS_MAILBOX_PROCESSED');
"""
    ts_mailbox_out = run_ts_code(ts_claim_code, surreal_url, namespace)
    assert "TS_MAILBOX_PROCESSED" in ts_mailbox_out

    # 9. Python confirms inbox is clean
    messages_after = await fs.receive_messages("agent-ts")
    assert len(messages_after) == 0


@pytest.mark.asyncio
async def test_cross_language_recursive_tree_and_grep(fs, surreal_url, namespace):
    """Verify recursive directory operations, tree traversal, and grep."""
    # 1. Python writes tree
    await fs.write_text("/tree/sub/a.txt", "Alpha line with TARGET_WORD here")
    await fs.write_text("/tree/sub/b.txt", "Beta line without match")
    await fs.write_text("/tree/c.txt", "Gamma line with TARGET_WORD too")

    # 2. Rust CLI recursive grep
    grep_out = run_rust_cli(
        ["grep", "TARGET_WORD", "/tree", "-r"], surreal_url, namespace
    )
    assert "/tree/sub/a.txt" in grep_out
    assert "/tree/c.txt" in grep_out
    assert "/tree/sub/b.txt" not in grep_out

    # 3. TS SDK tree
    ts_tree_code = """
const files = await fs.tree('/tree');
if (!files.includes('/tree/sub/a.txt') || !files.includes('/tree/c.txt')) {
    throw new Error('TS tree mismatch: ' + JSON.stringify(files));
}
console.log('TS_TREE_OK');
"""
    ts_out = run_ts_code(ts_tree_code, surreal_url, namespace)
    assert "TS_TREE_OK" in ts_out

    # 4. Rust CLI recursive rm
    run_rust_cli(["rm", "/tree", "-r"], surreal_url, namespace)

    # 5. Python confirms deletion
    assert not await fs.exists("/tree")
    assert not await fs.exists("/tree/sub/a.txt")


@pytest.mark.asyncio
async def test_cross_language_history_and_restore(fs, surreal_url, namespace):
    """Verify version history tracking and rollback across Rust, TS, and Python."""
    path = "/history/doc.txt"

    # 1. Rust CLI writes Gen 1
    run_rust_cli(["write", path, "Generation One Draft"], surreal_url, namespace)

    # 2. Python writes Gen 2
    await fs.write_text(path, "Generation Two Review")

    # 3. TS SDK writes Gen 3
    ts_gen3 = f"""
await fs.writeText('{path}', 'Generation Three Final');
console.log('TS_GEN3_OK');
"""
    run_ts_code(ts_gen3, surreal_url, namespace)

    # 4. TS SDK reads history
    ts_hist = f"""
const hist = await fs.history('{path}');
if (hist.length < 2) {{
    throw new Error('History length mismatch: ' + hist.length);
}}
console.log('HIST_LEN_' + hist.length);
"""
    ts_hist_out = run_ts_code(ts_hist, surreal_url, namespace)
    assert "HIST_LEN_" in ts_hist_out

    # 5. Rust CLI restores to Gen 1
    run_rust_cli(["restore", path, "1"], surreal_url, namespace)

    # 6. Python verifies restored content
    content = await fs.read_text(path)
    assert content == "Generation One Draft"


@pytest.mark.asyncio
async def test_cross_language_crdt_collaboration_and_chunking(
    fs, surreal_url, namespace
):
    """Verify Yjs CRDT collaborative edits and chunking across Python, Rust, and TS."""
    path = "/collab/shared.md"
    initial_text = "# Project Spec\n\n## Overview\nInitial content\n"

    # 1. Python writes initial doc and enables CRDT
    await fs.write_text(path, initial_text)
    await fs.enable_crdt(path)
    stat = await fs.stat(path)
    assert stat.crdt is True

    # 2. Rust CLI chunks the file
    chunk_out = run_rust_cli(["chunk", path], surreal_url, namespace)
    assert "Project Spec > Overview" in chunk_out

    # 3. Rust CLI appends to CRDT doc
    rust_text = (
        "# Project Spec\n\n## Overview\nInitial content\n\n"
        "## Section Rust\nAdded by Rust\n"
    )
    run_rust_cli(["write", path, rust_text], surreal_url, namespace)

    # 4. TS SDK appends another line
    ts_append = f"""
await fs.appendText('{path}', '## Section TS\\nAdded by TypeScript\\n');
console.log('TS_CRDT_APPEND_OK');
"""
    ts_out = run_ts_code(ts_append, surreal_url, namespace)
    assert "TS_CRDT_APPEND_OK" in ts_out

    # 5. Python reads back converged document
    merged = await fs.read_text(path)
    assert "Added by Rust" in merged
    assert "Added by TypeScript" in merged

    # 6. Rust CLI compacts CRDT log
    compact_out = run_rust_cli(["crdt", "compact", path], surreal_url, namespace)
    assert "Compacted CRDT" in compact_out

    # 7. TS reads back after compaction
    ts_verify = f"""
const content = await fs.readText('{path}');
if (!content.includes('Added by Rust') || !content.includes('Added by TypeScript')) {{
    throw new Error('Compaction lost data: ' + content);
}}
console.log('TS_CRDT_VERIFIED');
"""
    ts_verify_out = run_ts_code(ts_verify, surreal_url, namespace)
    assert "TS_CRDT_VERIFIED" in ts_verify_out
