-- SurrealFS: the `file` table.
--
-- A file tree is modelled with parent record links. `path` is derived, never
-- stored, so renaming or moving a folder updates every descendant's path for
-- free -- a rename is a single UPDATE of `filename`.
--
-- REQUIRES a SurrealDB 3.x *server*. COMPUTED fields and FULLTEXT indexes do
-- not exist in 2.x at all, and the engine in `surrealdb[embedded]` 3.0.0a4
-- (`mem://`, `file://`) parses them but returns NULL for computed fields on any
-- indexed read -- which is most of them. See tests/conftest.py.

-- Permission predicates
--
-- The rule lives here once. `SurrealFs` mirrors the bit arithmetic in Python
-- for single-path operations (better errors, and it works under a system
-- credential where table permissions are bypassed); these are what the four
-- bulk queries in `fs.py` and the table's own PERMISSIONS clause both call.
-- `tests/test_permissions.py` asserts the two implementations agree.

-- Who is asking: the record user's name, or NONE under a system credential.
-- The username *is* the record id (`user:alice`), so this reads the id without
-- fetching the row -- no `user` table select permission needed, and usernames
-- are unique by construction. See `schema/record_auth.surql`.
DEFINE FUNCTION OVERWRITE fn::sfs_me() {
    RETURN IF $auth IS NONE THEN NONE ELSE record::id($auth) END;
};

-- The gate on `$parent`'s children: who -- if anyone -- may traverse down to
-- them. NONE when every directory above is world-traversable, the owner when
-- every closed one above is that same owner, and `'!closed'` when two closed
-- directories with different owners are in the chain.
--
-- Unix needs the `x` bit on *every* ancestor, and a permission check cannot
-- loop, so the whole chain has to collapse into this one column. It recurses
-- through `$parent.gate` -- that parent's own COMPUTED field -- but only
-- *after* consulting `$parent`'s own mode: short-circuiting on the inherited
-- value instead would report the outermost closed ancestor and let a nested
-- private folder be read by whoever owns something above it.
--
-- `'!closed'` matches nobody: `_slug` in `integrations/_connect.py` and
-- `SurrealFs.__init__` both restrict a username to [A-Za-z0-9_-], so
-- `fn::sfs_reachable` denies every caller -- which is right, since no single
-- user can traverse two folders closed by two different owners.
--
-- `% 2` is the other-execute bit; SurrealQL has no bitwise operators, so the
-- digit arithmetic here and in `fn::sfs_bits` uses `math::floor` and `%`.
-- ponytail: only the *other* x bit gates. A directory with its owner-x bit
-- clear (0600) does not block its own owner, which real unix would. Track the
-- owner bit too if anyone ever writes such a mode on purpose.
DEFINE FUNCTION OVERWRITE fn::sfs_gate($parent: option<record<file>>) {
    -- `$parent`'s own contribution: its owner if it is closed, else NONE.
    --
    -- A row written before `mode` existed belongs to nobody but root until
    -- `docs/migrate_permissions.surql` has run. Treating a missing mode as
    -- traversable would expose an un-migrated tree; erroring on it -- which
    -- is what `NONE % 2` does -- takes the whole database down, including
    -- the migration itself, because re-writing a row recomputes this.
    LET $own = IF $parent IS NONE THEN NONE
        ELSE IF $parent.mode IS NONE OR $parent.mode % 2 = 0
        THEN ($parent.owner ?? 'root')
        ELSE NONE END;
    RETURN IF $own IS NONE THEN $parent.gate
        ELSE IF $parent.gate IS NONE OR $parent.gate = $own THEN $own
        ELSE '!closed' END;
};

-- Whether `$me` can traverse to a row gated by `$gate`. An unauthenticated
-- caller is nobody and gets nothing: without this guard `$gate IS NONE` would
-- read as "unguarded" and hand the shared tree to an anonymous connection.
DEFINE FUNCTION OVERWRITE fn::sfs_reachable($gate: option<string>, $me: option<string>) {
    RETURN $me IS NOT NONE AND ($gate IS NONE OR $gate = $me);
};

-- The octal digit that applies to `$me`: the owner digit for the owner,
-- otherwise the other digit. Within a digit, `r` is `d >= 4`, `w` is
-- `math::floor(d / 2) % 2`, and `x` is `d % 2`.
--
-- ponytail: the group digit (`math::floor($mode / 8) % 8`) is stored and
-- round-trips through `chmod`, but is never consulted -- nothing in SurrealFS
-- has a group concept to consult it with. Add a group table if teams ever need
-- a folder outsiders cannot see.
-- `$owner` and `$mode` are optional for one reason only: a row written before
-- those fields existed has neither, and it must fail *closed* -- no owner reads
-- as root's, no mode as no permissions -- until
-- `docs/migrate_permissions.surql` has run. `SurrealFs.from_row` coalesces the same
-- way, so the two implementations still agree.
DEFINE FUNCTION OVERWRITE fn::sfs_bits(
    $owner: option<string>, $mode: option<int>, $me: option<string>
) {
    RETURN IF ($owner ?? 'root') = $me
        THEN math::floor(($mode ?? 0) / 64) % 8
        ELSE ($mode ?? 0) % 8
    END;
};

-- Readable by `$me`: reachable through every ancestor, and the read bit set.
DEFINE FUNCTION OVERWRITE fn::sfs_can_read(
    $gate: option<string>, $owner: option<string>, $mode: option<int>,
    $me: option<string>
) {
    RETURN fn::sfs_reachable($gate, $me) AND fn::sfs_bits($owner, $mode, $me) >= 4;
};

-- The write bit of the digit that applies to `$me`. Within a digit `w` is
-- `math::floor(d / 2) % 2`; see `fn::sfs_bits`.
DEFINE FUNCTION OVERWRITE fn::sfs_w(
    $owner: option<string>, $mode: option<int>, $me: option<string>
) {
    RETURN math::floor(fn::sfs_bits($owner, $mode, $me) / 2) % 2 = 1;
};

-- Writable by `$me`: reachable through every ancestor, and the write bit set.
DEFINE FUNCTION OVERWRITE fn::sfs_can_write(
    $gate: option<string>, $owner: option<string>, $mode: option<int>,
    $me: option<string>
) {
    RETURN fn::sfs_reachable($gate, $me) AND fn::sfs_w($owner, $mode, $me);
};

-- Whether `$me` may add or remove an *entry* in `$parent`: write and execute on
-- the folder, which is what unix keys create, delete and rename on -- never the
-- entry's own bits. This is the rule `SurrealFs._require_writable_dir` applies.
--
-- NONE is the root directory. It is not a row (see `_resolve`) and behaves as
-- 0777 owned by root, so anyone signed in may make a top-level entry -- which is
-- the shared tree the whole design rests on. `fn::sfs_home_ok` is what stops
-- that from including `/home`.
DEFINE FUNCTION OVERWRITE fn::sfs_can_enter(
    $parent: option<record<file>>, $me: option<string>
) {
    RETURN IF $me IS NONE THEN false
        ELSE IF $parent IS NONE THEN true
        ELSE fn::sfs_reachable($parent.gate, $me)
             -- w and x are the low two bits of the digit, so both set is `% 4 = 3`.
             AND fn::sfs_bits($parent.owner, $parent.mode, $me) % 4 = 3
    END;
};

-- `SurrealFs._guard_home`, written for the database: `/home` belongs to root,
-- and a home directly under it belongs to the name it carries.
--
-- Without this one CREATE squats an unprovisioned `/home/alice` -- and since
-- `/home` is 0777 so that a user can make their own home, one DELETE takes
-- `/home` itself, orphaning every home to the top level with a dangling parent.
DEFINE FUNCTION OVERWRITE fn::sfs_home_ok(
    $parent: option<record<file>>, $filename: string, $me: option<string>
) {
    RETURN IF $parent IS NONE THEN $filename != 'home'
        ELSE IF $parent.parent IS NONE AND $parent.filename = 'home'
        THEN $filename = $me
        ELSE true
    END;
};

-- Ownership and unix mode bits are enforced in two places, and the split is
-- deliberate. `SurrealFs` applies the exact unix rules with the exact errors,
-- and is the only way an agent gets in -- no tool exposes raw SurrealQL. The
-- clauses below are the same rules for anyone holding a record credential and a
-- client of their own; they are inert unless you opt into record auth, because
-- system users bypass table and field permissions entirely.
--
-- They enforce reads *and* writes. An earlier version checked only ancestry on
-- update and delete, which let any record user rewrite, chmod, chown or delete
-- any row it could reach -- including `/home`. See `docs/permissions.md`.
--
-- The clause says `fn::sfs_gate(parent)` and must never be "tidied" to `gate`.
-- A table permission is evaluated against the raw record *before* computed
-- fields run, so `gate` reads NONE there -- and NONE means "no closed
-- ancestor", i.e. readable. That fails open: verified on 3.2.4, the naive
-- version hands every private file to every record user, silently.
-- `fn::sfs_gate(parent)` dereferences a *linked* record, which does compute.
DEFINE TABLE OVERWRITE file SCHEMAFULL
    PERMISSIONS
        FOR select
            WHERE fn::sfs_can_read(fn::sfs_gate(parent), owner, mode, fn::sfs_me())
        FOR create
            WHERE owner = fn::sfs_me()
              AND fn::sfs_can_enter(parent, fn::sfs_me())
              AND fn::sfs_home_ok(parent, filename, fn::sfs_me())
        -- An update clause is evaluated against the before state *and* the after
        -- state -- verified on 3.2.4 -- so `parent` here means the source folder
        -- on one pass and the destination on the other, which is exactly what a
        -- rename needs. The disjunction is deliberate: unix keys a content write
        -- on the file's own `w` and a rename on the folder's `w`+`x`, and a
        -- permission clause cannot tell which field a statement touched
        -- (`$before` is not bound in one). The per-field clauses below are what
        -- make it precise; this is the coarse union of the two rules.
        FOR update
            WHERE fn::sfs_reachable(fn::sfs_gate(parent), fn::sfs_me())
              AND fn::sfs_home_ok(parent, filename, fn::sfs_me())
              AND (fn::sfs_w(owner, mode, fn::sfs_me())
                   OR fn::sfs_can_enter(parent, fn::sfs_me()))
        -- ponytail: the union is loose in one direction. A record user holding
        -- the `w` bit on a row satisfies it on both passes, so they can also
        -- *move* that row without `w`+`x` on either folder, which `SurrealFs.mv`
        -- requires. It buys them nothing they did not already have -- the `w` bit
        -- lets them overwrite the content outright -- and tightening it means
        -- requiring `w`+`x` on the parent for every content write, which is
        -- stricter than unix and than the Python layer. Revisit if a rename ever
        -- becomes more consequential than a rewrite.
        -- Keyed on the containing folder, never the row: a read-only file in a
        -- folder you can write is yours to remove, same as `SurrealFs.rm`.
        FOR delete
            WHERE fn::sfs_can_enter(parent, fn::sfs_me())
              AND fn::sfs_home_ok(parent, filename, fn::sfs_me());

-- There is no `chown`, so `owner` is immutable once written -- and it has to be,
-- because a field permission is evaluated only against the *after* state
-- (verified on 3.2.4): `FOR update WHERE owner = fn::sfs_me()` reads as true the
-- moment somebody writes their own name into it, which let any record user take
-- a file they could reach. `FOR update WHERE false` has no such hole, and leaves
-- `create` alone -- the table clause above already pins `owner` to the creator.
--
-- With `owner` pinned, the self-reference in `mode`'s clause is sound: the
-- after-state owner is always the before-state owner, so this really is "only
-- the owner may chmod", the rule `SurrealFs.chmod` applies.
DEFINE FIELD OVERWRITE owner ON TABLE file TYPE string DEFAULT 'root'
    PERMISSIONS FOR update WHERE false;
DEFINE FIELD OVERWRITE mode  ON TABLE file TYPE int    DEFAULT 438  -- 0o666
    PERMISSIONS FOR update WHERE owner = fn::sfs_me();
DEFINE FIELD OVERWRITE filename ON TABLE file TYPE string;
DEFINE FIELD OVERWRITE parent ON file TYPE option<record<file>> DEFAULT NONE;
-- Stored mirror of `parent`, with the root spelled 'root' instead of NONE.
-- This exists because a composite UNIQUE index skips any row where one of its
-- fields is NONE: indexing (parent, filename) directly would leave every
-- root-level file unprotected, allowing two `/notes.md`. It cannot be a
-- COMPUTED field -- SurrealDB refuses to index those -- so it is a stored VALUE,
-- re-evaluated on every write and therefore correct across moves.
DEFINE FIELD OVERWRITE parent_key ON file TYPE string
    VALUE IF $this.parent IS NONE THEN 'root' ELSE <string>$this.parent END;

-- <string> is to avoid error when $this.filename is NONE. This happens when deleting a file.
DEFINE FIELD OVERWRITE path ON file COMPUTED (
    ($this.parent.path || '') + '/' + <string>$this.filename
) TYPE string;
DEFINE FIELD OVERWRITE is_folder ON TABLE file COMPUTED (
    $this.file IS NONE AND $this.content IS NONE AND $this.symlink IS NONE
);

-- Who may traverse down to this row: NONE when every directory above it is
-- world-traversable, otherwise the owner of the closed ones (see
-- `fn::sfs_gate`). This is what makes a 0666 file inside a 0700 home
-- unreachable.
--
-- Recursive, exactly like `path` above, and for the same payoff: a write stays
-- a single row and `mv` re-derives a whole subtree for free. Verified on 3.2.4
-- to be populated on FULLTEXT- and HNSW-indexed reads -- if it ever came back
-- NULL there, `search` would silently stop filtering, so `tests/
-- test_permissions.py` asserts it through both indexes.
--
-- The rule itself lives in `fn::sfs_gate` so that this field and the table's
-- PERMISSIONS clause share one definition rather than two that must agree.
DEFINE FIELD OVERWRITE gate ON TABLE file TYPE option<string>
    COMPUTED fn::sfs_gate($this.parent);

-- File content (folders are defined by leaving all of these empty)
--
-- The write bit, per field, because the table clause above cannot be this
-- precise: it has to stay permissive enough for a rename, which unix keys on
-- the folder instead. These are what a content write actually touches.
--
-- ponytail: `hash` and the embedding fields below are left open. They are
-- derived -- `evt_file_hash` recomputes `hash` on any content change, and a
-- wrong `embedded_hash` only makes the indexer re-embed -- and none of them
-- gates access. Guard them too if anything ever trusts them for more than
-- staleness.
DEFINE FIELD OVERWRITE file       ON TABLE file TYPE option<bytes> DEFAULT NONE
    PERMISSIONS FOR update WHERE fn::sfs_w(owner, mode, fn::sfs_me());
DEFINE FIELD OVERWRITE content    ON TABLE file TYPE option<string> DEFAULT NONE
    PERMISSIONS FOR update WHERE fn::sfs_w(owner, mode, fn::sfs_me());
DEFINE FIELD OVERWRITE symlink    ON TABLE file TYPE option<file> DEFAULT NONE
    PERMISSIONS FOR update WHERE fn::sfs_w(owner, mode, fn::sfs_me());
DEFINE FIELD OVERWRITE content_type ON TABLE file TYPE string
    PERMISSIONS FOR update WHERE fn::sfs_w(owner, mode, fn::sfs_me());
DEFINE FIELD OVERWRITE hash       ON file TYPE string DEFAULT '';

-- Timestamps
DEFINE FIELD OVERWRITE created_at ON TABLE file TYPE datetime VALUE time::now() READONLY;
DEFINE FIELD OVERWRITE updated_at ON TABLE file TYPE datetime VALUE time::now();
DEFINE FIELD OVERWRITE deleted_at ON TABLE file TYPE option<datetime> DEFAULT NONE;

-- Events

-- Update hash when content changes. Guard on $event: a DELETE also satisfies
-- `$before.content != $after.content` ($after is NONE), and `UPDATE NONE` then
-- errors out and aborts the DELETE.
DEFINE EVENT OVERWRITE evt_file_hash ON file
    WHEN $event != "DELETE" AND $before.content != $after.content
    THEN (
        UPDATE $after.id SET hash = crypto::md5($after.content)
);


-- Vector embedding
DEFINE FIELD OVERWRITE embedding       ON TABLE file TYPE option<array<float>> DEFAULT NONE;
-- Stamp set by the indexer when `embedding` is written. Informational only:
-- it cannot detect staleness, because `updated_at` is bumped by *any* write,
-- including the indexer's own.
DEFINE FIELD OVERWRITE embedded_at     ON TABLE file TYPE option<datetime>     DEFAULT NONE;
-- md5 of the content the current `embedding` was produced from. This is what
-- makes staleness detectable: content changed iff it differs from the content's
-- hash now. Written by the indexer, never by the content-write path.
DEFINE FIELD OVERWRITE embedded_hash   ON TABLE file TYPE option<string>       DEFAULT NONE;
-- Identifier of the embedder that produced the current vector (e.g.
-- "openai:text-embedding-3-small"). Lets a later model swap invalidate stale rows.
DEFINE FIELD OVERWRITE indexer_version ON TABLE file TYPE option<string>       DEFAULT NONE;

-- Indexes

-- Field order matters: (parent_key, filename) serves BOTH the uniqueness
-- constraint and prefix lookups by parent alone -- the hottest query in the
-- library (ls, path resolution, recursive rm/cp). The reverse order forces a
-- table scan for `WHERE parent_key = $k`, verified with EXPLAIN.
DEFINE INDEX OVERWRITE child_unique ON TABLE file FIELDS parent_key, filename UNIQUE;

-- Vector search
DEFINE INDEX IF NOT EXISTS idx_hnsw_file_embedding ON file
    FIELDS embedding
    HNSW DIMENSION 1536
    DIST COSINE
    TYPE F32
    EFC 150 M 12 M0 24;

-- Full-text search.
-- NOTE: on SurrealDB 3.2.x the `@@` match operator is index-backed and correct,
-- but search::score/highlight/offsets return 0.0/passthrough/null. SurrealFs
-- therefore ranks and snippets in Python. BM25 params are kept for when the
-- server-side scoring lands.
DEFINE ANALYZER IF NOT EXISTS fts_simple
  TOKENIZERS blank,class,camel,punct
  FILTERS snowball(english);

DEFINE INDEX IF NOT EXISTS idx_file_content ON file
    FIELDS content
    FULLTEXT ANALYZER fts_simple
    BM25(1.2,0.75);

-- Seed: /home
--
-- `/home` has to exist and belong to root *before* anyone creates a home
-- inside it. `SurrealFs._guard_home` only guards the level below -- and the
-- root directory is not a row, so it has no owner to inherit -- which left
-- `/home` up for grabs: the first user to `mkdir -p /home/<name>` owned it at
-- 0777 and could then `chmod 700 /home`, locking every other user out of
-- their own home and (through `gate`) reading into it.
--
-- Two statements, both idempotent, so this is safe on an existing database:
-- the UPDATE reclaims a `/home` somebody already squatted (whatever its record
-- id), and the INSERT creates it when there is none -- ignored when the UPDATE
-- already found one, because `child_unique` rejects the duplicate.
UPDATE file SET owner = 'root', mode = 511  -- 0o777
    WHERE parent_key = 'root' AND filename = 'home';
INSERT IGNORE INTO file {
    id: file:home,
    filename: 'home',
    parent: NONE,
    owner: 'root',
    mode: 511,  -- 0o777
    content_type: 'inode/directory'
};
