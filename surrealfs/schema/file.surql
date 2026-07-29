-- SurrealFS: the `file` table.
--
-- A file tree is modelled with parent record links. `path` is derived, never
-- stored, so renaming or moving a folder updates every descendant's path for
-- free -- a rename is a single UPDATE of `filename`.
--
-- REQUIRES SurrealDB 3.x: COMPUTED fields and FULLTEXT indexes do not exist in
-- 2.x. In particular the embedded engine bundled with the Python SDK (`mem://`,
-- `file://`) is a 2.0.0 core and cannot run this schema -- use a real server.

DEFINE TABLE OVERWRITE file SCHEMAFULL
    PERMISSIONS
        FOR select, create, update, delete
        WHERE owner = $auth.id;

DEFINE FIELD OVERWRITE owner ON TABLE file TYPE option<record<user>> DEFAULT $auth.id;
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
