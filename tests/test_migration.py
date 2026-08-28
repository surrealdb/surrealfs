"""A database that already holds files, before the permissions schema.

The DDL is self-applying -- every statement is OVERWRITE or IF NOT EXISTS --
but rows written before `owner` and `mode` existed are not. The backfill for
those lives in `docs/migrate_permissions.surql` and is run by hand; there are
no released users, so the only database it applies to is the author's own.

What has to be tested from Python is the state the backfill runs *through*:
a row with no mode must be opaque, not fatal, or the migration cannot run at
all. Delete this file along with the coalescing in `fn::sfs_gate`,
`fn::sfs_bits` and `FileEntry.from_row` once no such database is left.

A legacy row is made here the way the database really produced one: drop the
field definitions, write the row, then re-apply the schema.
"""

from __future__ import annotations

import pytest

from surrealfs import ROOT, PermissionDenied, SurrealFs, apply_schema


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
