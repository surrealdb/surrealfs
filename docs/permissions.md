# Permissions: the decisions

SurrealFS files carry a unix `owner` and `mode`. `/home/<user>` is private,
everything else is shared, and `chmod` moves things between the two. This
records why it is built the way it is.

## Enforcement is in `SurrealFs`, with the database as an optional backstop

The `file` table used to carry `PERMISSIONS ... WHERE owner = $auth.id`, and it
never did anything. Every surface — the browser, the Hermes plugin, the
indexer, the examples — signs in as a root, namespace or database *system*
user, and those bypass table permissions entirely. `$auth.id` was NONE, so
`owner` defaulted to NONE on every row ever written.

Record authentication would make the clause live, and it was rejected as the
*default* — not as an option. Making it mandatory would mean provisioning a
`user` row and a signin for every agent and every person, in every deployment,
before anything worked at all. Most people running SurrealFS have one agent and
one database and need none of that.

So the checks live in `SurrealFs`. That is a real boundary rather than a
courtesy one, for a specific reason: **no tool exposes raw SurrealQL**. The
fifteen tools, the browser and every integration all funnel through this one
class, so an agent has no path into the tree that skips it.

What that alone does *not* defend against is someone querying the database
directly with the credential. For deployments that care — several agents, one
database — record auth closes it, opt-in, and the section at the end of this
document describes it.

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
are. It reads `SURREALFS_AGENT_USER`, falling back to the machine account —
except under record auth, where the credential *is* the identity and the name
comes from `SURREALDB_USER` alone, so the two can never disagree about which
user's permissions apply to whose rows. Its reasoning is otherwise unchanged from when it was only a path convention: the
home belongs to the *agent*, not the human, because two agents owned by two
different people collide otherwise. What has changed is the stakes — that
string is now the only home the process can read or write.

## `gate`: how ancestry is checked without a loop

Unix needs the `x` bit on *every* directory above a file. A permission check
cannot loop, and walking the chain per row would cost a query per level.

`gate` is a recursive COMPUTED field collapsing that chain into one column:
NONE when every ancestor is world-traversable, the owner when every closed one
belongs to that same owner, and `'!closed'` — a value no username can equal —
when two owners have each closed a directory in the chain, since nobody may
traverse both. It consults each parent's own mode *before* the value it
inherits; the other order reports the outermost closed ancestor and leaves a
nested private folder readable by whoever owns something above it. It is the same
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

## Record auth, optionally

`apply_schema(db, record_auth=True)` plus `python -m surrealfs.users add alice`
plus `SURREALDB_AUTH_LEVEL=record` makes SurrealDB enforce the tree itself, so
the credential alone is no longer a way around the mode bits.

Applying the schema is an admin's job here, once, and `connected()` will not do
it for you: a record user has no DDL rights, so a `connected()` that tried
would fail *every* call rather than the first. Run
`python -m surrealfs.schema --record-auth` with a system credential first.

**There is almost nothing extra to maintain, by construction.** The rule already
lived once, in `fn::sfs_can_read`. The table's PERMISSIONS clause calls the same
function the four bulk queries in `fs.py` call, substituting the signed-in user
for `$me`. Two small extractions made that possible, and both *reduced* the
total expression: the ancestry rule moved into `fn::sfs_gate($parent)`, shared
by the `gate` COMPUTED field and the clause; and "who is asking" moved into
`fn::sfs_me()`, used by the clause.

The clause is defined **unconditionally**. A table permission is inert for
system users, so it costs non-adopters nothing and there is no conditional DDL
to keep in step. `record_auth.surql` adds only the identity half.

### The clause must say `fn::sfs_gate(parent)`, never `gate`

This is the one line to be careful with, and the failure is silent.

SurrealDB evaluates a table permission against the **raw record, before
computed fields run**. `gate` is a COMPUTED field on `file`, so inside `file`'s
own clause it reads NONE — and NONE means "no closed ancestor", which means
readable. Verified on 3.2.4: the naive version hands every private file to
every record user, with no error. It **fails open**.

Traversing to a *linked* record is the opposite: a permission predicate reads
links with permissions off (`skip_fetch_perms`), and that path explicitly does
compute the fetched record's computed fields. So `fn::sfs_gate(parent)` resolves
correctly and recurses the whole way up. The same flag is what stops the
recursion looping — a nested permission check encountered while already inside
one returns true rather than re-entering.

`tests/test_record_auth.py` covers a four-deep chain specifically so that
"tidying" the clause back to `gate` fails a test instead of opening the tree.

### The two layers do different jobs

| | Python (`SurrealFs`) | The PERMISSIONS clause |
|---|---|---|
| `select` | exact unix bits | exact — the same `fn::sfs_can_read` |
| create / update / delete | exact unix bits | reachability only |

Writes are deliberately looser in the database, and the reason is that unix
keys `rm` and `mv` on the **containing directory**, not on the file. A
row-level clause cannot see the directory without traversing to it, and an
exact one would start *rejecting operations Python allows* — removing a 0444
file from a folder you can write, for instance. A conflict between the layers
is a confusing error, not a safe failure. Reachability-only can never conflict,
because Python is strictly stricter, and it still guarantees the thing that
matters: **nothing inside another user's private subtree can be touched.**

What it does not cover is the mode bits on files in *shared* folders against an
attacker writing raw SurrealQL. Python covers those; the database does not.

### Two behaviours that differ under record auth

Both are stricter, never looser, and both are asserted in the tests.

*Listing hides names.* `ls /home` shows every user's home under a system
credential — unix lists a directory you can read, whatever the entries' own
modes say. Under record auth the database filters per row, so other homes
vanish entirely. Matching it would mean granting select on any child of a
readable folder, which would expose the *content* of a 0600 file in a shared
folder. Hiding the name is the better trade.

*Errors are vaguer.* `_resolve` walks a path segment by segment. When the
database hides an intermediate folder the walk dead-ends, so you get `NotFound`
where a system credential would resolve the row and raise `PermissionDenied`.
Less informative, and it leaks less.

### No SIGNUP

Self-service registration would let anyone claim any unused username, and
`/home/<name>` belongs to the name it carries — so registering `alice` would
hand over alice's home. That is exactly the squatting hole `_guard_home` closes
on the Python side. Provisioning is an admin operation against a system
credential, which is also why the `user` table's own permissions are
`FOR create, update, delete NONE`.

The username **is** the record id (`user:alice`). That keeps `fn::sfs_me()` a
pure read of `$auth` with no row fetch — `$auth` is bound to a record *id*, not
an object, so dereferencing a field on it outside a permission clause would
re-enter the `user` table's permissions — and it means a username cannot drift
from the home it names.

### One thing we get for free

Our BM25 runs in Python over rows the database has already returned, so unlike
`search::score` it cannot leak corpus statistics about documents the caller
cannot see.

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
