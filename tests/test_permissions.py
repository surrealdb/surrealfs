"""Unix permissions: `/home/<user>` is private, everything else is shared.

The tests that matter most are the leak tests. A missed filter in `search`
hands another user's file *content* back to a model, so each of the four bulk
read paths -- ls, glob, search_text, search_semantic -- is asserted to return
nothing for a user who should not see it.
"""

from __future__ import annotations

import pytest

from surrealfs import FileEntry, PermissionDenied, format_mode

SECRET = "the launch codes are hunter2"
PUBLIC = "the launch is on friday"


@pytest.fixture
async def tree(as_user):
    """Alice's private home and a shared folder, seeded by their real owners."""
    alice = as_user("alice")
    bob = as_user("bob")
    await alice.mkdir("/home/alice/notes", parents=True)
    await alice.write_text("/home/alice/notes/secret.md", SECRET)
    await bob.mkdir("/projects", parents=True)
    await bob.write_text("/projects/public.md", PUBLIC)
    return alice, bob


# --------------------------------------------------------------- the defaults


async def test_a_home_directory_is_created_private(tree):
    alice, _ = tree
    assert (await alice.stat("/home/alice")).mode == 0o700
    assert (await alice.stat("/home/alice")).permissions == "drwx------"


async def test_everything_outside_home_is_shared(tree):
    alice, bob = tree
    assert (await bob.stat("/projects")).mode == 0o777
    assert (await bob.stat("/projects/public.md")).mode == 0o666
    # Shared means shared: alice can read *and* write bob's folder.
    assert await alice.read_text("/projects/public.md") == PUBLIC
    await alice.write_text("/projects/from-alice.md", "hi")
    assert (await alice.stat("/projects/from-alice.md")).owner == "alice"


async def test_home_itself_is_traversable(tree):
    alice, bob = tree
    # /home is 0777 -- bob can list it and see that alice has a home, exactly
    # as `ls /home` does on a real machine. He just cannot go in.
    assert [e.filename for e in await bob.ls("/home")] == ["alice"]


# ------------------------------------------------------------- the leak tests


async def test_another_home_cannot_be_read(tree):
    _, bob = tree
    with pytest.raises(PermissionDenied):
        await bob.read_text("/home/alice/notes/secret.md")
    with pytest.raises(PermissionDenied):
        await bob.stat("/home/alice/notes/secret.md")
    with pytest.raises(PermissionDenied):
        await bob.ls("/home/alice")


async def test_exists_does_not_confirm_a_name_in_another_home(tree):
    """`exists` skipped the gate check every other read goes through.

    It resolved the path directly, so it answered True for a file `stat` on the
    same path refuses to describe -- confirming the *name* of something inside
    alice's private home. False is what `os.path.exists` gives on EACCES.
    """
    alice, bob = tree
    assert await alice.exists("/home/alice/notes/secret.md")
    assert not await bob.exists("/home/alice/notes/secret.md")
    with pytest.raises(PermissionDenied):
        await bob.stat("/home/alice/notes/secret.md")
    # Not a blanket False: the shared tree still answers, and so does the home
    # itself, which `ls /home` shows anyway.
    assert await bob.exists("/projects/public.md")
    assert await bob.exists("/home/alice")


async def test_search_does_not_leak_another_home(tree):
    """The worst path: full-text search returns content, not just a path."""
    alice, bob = tree
    assert [h.path for h in await bob.search_text("launch")] == ["/projects/public.md"]
    assert sorted(h.path for h in await alice.search_text("launch")) == [
        "/home/alice/notes/secret.md",
        "/projects/public.md",
    ]
    assert all(
        SECRET not in (h.entry.content or "") for h in await bob.search_text("launch")
    )


async def test_semantic_search_does_not_leak_another_home(tree, db):
    """The other indexed read path: `gate` must survive an HNSW lookup too."""
    alice, bob = tree

    def vector(first: float) -> list[float]:
        return [first] + [0.01] * 1535

    for filename, first in (("secret.md", 0.99), ("public.md", 0.95)):
        await db.query_raw(
            "UPDATE file SET embedding = $v WHERE filename = $f",
            {"v": vector(first), "f": filename},
        )
    query = vector(1.0)
    assert [h.path for h in await bob.search_semantic(query, k=5)] == [
        "/projects/public.md"
    ]
    assert len(await alice.search_semantic(query, k=5)) == 2


async def test_glob_and_recursive_ls_do_not_leak(tree):
    _, bob = tree
    assert await bob.glob("/home/**") == []
    listed = [e.path for e in await bob.ls("/", recursive=True)]
    assert "/home/alice" in listed  # the home itself is visible, like `ls /home`
    assert not any(p.startswith("/home/alice/") for p in listed)


async def test_a_file_is_hidden_by_its_ancestor_not_its_own_bits(tree, as_user):
    """0666 inside 0700 is still unreachable -- the whole point of `gate`."""
    alice, bob = tree
    assert (await alice.stat("/home/alice/notes/secret.md")).mode == 0o666
    with pytest.raises(PermissionDenied):
        await bob.read_text("/home/alice/notes/secret.md")


async def test_every_closed_ancestor_gates_not_just_the_outermost(tree, fs):
    """`gate` collapses the whole chain, so it must consult every link in it.

    Short-circuiting on the parent's inherited `gate` reported the *outermost*
    closed ancestor: bob closing his own `/projects` then made the gate on
    everything below it say "bob", and handed him alice's 0700 folder inside.
    """
    alice, bob = tree
    await alice.mkdir("/projects/secret")
    await alice.write_text("/projects/secret/notes.md", SECRET)
    await alice.chmod("/projects/secret", 0o700)

    # One closure, alice's: she gets in, bob does not.
    assert await alice.read_text("/projects/secret/notes.md") == SECRET
    with pytest.raises(PermissionDenied):
        await bob.read_text("/projects/secret/notes.md")

    # Two closures, two owners: neither can traverse both, so the gate matches
    # nobody -- including the owner of the outer folder.
    await bob.chmod("/projects", 0o700)
    assert (await fs.stat("/projects/secret/notes.md")).gate == "!closed"
    for user in (alice, bob):
        with pytest.raises(PermissionDenied):
            await user.read_text("/projects/secret/notes.md")
        # Not just `read_text`: the bulk read paths share `fn::sfs_can_read`.
        # (Alice's own copy in her home is a legitimate hit for her.)
        hits = [h.path for h in await user.search_text("hunter2")]
        assert "/projects/secret/notes.md" not in hits
    # Root still gets through, as it does on a real machine.
    assert await fs.read_text("/projects/secret/notes.md") == SECRET


# -------------------------------------------------------------------- writing


async def test_another_home_cannot_be_written(tree):
    _, bob = tree
    with pytest.raises(PermissionDenied):
        await bob.write_text("/home/alice/notes/planted.md", "x")
    with pytest.raises(PermissionDenied):
        await bob.mkdir("/home/alice/sneaky")
    with pytest.raises(PermissionDenied):
        await bob.rm("/home/alice/notes/secret.md")


async def test_root_seeding_a_home_does_not_lock_its_user_out(fs, as_user):
    """A home belongs to the user it is named after, whoever runs the mkdir.

    There is no `chown`, so if the indexer or the browser created `/home/alice`
    first and kept it, alice could never read her own home again.
    """
    await fs.mkdir("/home/alice/notes", parents=True)  # fs is root
    assert (await fs.stat("/home/alice")).owner == "alice"
    alice = as_user("alice")
    await alice.write_text("/home/alice/notes/mine.md", "mine")
    assert await alice.read_text("/home/alice/notes/mine.md") == "mine"


async def test_home_itself_belongs_to_root(as_user):
    """`/home` is seeded root-owned by the schema, and no user may take it.

    `_guard_home` only guards the level *below* `/home`, and `/` is not a row
    with an owner to deny anything -- so without the seed the first user to
    `mkdir -p /home/<name>` owned `/home` at 0777, and could then `chmod 700`
    it: everyone else locked out of their own home, and their files gated to
    the squatter. Removing or renaming it re-opens the same hole, so those are
    guarded too.
    """
    bob = as_user("bob")
    home = await bob.stat("/home")
    assert (home.owner, home.mode) == ("root", 0o777)
    for hijack in (
        bob.chmod("/home", 0o700),
        bob.rm("/home", recursive=True),
        bob.mv("/home", "/hijacked"),
    ):
        with pytest.raises(PermissionDenied):
            await hijack


async def test_a_home_cannot_be_squatted(as_user):
    bob = as_user("bob")
    with pytest.raises(PermissionDenied):
        await bob.mkdir("/home/alice", parents=True)
    # His own is fine.
    await bob.mkdir("/home/bob", parents=True)
    assert (await bob.stat("/home/bob")).owner == "bob"


async def test_rm_is_keyed_on_the_parent_folder_not_the_file(tree, as_user):
    """A file you cannot write, in a folder you can, is still yours to remove."""
    alice, bob = tree
    await alice.write_text("/projects/readonly.md", "x")
    await alice.chmod("/projects/readonly.md", 0o444)
    with pytest.raises(PermissionDenied):
        await bob.write_text("/projects/readonly.md", "y")
    assert await bob.rm("/projects/readonly.md") == 1


async def test_recursive_rm_cannot_delete_a_subtree_it_cannot_see(tree, fs):
    """`rm -r` checked only the parent folder, so it destroyed private subtrees.

    `ls` names a folder's children whatever their own bits say, so alice's 0700
    folder was in bob's delete list -- while its contents, which `ls` refuses to
    descend into, were not. Bob deleted a folder he could not open and left the
    file inside it with a dangling parent: gone from every listing, its computed
    `path` collapsed to the top level, unreachable to alice and to root.
    """
    alice, bob = tree
    await alice.mkdir("/projects/secret")
    await alice.write_text("/projects/secret/notes.md", SECRET)
    await alice.chmod("/projects/secret", 0o700)

    with pytest.raises(PermissionDenied):
        await bob.rm("/projects", recursive=True)
    # Straight at it, too -- the folder looks empty to bob, which is not the
    # same as being empty.
    with pytest.raises(PermissionDenied):
        await bob.rm("/projects/secret", recursive=True)
    # All-or-nothing: bob's own files in /projects are still there.
    assert await bob.read_text("/projects/public.md") == PUBLIC
    assert await alice.read_text("/projects/secret/notes.md") == SECRET

    # 0733 is the subtle one: bob may unlink entries but cannot enumerate them,
    # so `rm -r` would orphan whatever it never listed.
    await alice.chmod("/projects/secret", 0o733)
    with pytest.raises(PermissionDenied):
        await bob.rm("/projects/secret", recursive=True)

    # Opened up, it is bob's to remove -- and alice's owner bits never blocked
    # her own.
    await alice.chmod("/projects/secret", 0o777)
    assert await bob.rm("/projects/secret", recursive=True) == 2
    assert [e.path for e in await fs.ls("/projects", recursive=True)] == [
        "/projects/public.md"
    ]


async def test_recursive_cp_does_not_silently_skip_what_it_cannot_read(tree, fs):
    """`cp -r` walked the same filtered `ls`, so it reported a partial copy.

    Alice's 0700 folder was listed but not descended into, so bob got a
    "backup" containing an empty `/backup/secret` and no error -- the worst
    shape for this bug, since the caller believes it has a copy. A 0600 file in
    a shared folder was worse still: `cp` never checked the read bit, so it
    handed bob the content in a file of his own.
    """
    alice, bob = tree
    await alice.mkdir("/projects/secret")
    await alice.write_text("/projects/secret/notes.md", SECRET)
    await alice.chmod("/projects/secret", 0o700)

    with pytest.raises(PermissionDenied):
        await bob.cp("/projects", "/backup", recursive=True)
    with pytest.raises(PermissionDenied):
        await bob.cp("/projects/secret", "/backup", recursive=True)
    # Nothing written at the destination, not even the folder.
    assert not await fs.exists("/backup")

    # A file bob may not read is not copyable either, whatever the folder says.
    await alice.chmod("/projects/secret", 0o777)
    await alice.chmod("/projects/secret/notes.md", 0o600)
    with pytest.raises(PermissionDenied):
        await bob.cp("/projects", "/backup", recursive=True)
    assert not await fs.exists("/backup")

    # Readable throughout: a real copy, with everything in it.
    await alice.chmod("/projects/secret/notes.md", 0o666)
    await bob.cp("/projects", "/backup", recursive=True)
    assert await bob.read_text("/backup/secret/notes.md") == SECRET


async def test_a_copied_folder_keeps_its_mode(tree):
    """`cp -r` recreated every folder with a bare `mkdir`, so the copy came out
    at `default_mode` -- 0777 -- however private the original was, and handed
    its contents to everyone."""
    alice, bob = tree
    await alice.mkdir("/projects/work/private", parents=True)
    await alice.write_text("/projects/work/private/notes.md", SECRET)
    await alice.chmod("/projects/work/private", 0o700)
    await alice.chmod("/projects/work", 0o750)
    with pytest.raises(PermissionDenied):
        await bob.read_text("/projects/work/private/notes.md")

    await alice.cp("/projects/work", "/projects/copy", recursive=True)
    # The copy root and the folder inside it, not just the files.
    assert (await alice.stat("/projects/copy")).mode == 0o750
    assert (await alice.stat("/projects/copy/private")).mode == 0o700
    assert await alice.read_text("/projects/copy/private/notes.md") == SECRET
    with pytest.raises(PermissionDenied):
        await bob.read_text("/projects/copy/private/notes.md")


# ---------------------------------------------------------------------- chmod


async def test_chmod_hides_and_reveals_a_shared_folder(tree, as_user):
    alice, bob = tree
    await alice.mkdir("/projects/private")
    await alice.write_text("/projects/private/plan.md", "the plan")
    assert await bob.read_text("/projects/private/plan.md") == "the plan"

    await alice.chmod("/projects/private", 0o700)
    with pytest.raises(PermissionDenied):
        await bob.read_text("/projects/private/plan.md")
    assert [h.path for h in await bob.search_text("plan")] == []

    await alice.chmod("/projects/private", 0o777)
    assert await bob.read_text("/projects/private/plan.md") == "the plan"


async def test_only_the_owner_may_chmod(tree):
    """Anyone who could chmod a folder could open it, so only the owner may."""
    alice, bob = tree
    # The file is world-writable, and alice still may not change its bits.
    assert (await bob.stat("/projects/public.md")).owner == "bob"
    with pytest.raises(PermissionDenied):
        await alice.chmod("/projects/public.md", 0o600)
    await bob.chmod("/projects/public.md", 0o644)
    assert (await bob.stat("/projects/public.md")).mode == 0o644


async def test_chmod_recursive(tree, as_user):
    alice, bob = tree
    await alice.mkdir("/projects/tree/inner", parents=True)
    await alice.write_text("/projects/tree/inner/a.md", "a")
    assert await alice.chmod("/projects/tree", 0o700, recursive=True) == 3
    assert (await alice.stat("/projects/tree/inner/a.md")).mode == 0o700


async def test_chmod_recursive_skips_what_it_does_not_own(tree, fs, as_user):
    """`chmod -R` used to refuse the whole tree over one foreign file.

    A shared folder legitimately holds other people's work -- alice writing into
    bob's /projects is the documented default -- so an agent asked to lock down
    a folder it owns got a denial naming somebody else's file and changed
    nothing. Unix skips those and carries on.
    """
    alice, bob = tree
    await alice.mkdir("/projects/mine", parents=True)
    await alice.write_text("/projects/mine/a.md", "a")
    await bob.write_text("/projects/mine/from-bob.md", "b")

    # The folder and alice's own file: two of the three rows underneath.
    assert await alice.chmod("/projects/mine", 0o700, recursive=True) == 2
    assert (await alice.stat("/projects/mine")).mode == 0o700
    assert (await alice.stat("/projects/mine/a.md")).mode == 0o700
    assert (await fs.stat("/projects/mine/from-bob.md")).mode == 0o666

    # Quiet skipping is safe because the folder is what hides the contents:
    # bob's file keeps its 0666 bits and is unreachable to everyone anyway --
    # bob included, since he cannot traverse alice's folder either.
    for user in (as_user("carol"), bob):
        with pytest.raises(PermissionDenied):
            await user.read_text("/projects/mine/from-bob.md")
    assert await fs.read_text("/projects/mine/from-bob.md") == "b"

    # And the path actually named is still an error when it is not yours: that
    # one is the request, not a skip.
    with pytest.raises(PermissionDenied):
        await alice.chmod("/projects/mine/from-bob.md", 0o600)


async def test_chmod_rejects_out_of_range(tree):
    alice, _ = tree
    with pytest.raises(ValueError):
        await alice.chmod("/projects/public.md", 0o1777)


# ------------------------------------------------------------ move, and root


async def test_moving_a_subtree_into_a_private_folder_hides_all_of_it(tree):
    """One UPDATE re-gates every descendant -- proves `gate` really recurses."""
    alice, bob = tree
    await alice.mkdir("/projects/work/deep", parents=True)
    await alice.write_text("/projects/work/deep/notes.md", "visible for now")
    assert await bob.read_text("/projects/work/deep/notes.md") == "visible for now"

    await alice.mv("/projects/work", "/home/alice/work")
    with pytest.raises(PermissionDenied):
        await bob.read_text("/home/alice/work/deep/notes.md")
    assert [h.path for h in await bob.search_text("visible")] == []
    assert await alice.read_text("/home/alice/work/deep/notes.md") == "visible for now"


async def test_root_sees_and_writes_everything(tree, fs):
    assert await fs.read_text("/home/alice/notes/secret.md") == SECRET
    # /home, /home/alice, /home/alice/notes, .../secret.md, /projects, public.md
    assert len(await fs.ls("/", recursive=True)) == 6
    await fs.write_text("/home/alice/notes/from-root.md", "x")
    await fs.chmod("/home/alice", 0o755)


async def test_a_user_is_required():
    from surrealfs import InvalidPath, SurrealFs

    with pytest.raises(TypeError):
        SurrealFs(object())  # type: ignore[call-arg]
    with pytest.raises(InvalidPath):
        SurrealFs(object(), user="  ")


async def test_python_and_surrealql_agree_on_every_mode(db, as_user):
    """The two implementations of the mode check must never drift apart.

    `SurrealFs._bits` uses `>>`/`&`; `fn::sfs_bits` uses `math::floor`/`%`
    because SurrealQL has no bitwise operators. They are the same rule written
    twice -- Python decides single-path operations, SurrealQL filters the bulk
    queries -- so a disagreement is a hole that only shows up in `search`.
    """
    alice = as_user("alice")
    cases = [
        (owner, mode)
        for owner in ("alice", "bob")
        for mode in range(0o000, 0o777 + 1, 7)  # a spread across all 9 bits
    ]
    rows = (
        await db.query_raw(
            "RETURN $cases.map(|$c| fn::sfs_can_read(NONE, $c[0], $c[1], 'alice'));",
            {"cases": [[owner, mode] for owner, mode in cases]},
        )
    )["result"][0]["result"]

    for (owner, mode), from_db in zip(cases, rows, strict=True):
        entry = FileEntry(
            id=None,
            path="/x",
            filename="x",
            content_type="text/plain",
            is_folder=False,
            owner=owner,
            mode=mode,
            gate=None,
        )
        assert alice._allowed(entry, 4) is bool(from_db), (owner, oct(mode))


def test_format_mode():
    assert format_mode(0o700, is_folder=True) == "drwx------"
    assert format_mode(0o666) == "-rw-rw-rw-"
    assert format_mode(0o644) == "-rw-r--r--"
