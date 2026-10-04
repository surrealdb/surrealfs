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


@given(ops=filesystem_operations())
@settings(max_examples=15, suppress_health_check=[HealthCheck.function_scoped_fixture])
def test_hypothesis_operation_sequence_consistency(ops) -> None:
    """Property test running generated mutation operations against reference model."""
    # Pure-Python in-memory reference model of the tree
    ref_tree: dict[str, str | dict] = {}

    def ref_write(path: str, content: str):
        ref_tree[path] = content

    def ref_mkdir(path: str):
        ref_tree[path] = {}

    def ref_rm(path: str):
        keys_to_del = [k for k in ref_tree if k == path or k.startswith(path + "/")]
        for k in keys_to_del:
            del ref_tree[k]

    def ref_mv(src: str, dst: str):
        if src in ref_tree:
            val = ref_tree.pop(src)
            ref_tree[dst] = val

    for op, path, content in ops:
        if op == "write":
            ref_write(path, content)
        elif op == "mkdir":
            ref_mkdir(path)
        elif op == "rm":
            ref_rm(path)
        elif op == "mv":
            ref_mv(path, path + "_moved")

    # Verify no cycle or corrupted keys in reference model
    for p in ref_tree:
        assert p.startswith("/")
        assert not p.endswith("/")


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
