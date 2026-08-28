"""Rolling the permissions schema onto a database that already holds files.

The DDL is self-applying -- every statement is OVERWRITE or IF NOT EXISTS --
but rows written before `owner` and `mode` existed are not. Those rows are the
whole reason `migrate_permissions.surql` exists, and an existing `/home/martin`
staying world-readable would defeat the point of the release.

A legacy row is made here the way the database really produced one: drop the
field definitions, write the row, then re-apply the schema.
"""

from __future__ import annotations

import pytest

from surrealfs import ROOT, PermissionDenied, SurrealFs, apply_schema
from surrealfs.schema import migrate


async def make_legacy(db) -> None:
    """Seed rows with no `owner` and no `mode`, as the previous release wrote."""
    await db.query_raw("REMOVE FIELD mode ON file; REMOVE FIELD owner ON file;")
    await db.query_raw("""
        LET $home = (CREATE ONLY file SET filename='home',
                     content_type='inode/directory');
        LET $m = (CREATE ONLY file SET filename='martin', parent=$home.id,
                  content_type='inode/directory');
        LET $n = (CREATE ONLY file SET filename='notes', parent=$m.id,
                  content_type='inode/directory');
        CREATE file SET filename='diary.md', parent=$n.id,
               content_type='text/markdown', content='personal thoughts';
        LET $p = (CREATE ONLY file SET filename='projects',
                  content_type='inode/directory');
        CREATE file SET filename='plan.md', parent=$p.id,
               content_type='text/markdown', content='shared plan';
    """)
    await apply_schema(db)  # restores the field definitions, rows untouched


async def test_an_unmigrated_row_is_opaque_rather_than_fatal(db):
    """A missing mode must not take the database down, nor expose the row.

    `NONE % 2` raises, and `gate` is recomputed whenever a row is rewritten --
    so a naive version makes every read *and the migration itself* fail. Nor can
    a missing mode read as traversable: that would expose the un-migrated tree.
    """
    await make_legacy(db)
    root = SurrealFs(db, user=ROOT)
    assert await root.read_text("/home/martin/notes/diary.md") == "personal thoughts"

    martin = SurrealFs(db, user="martin")
    with pytest.raises((PermissionDenied, FileNotFoundError)):
        await martin.read_text("/home/martin/notes/diary.md")
    assert [h.path for h in await martin.search_text("personal")] == []


async def test_migrate_backfills_owner_and_mode(db):
    await make_legacy(db)
    await migrate(db)
    root = SurrealFs(db, user=ROOT)

    shared = await root.stat("/projects/plan.md")
    assert (shared.owner, shared.mode) == ("root", 0o666)
    assert (await root.stat("/projects")).mode == 0o777


async def test_migrate_closes_existing_homes(db):
    """The step that actually secures a tree that was open before."""
    await make_legacy(db)
    await migrate(db)

    root = SurrealFs(db, user=ROOT)
    home = await root.stat("/home/martin")
    assert (home.owner, home.mode) == ("martin", 0o700)
    # And the contents belong to them, not to root -- otherwise they could not
    # chmod their own files.
    assert (await root.stat("/home/martin/notes/diary.md")).owner == "martin"

    martin = SurrealFs(db, user="martin")
    assert await martin.read_text("/home/martin/notes/diary.md") == "personal thoughts"
    await martin.chmod("/home/martin/notes/diary.md", 0o600)

    hermes = SurrealFs(db, user="hermes")
    with pytest.raises((PermissionDenied, FileNotFoundError)):
        await hermes.read_text("/home/martin/notes/diary.md")
    assert [h.path for h in await hermes.search_text("personal")] == []
    # The shared tree stays shared.
    assert await hermes.read_text("/projects/plan.md") == "shared plan"


async def test_migrate_is_safe_to_re_run(db):
    await make_legacy(db)
    await migrate(db)
    martin = SurrealFs(db, user="martin")
    await martin.chmod("/home/martin/notes/diary.md", 0o600)

    await migrate(db)

    assert (await martin.stat("/home/martin/notes/diary.md")).mode == 0o600
    assert (await martin.stat("/home/martin")).mode == 0o700


async def test_migrate_does_nothing_to_a_fresh_database(db):
    fs = SurrealFs(db, user=ROOT)
    await fs.write_text("/projects/plan.md", "shared plan")
    before = await fs.stat("/projects/plan.md")

    await migrate(db)

    after = await fs.stat("/projects/plan.md")
    assert (after.owner, after.mode) == (before.owner, before.mode)


async def test_migrate_without_a_home_folder_leaves_top_level_alone(db):
    """`parent = NONE` matches every root row, so the guard has to hold."""
    await make_legacy(db)
    await db.query_raw("DELETE file WHERE filename = 'home'")
    await migrate(db)

    projects = await SurrealFs(db, user=ROOT).stat("/projects")
    assert (projects.owner, projects.mode) == ("root", 0o777)


async def test_migrate_retires_the_legacy_user_table(db):
    """The old `--include-user` schema required an email; provisioning breaks."""
    await db.query_raw("""
        DEFINE TABLE OVERWRITE user SCHEMAFULL;
        DEFINE FIELD OVERWRITE email ON TABLE user TYPE string;
        DEFINE INDEX OVERWRITE user_email_unique ON TABLE user FIELDS email UNIQUE;
    """)
    await apply_schema(db, record_auth=True)
    await migrate(db)

    from surrealfs.users import add as add_user

    assert await add_user(db, "alice", "pw") == "/home/alice"
