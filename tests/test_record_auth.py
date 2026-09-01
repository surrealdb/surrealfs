"""Optional record auth: SurrealDB enforcing the `file` permissions itself.

Every other test drives `SurrealFs` under a system credential, where table
permissions are bypassed and the Python layer is the only thing standing. These
tests take the Python layer out of the picture entirely and issue raw SurrealQL
over a record-authenticated connection -- which is exactly the attacker this
feature exists to stop.
"""

from __future__ import annotations

import pytest

from surrealfs import ROOT, SurrealFs, apply_schema
from surrealfs.users import add as add_user

SECRET = "the launch codes are hunter2"
PUBLIC = "the launch is on friday"


async def rows(connection, sql, variables=None):
    """Run one statement and return its rows, or raise on an error status."""
    from surrealfs.fs import raise_for_status

    return raise_for_status(await connection.query_raw(sql, variables or {}))[-1] or []


async def paths(connection, where=""):
    listed = await rows(connection, f"SELECT path FROM file {where} ORDER BY path")
    return [row["path"] for row in listed]


@pytest.fixture
async def tenants(db, signed_in):
    """Two provisioned record users, a private home each, and a shared folder."""
    await apply_schema(db, record_auth=True)
    await add_user(db, "alice", "pw-alice")
    await add_user(db, "bob", "pw-bob")

    root = SurrealFs(db, user=ROOT)
    await root.mkdir("/home/alice/notes", parents=True)
    await SurrealFs(db, user="alice").write_text("/home/alice/notes/secret.md", SECRET)
    await SurrealFs(db, user="bob").write_text("/projects/public.md", PUBLIC)

    return await signed_in("alice", "pw-alice"), await signed_in("bob", "pw-bob")


# ------------------------------------------------------------------ the leaks


async def test_a_record_user_cannot_select_another_home(tenants):
    alice, bob = tenants
    # Bob sees his own home and the shared tree. Alice's home is 0700, so even
    # its *name* is absent -- stricter than the Python layer's `ls /home`, and
    # stricter only ever in the safe direction. See docs/permissions.md.
    assert await paths(bob) == [
        "/home",
        "/home/bob",
        "/projects",
        "/projects/public.md",
    ]
    assert "/home/alice/notes/secret.md" in await paths(alice)


async def test_a_record_user_cannot_target_the_path_directly(tenants):
    _, bob = tenants
    assert await paths(bob, "WHERE path = '/home/alice/notes/secret.md'") == []
    assert (
        await rows(bob, "SELECT content FROM file WHERE filename = 'secret.md'") == []
    )


async def test_full_text_search_is_filtered_by_the_database(tenants):
    alice, bob = tenants
    assert await paths(bob, "WHERE content @1,OR@ 'launch'") == ["/projects/public.md"]
    assert len(await paths(alice, "WHERE content @1,OR@ 'launch'")) == 2


async def test_vector_search_is_filtered_by_the_database(tenants, db):
    alice, bob = tenants

    def vector(first: float) -> list[float]:
        return [first] + [0.01] * 1535

    for filename, first in (("secret.md", 0.99), ("public.md", 0.95)):
        await db.query_raw(
            "UPDATE file SET embedding = $v WHERE filename = $f",
            {"v": vector(first), "f": filename},
        )
    knn = "WHERE embedding <|5,40|> $q"
    assert await paths(bob, knn.replace("$q", str(vector(1.0)))) == [
        "/projects/public.md"
    ]
    assert len(await paths(alice, knn.replace("$q", str(vector(1.0))))) == 2


async def test_the_gate_recursion_holds_several_levels_deep(tenants, db):
    """The case that fails open if the clause is ever 'tidied' to use `gate`."""
    root = SurrealFs(db, user=ROOT)
    await root.mkdir("/home/alice/notes/a/b", parents=True)
    await SurrealFs(db, user="alice").write_text(
        "/home/alice/notes/a/b/deep.md", "buried treasure"
    )
    _, bob = tenants
    assert await paths(bob, "WHERE filename = 'deep.md'") == []
    assert await paths(bob, "WHERE content @1,OR@ 'treasure'") == []


# ----------------------------------------------------------------- the writes


async def test_a_record_user_cannot_write_into_another_home(tenants, db):
    _, bob = tenants
    notes = await rows(
        db, "SELECT VALUE id FROM ONLY file WHERE filename='notes' LIMIT 1"
    )
    await rows(bob, "UPDATE file SET content = 'pwned' WHERE filename = 'secret.md'")
    await rows(bob, "DELETE file WHERE filename = 'secret.md'")
    await rows(
        bob,
        "CREATE file SET filename='planted.md', parent=$p, owner='bob', "
        "mode=438, content='x', content_type='text/plain'",
        {"p": notes},
    )
    intact = await rows(db, "SELECT VALUE content FROM file WHERE filename='secret.md'")
    assert intact == [SECRET]
    assert await rows(db, "SELECT * FROM file WHERE filename='planted.md'") == []


async def test_a_record_user_cannot_create_a_file_owned_by_someone_else(tenants, db):
    _, bob = tenants
    await rows(
        bob,
        "CREATE file SET filename='fake.md', owner='alice', mode=438, "
        "content='x', content_type='text/plain'",
    )
    assert await rows(db, "SELECT * FROM file WHERE filename='fake.md'") == []


async def test_a_record_user_can_still_work_in_the_shared_tree(tenants, db):
    _, bob = tenants
    projects = await rows(
        db, "SELECT VALUE id FROM ONLY file WHERE filename='projects' LIMIT 1"
    )
    made = await rows(
        bob,
        "CREATE file SET filename='mine.md', parent=$p, owner='bob', "
        "mode=438, content='ok', content_type='text/plain' RETURN filename",
        {"p": projects},
    )
    assert made and made[0]["filename"] == "mine.md"


# --------------------------------------------------- the two layers together


async def test_surrealfs_behaves_the_same_over_a_record_connection(tenants, db):
    """The DB backstop must never contradict the Python layer."""
    from surrealfs import NotFound, PermissionDenied

    _, bob_connection = tenants
    over_record = SurrealFs(bob_connection, user="bob")
    over_system = SurrealFs(db, user="bob")

    for fs in (over_record, over_system):
        assert await fs.read_text("/projects/public.md") == PUBLIC
        assert [h.path for h in await fs.search_text("launch")] == [
            "/projects/public.md"
        ]
        # Both refuse. The *error* differs by design: under record auth the
        # database hides the intermediate folder, so the segment walk in
        # `_resolve` dead-ends and reports NotFound rather than resolving the
        # row and denying it. Less informative, never less safe.
        with pytest.raises((PermissionDenied, NotFound)):
            await fs.read_text("/home/alice/notes/secret.md")

    await over_record.write_text("/projects/from-bob.md", "hi")
    assert await over_system.read_text("/projects/from-bob.md") == "hi"


async def test_the_agent_surfaces_work_under_record_auth(
    db, surreal_url, namespace, monkeypatch
):
    """The documented record-auth setup has to actually run.

    `connected()` applied the schema on every call, which a record user has no
    DDL rights for -- and the flag is only set on success, so it failed on
    *every* call, not just the first: every Hermes tool call and every memory
    write, dead. Applying it is an admin's job under record auth.

    The identity has to come from the credential too. `agent_user()` used to
    return the machine account, so `SurrealFs` enforced one user's permissions
    over another user's rows -- and the denials surfaced as `IndexError` from
    unpacking an empty write result.
    """
    from surrealfs.integrations import _connect

    await apply_schema(db, record_auth=True)
    await add_user(db, "erin", "pw-erin")

    monkeypatch.setenv("SURREALDB_URL", surreal_url)
    monkeypatch.setenv("SURREALDB_NAMESPACE", namespace)
    monkeypatch.setenv("SURREALDB_DATABASE", "test")
    monkeypatch.setenv("SURREALDB_AUTH_LEVEL", "record")
    monkeypatch.setenv("SURREALDB_USER", "erin")
    monkeypatch.setenv("SURREALDB_PASS", "pw-erin")
    # Set, and outranked: the credential is what the database enforces.
    monkeypatch.setenv("SURREALFS_AGENT_USER", "not-me")
    monkeypatch.setattr(_connect, "_schema_applied", False)

    assert _connect.agent_user() == "erin"
    async with _connect.connected() as connection:
        fs = SurrealFs(connection, user=_connect.agent_user())
        await fs.write_text("/home/erin/notes.md", "mine")
        assert await fs.read_text("/home/erin/notes.md") == "mine"


def test_a_write_the_database_rejects_is_a_denial_not_an_indexerror():
    """A rejected write is not an error: SurrealDB returns an empty result.

    Only record auth can produce this -- a system credential bypasses the
    clause -- and `SurrealFs`'s own checks should have refused first, so it
    means the two disagreed about a row. It must still read as a denial.
    """
    from surrealfs.errors import PermissionDenied as Denied
    from surrealfs.fs import _written

    with pytest.raises(Denied, match="/a.md"):
        _written([], "/a.md")
    assert _written([{"filename": "a.md"}], "/a.md") == {"filename": "a.md"}


async def test_a_system_credential_still_sees_everything(tenants, db):
    root = SurrealFs(db, user=ROOT)
    assert await root.read_text("/home/alice/notes/secret.md") == SECRET


# --------------------------------------------------------------- provisioning


async def test_add_user_creates_a_private_home_owned_by_them(db):
    await apply_schema(db, record_auth=True)
    assert await add_user(db, "carol", "pw") == "/home/carol"
    home = await SurrealFs(db, user=ROOT).stat("/home/carol")
    assert home.owner == "carol"
    assert home.mode == 0o700


async def test_signin_fails_on_a_bad_password(db, signed_in):
    from surrealdb.errors import SurrealDBMethodError

    await apply_schema(db, record_auth=True)
    await add_user(db, "dave", "right")
    with pytest.raises(SurrealDBMethodError):
        await signed_in("dave", "wrong")
