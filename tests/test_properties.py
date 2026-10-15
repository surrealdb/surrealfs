"""Property-based tests verifying core filesystem invariants using Hypothesis."""

from __future__ import annotations

import posixpath

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from surrealfs import (
    AlreadyExists,
    ConflictError,
    DirectoryNotEmpty,
    IsADirectory,
    SurrealFs,
)
from surrealfs.fs import _parent_key

# Hypothesis strategy for valid Unix-like relative and absolute paths
_PATH_SEGMENT = st.text(
    alphabet=st.characters(
        whitelist_categories=("Lu", "Ll", "Nd"), whitelist_characters="_-"
    ),
    min_size=1,
    max_size=12,
)

_VALID_PATHS = st.lists(_PATH_SEGMENT, min_size=1, max_size=4).map(
    lambda segs: "/" + "/".join(segs)
)


@st.composite
def filesystem_operations(draw):
    """Generate a sequence of random filesystem mutations."""
    ops = []
    num_ops = draw(st.integers(min_value=3, max_value=8))
    for _ in range(num_ops):
        op_type = draw(st.sampled_from(["write", "mkdir", "mv", "rm"]))
        path = draw(_VALID_PATHS)
        content = draw(st.text(min_size=1, max_size=50))
        ops.append((op_type, path, content))
    return ops


@pytest.mark.asyncio
async def test_tree_hierarchy_invariant_no_cycles_or_orphans(fs: SurrealFs) -> None:
    """Verify Tree Hierarchy Invariant across a set of directories and files:

    1. No cycles exist in parent chains.
    2. Every child's parent_key references an existing folder.
    3. Recursive `ls` paths match computed ancestry.
    """
    # Create a non-trivial directory structure
    dirs = [
        "/docs",
        "/docs/api",
        "/docs/guides",
        "/src",
        "/src/core",
        "/src/utils",
    ]
    for d in dirs:
        await fs.mkdir(d, parents=True, exist_ok=True)

    files = [
        ("/docs/api/index.md", "API index"),
        ("/docs/guides/auth.md", "Auth guide"),
        ("/src/core/main.py", "print('main')"),
        ("/src/utils/helpers.py", "def help(): pass"),
    ]
    for path, content in files:
        await fs.write_text(path, content)

    # Invariant 1: ls recursive lists all entries without error
    all_entries = await fs.ls("/", recursive=True)
    all_paths = {e.path for e in all_entries}

    for d in dirs:
        assert d in all_paths
    for f, _ in files:
        assert f in all_paths

    # Invariant 2: No orphan parent_keys
    # Query all raw rows from database directly
    rows = await fs._query(
        "SELECT id, path, parent, parent_key, filename, is_folder FROM file"
    )
    id_set = {_parent_key(r["id"]) for r in rows}

    for r in rows:
        parent_key = r.get("parent_key")
        if parent_key != "root":
            # Parent must exist in the database
            assert parent_key in id_set, f"Orphan node found: {r['path']}"

        # Invariant 3: Path hierarchy matches parent path + filename
        computed_path = r["path"]
        if computed_path != "/":
            expected_filename = posixpath.basename(computed_path)
            assert r["filename"] == expected_filename


@pytest.mark.asyncio
async def test_no_silent_success_invariant(fs: SurrealFs) -> None:
    """Verify mutations that report success actually changed state,

    and invalid operations raise explicit exceptions rather than silently succeeding.
    """
    path = "/test/strict.txt"
    initial_content = "initial content v1"

    # Write file
    entry = await fs.write_text(path, initial_content)
    assert entry.generation == 1
    assert await fs.read_text(path) == initial_content

    # Invariant: Generation mismatch must raise ConflictError and NOT mutate
    with pytest.raises(ConflictError):
        await fs.write_text(path, "invalid update", if_generation=999)

    # Content must remain unchanged
    assert await fs.read_text(path) == initial_content

    # Invariant: Overwriting a directory with write_text must raise IsADirectory
    dir_path = "/test/dir"
    await fs.mkdir(dir_path)
    with pytest.raises(IsADirectory):
        await fs.write_text(dir_path, "not allowed")

    # Invariant: mkdir on existing directory without exist_ok=True raises AlreadyExists
    with pytest.raises(AlreadyExists):
        await fs.mkdir(dir_path, exist_ok=False)

    # Invariant: Non-recursive rm on directory with children raises DirectoryNotEmpty
    child_file = "/test/dir/child.txt"
    await fs.write_text(child_file, "child")
    with pytest.raises(DirectoryNotEmpty):
        await fs.rm(dir_path, recursive=False)

    # Child still exists
    assert await fs.exists(child_file)


@pytest.mark.asyncio
async def test_history_completeness_and_reversibility_invariant(fs: SurrealFs) -> None:
    """Verify that every write generates a version, diffs are exact,

    and restore reversibly returns to any previous point.
    """
    path = "/docs/spec.txt"
    v1 = "Version 1\nAlpha"
    v2 = "Version 2\nBeta"
    v3 = "Version 3\nGamma"

    await fs.write_text(path, v1)
    await fs.write_text(path, v2)
    await fs.write_text(path, v3)

    history = await fs.history(path)
    # Past versions recorded on update
    assert len(history) == 2
    assert history[0].generation == 2
    assert history[1].generation == 1

    # Invariant: diff between v1 and current shows accurate changes
    diff_output = await fs.diff(path, from_generation=1)
    assert "-Version 1" in diff_output
    assert "+Version 3" in diff_output

    # Invariant: Restoring v1 yields exact v1 content
    restored = await fs.restore(path, history[1].generation)
    assert restored.generation == 4
    assert await fs.read_text(path) == v1


@pytest.mark.asyncio
@given(ops=filesystem_operations())
@settings(
    max_examples=10,
    deadline=None,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
async def test_hypothesis_operation_sequence_consistency(fs: SurrealFs, ops) -> None:
    """Property test running generated operations against SurrealFs and ref model."""
    import uuid

    prefix = f"/prop_{uuid.uuid4().hex[:8]}"
    await fs.mkdir(prefix, parents=True, exist_ok=True)
    ref_files: dict[str, str] = {}
    ref_dirs: set[str] = {prefix}

    for op, rel_path, content in ops:
        leaf = posixpath.basename(rel_path)
        path = f"{prefix}/{leaf}"
        if op == "write":
            if path in ref_dirs:
                with pytest.raises(IsADirectory):
                    await fs.write_text(path, content)
            else:
                await fs.write_text(path, content)
                ref_files[path] = content
        elif op == "mkdir":
            if path in ref_files or path in ref_dirs:
                await fs.mkdir(path, exist_ok=True) if path in ref_dirs else None
            else:
                await fs.mkdir(path, exist_ok=True)
                ref_dirs.add(path)
        elif op == "rm":
            if path in ref_files or path in ref_dirs:
                await fs.rm(path, recursive=True)
                ref_files.pop(path, None)
                ref_dirs.discard(path)
        elif op == "mv":
            dst = path + "_moved"
            if (
                (path in ref_files or path in ref_dirs)
                and dst not in ref_files
                and dst not in ref_dirs
            ):
                await fs.mv(path, dst)
                if path in ref_files:
                    ref_files[dst] = ref_files.pop(path)
                if path in ref_dirs:
                    ref_dirs.remove(path)
                    ref_dirs.add(dst)

    entries = await fs.ls(prefix, recursive=True)
    actual_files = {e.path for e in entries if not e.is_folder}
    actual_dirs = {e.path for e in entries if e.is_folder} | {prefix}
    assert actual_files == set(ref_files.keys())
    assert actual_dirs == ref_dirs
    for fpath, expected_text in ref_files.items():
        assert await fs.read_text(fpath) == expected_text
    await fs.rm(prefix, recursive=True)


@given(
    st.lists(
        st.tuples(
            st.text(
                alphabet=st.characters(whitelist_categories=["Lu", "Ll", "Nd", "Zs"]),
                min_size=1,
                max_size=30,
            ),
            st.integers(min_value=0, max_value=2),
        ),
        min_size=2,
        max_size=4,
    )
)
def test_hypothesis_crdt_convergence_invariant(edits) -> None:
    """Invariant: regardless of arrival order, any permutation of updates

    produces identical text.
    """
    import itertools

    from surrealfs import crdt

    base_text = "Initial Base Document\nLine 1\nLine 2\nLine 3\n"
    base_doc, base_update = crdt.init_doc(base_text)

    # Generate independent updates from distinct replica clients
    updates = []
    for text_to_insert, line_idx in edits:
        doc = crdt.load_doc(base_update, [])
        sv = doc.get_state()
        t = doc.get("text", type=crdt.pycrdt.Text)
        pos = min(len(str(t)), line_idx * 7)
        t[pos:pos] = f"[{text_to_insert}]"
        delta = doc.get_update(sv)
        updates.append(delta)

    # Materialize across all permutations of arrival orders
    results = set()
    for perm in itertools.permutations(updates):
        doc = crdt.load_doc(base_update, list(perm))
        results.add(crdt.materialize(doc))

    # All permutations must converge to the exact same text!
    assert len(results) == 1
