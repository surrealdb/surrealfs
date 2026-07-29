"""Core filesystem behaviour, against a real SurrealDB server."""

from __future__ import annotations

import pytest

from surrealfs import (
    AlreadyExists,
    DirectoryNotEmpty,
    InvalidPath,
    IsADirectory,
    NotADirectory,
    NotATextFile,
    NotFound,
)


async def test_write_and_read_text(fs):
    entry = await fs.write_text("/notes/a.md", "hello world")
    assert entry.path == "/notes/a.md"
    assert entry.content_type == "text/markdown"
    assert await fs.read_text("/notes/a.md") == "hello world"


async def test_write_creates_missing_parents(fs):
    await fs.write_text("/a/b/c/d.md", "deep")
    assert [e.path for e in await fs.ls("/", recursive=True)] == [
        "/a",
        "/a/b",
        "/a/b/c",
        "/a/b/c/d.md",
    ]


async def test_write_replaces_existing(fs):
    await fs.write_text("/a.md", "first")
    await fs.write_text("/a.md", "second")
    assert await fs.read_text("/a.md") == "second"
    assert len(await fs.ls("/")) == 1


async def test_bytes_roundtrip(fs):
    data = bytes(range(256))
    entry = await fs.write_bytes("/img/x.png", data, content_type="image/png")
    assert entry.size == 256
    assert await fs.read_bytes("/img/x.png") == data
    with pytest.raises(NotATextFile):
        await fs.read_text("/img/x.png")


async def test_read_bytes_encodes_text(fs):
    await fs.write_text("/a.md", "héllo")
    assert await fs.read_bytes("/a.md") == "héllo".encode()


async def test_touch_creates_a_file_not_a_folder(fs):
    """content must be "" — a NONE-content row computes as is_folder."""
    entry = await fs.touch("/empty.txt")
    assert entry.is_folder is False
    assert entry.size == 0
    assert await fs.read_text("/empty.txt") == ""


async def test_touch_is_idempotent(fs):
    await fs.write_text("/a.md", "keep me")
    await fs.touch("/a.md")
    assert await fs.read_text("/a.md") == "keep me"


async def test_mkdir_requires_parents_flag(fs):
    with pytest.raises(NotFound):
        await fs.mkdir("/a/b/c")
    await fs.mkdir("/a/b/c", parents=True)
    assert (await fs.stat("/a/b/c")).is_folder


async def test_mkdir_rejects_existing(fs):
    await fs.mkdir("/a")
    with pytest.raises(AlreadyExists):
        await fs.mkdir("/a")


async def test_ls_root_and_folders(fs):
    await fs.write_text("/notes/a.md", "a")
    await fs.write_text("/b.md", "b")
    assert [e.path for e in await fs.ls("/")] == ["/b.md", "/notes"]
    assert [e.path for e in await fs.ls("/notes")] == ["/notes/a.md"]


async def test_ls_reports_sizes(fs):
    await fs.write_text("/a.md", "12345")
    await fs.write_bytes("/b.bin", b"\x00\x01\x02")
    await fs.mkdir("/d")
    sizes = {e.path: e.size for e in await fs.ls("/")}
    assert sizes == {"/a.md": 5, "/b.bin": 3, "/d": 0}


async def test_ls_on_a_file_is_an_error(fs):
    await fs.write_text("/a.md", "a")
    with pytest.raises(NotADirectory):
        await fs.ls("/a.md")


async def test_deep_path_resolution(fs):
    await fs.write_text("/a/b/c/d/e/f.md", "deep")
    assert await fs.read_text("/a/b/c/d/e/f.md") == "deep"
    assert (await fs.stat("/a/b/c/d/e/f.md")).path == "/a/b/c/d/e/f.md"


async def test_same_filename_under_different_parents(fs):
    await fs.write_text("/x/note.md", "one")
    await fs.write_text("/y/note.md", "two")
    assert await fs.read_text("/x/note.md") == "one"
    assert await fs.read_text("/y/note.md") == "two"


async def test_glob(fs):
    await fs.write_text("/notes/a.md", "a")
    await fs.write_text("/notes/deep/b.md", "b")
    await fs.write_text("/notes/c.txt", "c")
    await fs.write_text("/other/d.md", "d")
    assert [e.path for e in await fs.glob("/notes/**/*.md")] == [
        "/notes/a.md",
        "/notes/deep/b.md",
    ]
    assert [e.path for e in await fs.glob("/notes/*.md")] == ["/notes/a.md"]


async def test_edit_returns_a_diff(fs):
    await fs.write_text("/a.md", "hello world\nsecond line")
    diff = await fs.edit("/a.md", "world", "there")
    assert "-hello world" in diff
    assert "+hello there" in diff
    assert "second line" in diff
    assert await fs.read_text("/a.md") == "hello there\nsecond line"


async def test_edit_replace_all(fs):
    await fs.write_text("/a.md", "x x x")
    await fs.edit("/a.md", "x", "y")
    assert await fs.read_text("/a.md") == "y x x"
    await fs.edit("/a.md", "x", "y", replace_all=True)
    assert await fs.read_text("/a.md") == "y y y"


async def test_edit_missing_text_leaves_file_untouched(fs):
    await fs.write_text("/a.md", "original")
    with pytest.raises(NotFound):
        await fs.edit("/a.md", "absent", "x")
    assert await fs.read_text("/a.md") == "original"


async def test_mv_renames_and_cascades_to_descendants(fs):
    """`path` is COMPUTED, so a rename is one UPDATE and children follow."""
    await fs.write_text("/notes/deep/a.md", "a")
    await fs.mv("/notes", "/archive")
    assert [e.path for e in await fs.ls("/", recursive=True)] == [
        "/archive",
        "/archive/deep",
        "/archive/deep/a.md",
    ]
    assert await fs.read_text("/archive/deep/a.md") == "a"


async def test_mv_between_folders(fs):
    await fs.write_text("/a/x.md", "x")
    await fs.mkdir("/b")
    await fs.mv("/a/x.md", "/b/y.md")
    assert await fs.read_text("/b/y.md") == "x"
    assert not await fs.exists("/a/x.md")


async def test_mv_rejects_existing_destination(fs):
    await fs.write_text("/a.md", "a")
    await fs.write_text("/b.md", "b")
    with pytest.raises(AlreadyExists):
        await fs.mv("/a.md", "/b.md")


async def test_mv_rejects_moving_into_own_subtree(fs):
    await fs.mkdir("/a/b", parents=True)
    with pytest.raises(InvalidPath):
        await fs.mv("/a", "/a/b/c")


async def test_cp_file(fs):
    await fs.write_text("/a.md", "content")
    await fs.cp("/a.md", "/b/copy.md")
    assert await fs.read_text("/b/copy.md") == "content"
    assert await fs.read_text("/a.md") == "content"


async def test_cp_preserves_binary(fs):
    await fs.write_bytes("/x.png", b"\x89PNG\x00", content_type="image/png")
    entry = await fs.cp("/x.png", "/y.png")
    assert entry.content_type == "image/png"
    assert await fs.read_bytes("/y.png") == b"\x89PNG\x00"


async def test_cp_recursive(fs):
    await fs.write_text("/src/a.md", "a")
    await fs.write_text("/src/deep/b.md", "b")
    await fs.cp("/src", "/dst", recursive=True)
    assert [e.path for e in await fs.ls("/dst", recursive=True)] == [
        "/dst/a.md",
        "/dst/deep",
        "/dst/deep/b.md",
    ]
    assert await fs.read_text("/dst/deep/b.md") == "b"


async def test_cp_folder_without_recursive_is_an_error(fs):
    await fs.mkdir("/src")
    with pytest.raises(IsADirectory):
        await fs.cp("/src", "/dst")


async def test_rm_file(fs):
    await fs.write_text("/a.md", "a")
    assert await fs.rm("/a.md") == 1
    assert not await fs.exists("/a.md")


async def test_rm_non_empty_folder_needs_recursive(fs):
    await fs.write_text("/a/b.md", "b")
    with pytest.raises(DirectoryNotEmpty):
        await fs.rm("/a")
    assert await fs.exists("/a/b.md")


async def test_rm_recursive_removes_only_the_subtree(fs):
    await fs.write_text("/keep.md", "keep")
    await fs.write_text("/gone/deep/a.md", "a")
    assert await fs.rm("/gone", recursive=True) == 3
    assert [e.path for e in await fs.ls("/", recursive=True)] == ["/keep.md"]


async def test_rm_frees_the_name_for_reuse(fs):
    """A hard delete, not a tombstone — the UNIQUE index must not block reuse."""
    await fs.write_text("/a.md", "first")
    await fs.rm("/a.md")
    await fs.write_text("/a.md", "second")
    assert await fs.read_text("/a.md") == "second"


async def test_missing_paths_raise_not_found(fs):
    with pytest.raises(NotFound):
        await fs.read_text("/nope.md")
    with pytest.raises(NotFound):
        await fs.stat("/nope.md")
    with pytest.raises(NotFound):
        await fs.rm("/nope.md")


async def test_reading_a_folder_is_an_error(fs):
    await fs.mkdir("/d")
    with pytest.raises(IsADirectory):
        await fs.read_text("/d")


async def test_tail(fs):
    await fs.write_text("/log.txt", "\n".join(str(i) for i in range(1, 21)))
    assert await fs.tail("/log.txt", 3) == "18\n19\n20"
    assert len((await fs.tail("/log.txt")).splitlines()) == 10


async def test_hash_is_maintained_by_the_schema_event(fs):
    import hashlib

    await fs.write_text("/a.md", "hello")
    assert (await fs.stat("/a.md")).hash == hashlib.md5(b"hello").hexdigest()
    await fs.write_text("/a.md", "goodbye")
    assert (await fs.stat("/a.md")).hash == hashlib.md5(b"goodbye").hexdigest()


async def test_duplicate_names_are_rejected_at_the_root(fs):
    """A composite UNIQUE index skips rows with a NONE component, so the schema
    indexes `parent_key` ('root') rather than `parent` (NONE)."""
    from surrealfs.errors import QueryError

    await fs.write_text("/a.md", "first")
    with pytest.raises(QueryError, match="already contains"):
        await fs._query(
            "CREATE file CONTENT "
            "{filename: 'a.md', content_type: 'text/markdown', content: 'dup'}"
        )
    assert len(await fs.ls("/")) == 1


async def test_duplicate_names_are_rejected_in_a_folder(fs):
    from surrealfs.errors import QueryError

    await fs.write_text("/d/a.md", "first")
    parent = await fs.stat("/d")
    with pytest.raises(QueryError, match="already contains"):
        await fs._query(
            "CREATE file CONTENT {filename: 'a.md', parent: $p, "
            "content_type: 'text/markdown', content: 'dup'}",
            {"p": parent.id},
        )


async def test_missing_intermediate_segment_does_not_fall_back_to_root(fs):
    """Resolving /ghost/a.md must not find /a.md."""
    await fs.write_text("/a.md", "at the root")
    assert not await fs.exists("/ghost/a.md")
    with pytest.raises(NotFound):
        await fs.read_text("/ghost/a.md")


async def test_parent_key_follows_a_move(fs, db):
    """The stored key is a VALUE field, so it must be re-evaluated on update."""
    await fs.write_text("/a.md", "x")
    await fs.mkdir("/d")
    moved = await fs.mv("/a.md", "/d/a.md")
    # query() returns one entry per statement, each holding that statement's rows.
    (rows,) = await db.query(
        "SELECT parent_key FROM file WHERE id = $id", {"id": moved.id}
    )
    assert rows[0]["parent_key"] == str((await fs.stat("/d")).id)
    # And the name is free again at the root.
    await fs.write_text("/a.md", "y")
    assert await fs.read_text("/a.md") == "y"
    assert await fs.read_text("/d/a.md") == "x"


async def test_query_errors_are_not_swallowed(fs):
    """The SDK's query() hides failures in later statements; ours must not."""
    from surrealfs.errors import QueryError

    with pytest.raises(QueryError):
        await fs._query("LET $x = 1; THROW 'boom';")
