"""Tests for atomic in-database SurrealQL functions and subsystem tables."""

from __future__ import annotations

import pytest

from surrealfs import ConflictError, NotFound, SurrealFs, apply_schema
from surrealfs.fs import raise_for_status
from tests.conftest import ROOT, _namespace_for


async def test_resolve_and_stat(db):
    await apply_schema(db)
    res = await db.query_raw("""
        CREATE file:nested SET
            filename = 'alice',
            parent = file:home,
            content_type = 'inode/directory';
        RETURN [
            fn::sfs_resolve('/'),
            fn::sfs_resolve('/home'),
            fn::sfs_resolve('/home/alice'),
            fn::sfs_resolve('/home/nonexistent'),
            fn::sfs_resolve('/ghost/a.md')
        ];
    """)
    out = raise_for_status(res)[-1]
    assert out[0] is None  # root is virtual
    assert out[1].id == "home"
    assert out[2].id == "nested"
    assert out[3] is None
    assert out[4] is None

    # Stat root
    stat_raw = await db.query_raw("RETURN fn::sfs_stat('/');")
    stat_root = raise_for_status(stat_raw)[-1]
    assert stat_root["path"] == "/"
    assert stat_root["is_folder"] is True

    # Stat home
    stat_home_raw = await db.query_raw("RETURN fn::sfs_stat('/home');")
    stat_home = raise_for_status(stat_home_raw)[-1]
    assert stat_home["filename"] == "home"
    assert stat_home["path"] == "/home"

    # Stat nonexistent raises NotFound
    with pytest.raises(NotFound):
        raise_for_status(await db.query_raw("RETURN fn::sfs_stat('/missing');"))


async def test_ls_function(db):
    await apply_schema(db)
    await db.query_raw("""
        CREATE file:f1 SET
            filename = 'a.txt',
            parent = file:home,
            content_type = 'text/plain',
            content = 'hello';
        CREATE file:f2 SET
            filename = 'b.txt',
            parent = file:home,
            content_type = 'text/plain',
            content = 'world';
    """)
    res = await db.query_raw("RETURN fn::sfs_ls('/home');")
    items = raise_for_status(res)[-1]
    filenames = {item["filename"] for item in items}
    assert filenames == {"a.txt", "b.txt"}

    # ls on empty / non-folder / missing
    with pytest.raises(NotFound):
        raise_for_status(await db.query_raw("RETURN fn::sfs_ls('/nonexistent');"))


async def test_append_function(db):
    await apply_schema(db)
    res_create = await db.query_raw("""
        CREATE file:log SET
            filename = 'test.log',
            parent = file:home,
            content_type = 'text/plain',
            content = 'line 1\n',
            owner = 'root',
            mode = 511;
    """)
    raise_for_status(res_create)

    # Append without if_generation
    res1 = await db.query_raw("RETURN fn::sfs_append(file:log, 'line 2\n', NONE);")
    row1 = raise_for_status(res1)[-1]
    assert row1["content"] == "line 1\nline 2\n"
    gen = row1["generation"]

    # Append with matching if_generation
    res2 = await db.query_raw(
        "RETURN fn::sfs_append(file:log, 'line 3\n', $gen);", {"gen": gen}
    )
    row2 = raise_for_status(res2)[-1]
    assert row2["content"] == "line 1\nline 2\nline 3\n"
    assert row2["generation"] == gen + 1

    # Append with wrong if_generation raises ConflictError
    with pytest.raises(ConflictError):
        res3 = await db.query_raw("RETURN fn::sfs_append(file:log, 'line 4\n', 9999);")
        raise_for_status(res3)


async def test_lock_lifecycle(db):
    await apply_schema(db)

    # Alice acquires lock on /home
    res1 = await db.query_raw(
        "RETURN fn::sfs_acquire_lock('/home', 'alice', 60, 'refactoring');"
    )
    lock1 = raise_for_status(res1)[-1]
    assert lock1["holder"] == "alice"
    assert lock1["reason"] == "refactoring"

    # Bob cannot acquire lock while active
    with pytest.raises(ConflictError):
        res_bob = await db.query_raw(
            "RETURN fn::sfs_acquire_lock('/home', 'bob', 60, 'stealing');"
        )
        raise_for_status(res_bob)

    # Alice re-acquires / refreshes lease successfully
    res_alice2 = await db.query_raw(
        "RETURN fn::sfs_acquire_lock('/home', 'alice', 120, 'extended');"
    )
    lock2 = raise_for_status(res_alice2)[-1]
    assert lock2["holder"] == "alice"
    assert lock2["reason"] == "extended"

    # Bob cannot release Alice's lock
    with pytest.raises(ConflictError):
        res_bob_rel = await db.query_raw("RETURN fn::sfs_release_lock('/home', 'bob');")
        raise_for_status(res_bob_rel)

    # Alice releases lock
    res_rel = await db.query_raw("RETURN fn::sfs_release_lock('/home', 'alice');")
    assert raise_for_status(res_rel)[-1] is True

    # Now Bob can acquire
    res_bob_ok = await db.query_raw(
        "RETURN fn::sfs_acquire_lock('/home', 'bob', 30, 'mine now');"
    )
    assert raise_for_status(res_bob_ok)[-1]["holder"] == "bob"


async def test_chmod_function(db):
    await apply_schema(db)
    res_create = await db.query_raw("""
        CREATE file:perm SET
            filename = 'perm.txt',
            parent = file:home,
            content_type = 'text/plain',
            content = 'safe',
            owner = 'root',
            mode = 448; -- 0o700
    """)
    raise_for_status(res_create)

    res = await db.query_raw("RETURN fn::sfs_chmod('/home/perm.txt', 511);")
    assert raise_for_status(res)[-1] is True

    stat_raw = await db.query_raw("RETURN fn::sfs_stat('/home/perm.txt');")
    stat = raise_for_status(stat_raw)[-1]
    assert stat["mode"] == 511


async def test_version_event_capture(surreal_url, request):
    from surrealdb import AsyncSurreal

    db = AsyncSurreal(surreal_url)
    await db.signin(ROOT)
    name = _namespace_for(request)
    await db.use(name, "test")
    await apply_schema(db)

    # Create a file
    fs = SurrealFs(db, user="root")
    entry = await fs.write_text("/tracked.txt", "v1 content")
    gen1 = entry.generation

    # Update content -> triggers generation bump -> evt_file_version snapshots gen1
    entry2 = await fs.write_text("/tracked.txt", "v2 content")
    gen2 = entry2.generation
    assert gen2 > gen1

    # Query file_version
    res = await db.query_raw(
        "SELECT * FROM file_version WHERE path = '/tracked.txt' "
        "ORDER BY generation ASC;"
    )
    versions = raise_for_status(res)[-1]
    assert len(versions) == 1
    assert versions[0]["generation"] == gen1
    assert versions[0]["content"] == "v1 content"

    # Update content again -> snapshot of gen2
    await fs.write_text("/tracked.txt", "v3 content")
    res2 = await db.query_raw(
        "SELECT * FROM file_version WHERE path = '/tracked.txt' "
        "ORDER BY generation ASC;"
    )
    versions2 = raise_for_status(res2)[-1]
    assert len(versions2) == 2
    assert versions2[1]["generation"] == gen2
    assert versions2[1]["content"] == "v2 content"


async def test_workspace_and_branches(db):
    await apply_schema(db)
    res = await db.query_raw("""
        CREATE workspace:dev SET
            name = 'dev-workspace',
            owner = 'root',
            is_public = true;
        CREATE file:base SET
            filename = 'main.py',
            parent = file:home,
            content_type = 'text/x-python',
            content = 'print(1)';
        CREATE file_branch:b1 SET
            workspace = workspace:dev,
            base_file = file:base,
            path = '/home/main.py';
        SELECT name, owner FROM workspace;
        SELECT workspace, path FROM file_branch;
    """)
    out = raise_for_status(res)
    workspaces = out[-2]
    branches = out[-1]
    assert workspaces[0]["name"] == "dev-workspace"
    assert branches[0]["path"] == "/home/main.py"
