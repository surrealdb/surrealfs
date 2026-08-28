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

DEFINE TABLE OVERWRITE file SCHEMAFULL;

-- Ownership and unix mode bits. Enforced by `SurrealFs`, not by a table
-- PERMISSIONS clause: every shipped surface signs in as a root/namespace/
-- database *system* user, and those bypass table permissions entirely, so a
-- clause here would be decorative. `SurrealFs` is the only way in -- no tool
-- exposes raw SurrealQL -- and it is where the checks live. See
-- `docs/permissions.md`.
DEFINE FIELD OVERWRITE owner ON TABLE file TYPE string DEFAULT 'root';
DEFINE FIELD OVERWRITE mode  ON TABLE file TYPE int    DEFAULT 438;  -- 0o666
DEFINE FIELD OVERWRITE filename ON TABLE file TYPE string;
DEFINE FIELD OVERWRITE parent ON file TYPE option<record<file>> DEFAULT NONE;
DEFINE FIELD OVERWRITE content_type ON TABLE file TYPE string;

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

-- The owner of the nearest ancestor directory that is not world-traversable,
-- or NONE when every directory above this row is. This is what makes a 0666
-- file inside a 0700 home unreachable: unix needs the `x` bit on *every*
-- ancestor, and a permission check cannot loop.
--
-- Recursive, exactly like `path` above, and for the same payoff: a write stays
-- a single row and `mv` re-derives a whole subtree for free. Verified on 3.2.4
-- to be populated on FULLTEXT- and HNSW-indexed reads -- if it ever came back
-- NULL there, `search` would silently stop filtering, so `tests/
-- test_permissions.py` asserts it through both indexes.
--
-- `% 2` is the other-execute bit; SurrealQL has no bitwise operators, so the
-- digit arithmetic in fn::sfs_bits below uses `math::floor` and `%` throughout.
-- ponytail: only the *other* x bit gates. A directory with its owner-x bit
-- clear (0600) does not block its own owner, which real unix would. Track the
-- owner bit too if anyone ever writes such a mode on purpose.
DEFINE FIELD OVERWRITE gate ON TABLE file TYPE option<string> COMPUTED (
    IF $this.parent IS NONE THEN NONE
    ELSE IF $this.parent.gate IS NOT NONE THEN $this.parent.gate
    ELSE IF $this.parent.mode % 2 = 0 THEN $this.parent.owner
    ELSE NONE END
);

-- File content (folders are defined by leaving all of these empty)
DEFINE FIELD OVERWRITE file       ON TABLE file TYPE option<bytes> DEFAULT NONE;
DEFINE FIELD OVERWRITE content    ON TABLE file TYPE option<string> DEFAULT NONE;
DEFINE FIELD OVERWRITE symlink    ON TABLE file TYPE option<file> DEFAULT NONE;
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

-- Permission predicates

-- The octal digit that applies to `$me`: the owner digit for the owner,
-- otherwise the other digit. Within a digit, `r` is `d >= 4`, `w` is
-- `math::floor(d / 2) % 2`, and `x` is `d % 2`.
--
-- ponytail: the group digit (`math::floor($mode / 8) % 8`) is stored and
-- round-trips through `chmod`, but is never consulted -- nothing in SurrealFS
-- has a group concept to consult it with. Add a group table if teams ever need
-- a folder outsiders cannot see.
DEFINE FUNCTION OVERWRITE fn::sfs_bits($owner: string, $mode: int, $me: string) {
    RETURN IF $owner = $me
        THEN math::floor($mode / 64) % 8
        ELSE $mode % 8
    END;
};

-- Readable by `$me`: reachable through every ancestor, and the read bit set.
-- Defined once here and referenced by the four bulk queries in `fs.py` that
-- cannot filter row by row -- ls, glob, search_text, search_semantic.
DEFINE FUNCTION OVERWRITE fn::sfs_can_read(
    $gate: option<string>, $owner: string, $mode: int, $me: string
) {
    RETURN ($gate IS NONE OR $gate = $me)
        AND fn::sfs_bits($owner, $mode, $me) >= 4;
};
