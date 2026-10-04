"""Tests for Yjs CRDT real-time multi-agent collaborative editing."""

import pytest

from surrealfs import SurrealFs, crdt


@pytest.mark.asyncio
async def test_crdt_lifecycle(fs: SurrealFs) -> None:
    assert crdt.is_crdt_available()

    # 1. Create file and enable CRDT mode
    path = "/shared/notes.md"
    await fs.write_text(path, "# Meeting Notes\n\nAttendees:\n- Alice\n")
    await fs.enable_crdt(path)

    entry = await fs.stat(path)
    assert entry.crdt is True

    # 2. Append text via CRDT transparent translation
    await fs.append_text(path, "- Bob\n")
    content = await fs.read_text(path)
    assert "- Bob\n" in content
    assert "- Alice\n" in content

    # 3. Edit text via CRDT transparent translation
    diff = await fs.edit(path, "- Alice\n", "- Alice (Lead)\n")
    assert "- Alice (Lead)" in diff
    content_after_edit = await fs.read_text(path)
    assert "- Alice (Lead)\n" in content_after_edit
    assert "- Bob\n" in content_after_edit

    # 4. Check CRDT update records in SurrealDB
    file_id = entry.id
    updates = await fs._query(
        "SELECT seq, author FROM file_crdt_update "
        "WHERE file_id = $file_id ORDER BY seq ASC;",
        {"file_id": file_id},
    )
    assert len(updates) >= 3  # init (1) + append (2) + edit (3)

    # 5. Compact CRDT
    await fs.compact_crdt(path)
    updates_after = await fs._query(
        "SELECT seq FROM file_crdt_update WHERE file_id = $file_id;",
        {"file_id": file_id},
    )
    assert len(updates_after) == 0  # old updates compacted into snapshot

    # 6. Verify edits still work after compaction
    await fs.append_text(path, "- Charlie\n")
    content_final = await fs.read_text(path)
    assert "- Charlie\n" in content_final
    assert "- Alice (Lead)\n" in content_final


@pytest.mark.asyncio
async def test_concurrent_crdt_convergence(fs: SurrealFs) -> None:
    path = "/shared/spec.md"
    initial = "# Spec\n\n## Section 1\nPending\n\n## Section 2\nPending\n"
    await fs.write_text(path, initial)
    await fs.enable_crdt(path)

    entry = await fs.stat(path)
    file_id = entry.id

    # Agent 1 and Agent 2 read base document concurrently
    doc1, seq1 = await fs._load_crdt_doc(file_id)
    doc2, seq2 = await fs._load_crdt_doc(file_id)

    # Agent 1 updates Section 1
    delta1, _ = crdt.apply_edit(
        doc1, "## Section 1\nPending", "## Section 1\nDone by Agent 1"
    )

    # Agent 2 updates Section 2
    delta2, _ = crdt.apply_edit(
        doc2, "## Section 2\nPending", "## Section 2\nDone by Agent 2"
    )

    # Apply Agent 1's update to SurrealDB
    await fs._apply_crdt_mutation(
        file_id, delta1, crdt.materialize(doc1), seq1, entry.generation
    )

    # Agent 2's replica receives Agent 1's delta and merges it
    doc2.apply_update(delta1)
    doc1.apply_update(delta2)

    # Both documents converge to the exact same text
    assert crdt.materialize(doc1) == crdt.materialize(doc2)

    # Apply Agent 2's delta and materialize final converged text to SurrealDB
    await fs._apply_crdt_mutation(
        file_id, delta2, crdt.materialize(doc2), seq1 + 1, entry.generation
    )

    final_content = await fs.read_text(path)
    assert "Done by Agent 1" in final_content
    assert "Done by Agent 2" in final_content
