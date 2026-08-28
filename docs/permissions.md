# Permissions: the decisions

SurrealFS files carry a unix `owner` and `mode`. `/home/<user>` is private,
everything else is shared, and `chmod` moves things between the two. This
records why it is built the way it is.

## Enforcement is in `SurrealFs`, not in a table PERMISSIONS clause

The `file` table used to carry `PERMISSIONS ... WHERE owner = $auth.id`, and it
never did anything. Every surface — the browser, the Hermes plugin, the
indexer, the examples — signs in as a root, namespace or database *system*
user, and those bypass table permissions entirely. `$auth.id` was NONE, so
`owner` defaulted to NONE on every row ever written.

The obvious fix is record authentication, and it was deliberately rejected once
before (see the note this replaced in `integrations/_connect.py`): a person
signed in as a `user` record would see only files that record owned, and
everything the agents wrote would be invisible. That objection is dead now —
shared-by-default is exactly what mode bits express — but the cost is not. It
would mean provisioning a `user` row and a signin for every agent and every
person, in every deployment, before anything worked at all.

So the checks live in `SurrealFs`. That is a real boundary rather than a
courtesy one, for a specific reason: **no tool exposes raw SurrealQL**. The
fourteen tools, the browser and every integration all funnel through this one
class, so an agent has no path into the tree that skips it.

What this does *not* defend against is someone querying the database directly
with the same credentials. That is the same trust boundary the browser already
documents — anyone who can reach the port has whatever access the credentials
do — and it is a deployment question, not a code one. The `user` table, the
`account` record access and the `--include-user` flag were all deleted rather
than left lying around: nothing used them, and a permission system with two
half-wired enforcement points is worse than one with a clear boundary.

## Identity is required, and comes from the process

`SurrealFs(db, user=...)` has no default. The database cannot tell us who is
asking — every credential is a system credential — so the identity has to come
from the process that constructed the object, and guessing is bad in both
directions: guess the machine user and an agent silently owns its human's home;
guess root and every check is off.

`ROOT` bypasses everything, exactly as uid 0 does. The indexer runs as root
because it has to read every file in order to embed it, and the browser
defaults to root because it is an admin view over the whole tree.

Agent surfaces call `agent_user()`, lifted out of the Hermes memory provider so
that the tool plugin, the CLI and the provider cannot disagree about who they
are. It reads `SURREALFS_AGENT_USER`, falling back to the machine account,
and its reasoning is unchanged from when it was only a path convention: the
home belongs to the *agent*, not the human, because two agents owned by two
different people collide otherwise. What has changed is the stakes — that
string is now the only home the process can read or write.

## `gate`: how ancestry is checked without a loop

Unix needs the `x` bit on *every* directory above a file. A permission check
cannot loop, and walking the chain per row would cost a query per level.

`gate` is a recursive COMPUTED field holding the owner of the nearest ancestor
that is not world-traversable, or NONE when the whole chain is. It is the same
trick `path` already uses, and it buys the same thing: writes stay a single
row, and `mv` re-gates an entire subtree for free. A 0666 file inside a 0700
home is unreachable because its `gate` says so, not because anything walked up
to find out.

It cannot be indexed, which is why `ls`, `glob` and both search arms filter with
`fn::sfs_can_read` as a post-filter on an indexed scan. That was measured, not
assumed: `EXPLAIN` shows `idx_file_content` still driving a `FullTextScan` with
the predicate applied above it. The risk it was checked against is real —
COMPUTED fields read NULL through an index on some engines, and a NULL `gate`
would mean `search` silently stopped filtering and started returning other
people's file contents. `tests/test_permissions.py` asserts the FULLTEXT and
HNSW paths specifically for that reason.

## Everything outside `/home` is 0777

The unix default is 022, giving 0755 and 0644 — shared means readable, and only
the owner writes. We use 000 instead: 0777 folders, 0666 files.

The tree was a free-for-all before permissions existed, and `/projects` is where
agents hand work to each other and to people. Defaulting to 0755 would have
quietly broken that on the day this shipped, and the failure would look like an
agent that cannot write to a folder it can see. Making privacy opt-in and
sharing the default matches what the tree is *for*; `chmod` is how you tighten
something you actually care about.

A home is the one exception, created 0700 by `default_mode`, which is what makes
`mkdir -p /home/alice/memories` produce a private home with no provisioning step
anywhere.

## A home belongs to the user it is named after

`default_owner` overrides the creator for a folder directly under `/home`:
`/home/alice` is alice's whoever runs the `mkdir`.

Without it, whoever touched the path first owned it — and root touches it often
(the indexer, the browser, a seeding script). Since a home is 0700, root
creating `/home/alice` would have locked alice out of her own home permanently,
because there is no `chown` to undo it with. Making ownership a property of the
path rather than of timing removes the whole class of problem, and it is what
`useradd -m` does anyway.

`_guard_home` still refuses to let bob create `/home/alice`, so the mistake
fails loudly rather than silently doing something harmless.

## Deletion is keyed on the parent folder

`rm` checks write and execute on the *containing folder*, not on the file. This
surprises people, and it is what unix does: a read-only file in a folder you can
write is yours to remove. Agents rely on it more than they realise — it is why
`rm` on a 0444 file in a shared folder works.

## Known ceilings

Both are marked `ponytail:` where they live.

**Group bits are stored but never consulted.** `chmod 750` round-trips and
displays correctly, and the middle digit does nothing. Nothing in SurrealFS has
a group concept to consult, and inventing one — a group table, membership,
provisioning — is a lot of machinery for a need nobody has stated. Add it when
a team wants a folder outsiders cannot see.

**`gate` tracks only the other-`x` bit.** A directory whose *owner*-`x` bit is
clear (0600) does not block its own owner, which real unix would. Writing that
mode on a directory is something nobody does on purpose.
