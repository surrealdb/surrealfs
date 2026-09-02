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
    LET $own = IF $parent IS NONE THEN NONE
        ELSE IF $parent.mode % 2 = 0 THEN $parent.owner
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
-- `$owner` and `$mode` are required, not optional: every row has both, by
-- `DEFAULT`, so a row reaching here without one is a schema that did not apply
-- rather than a row to guess a permission for. Coalescing them to root's would
-- turn that into a silent wrong answer.
--
-- ponytail: the group digit (`math::floor($mode / 8) % 8`) is stored and
-- round-trips through `chmod`, but is never consulted -- nothing in SurrealFS
-- has a group concept to consult it with. Add a group table if teams ever need
-- a folder outsiders cannot see.
DEFINE FUNCTION OVERWRITE fn::sfs_bits($owner: string, $mode: int, $me: option<string>) {
    RETURN IF $owner = $me
        THEN math::floor($mode / 64) % 8
        ELSE $mode % 8
    END;
};

-- Readable by `$me`: reachable through every ancestor, and the read bit set.
DEFINE FUNCTION OVERWRITE fn::sfs_can_read(
    $gate: option<string>, $owner: string, $mode: int, $me: option<string>
) {
    RETURN fn::sfs_reachable($gate, $me) AND fn::sfs_bits($owner, $mode, $me) >= 4;
};

-- The write bit of the digit that applies to `$me`. Within a digit `w` is
-- `math::floor(d / 2) % 2`; see `fn::sfs_bits`.
DEFINE FUNCTION OVERWRITE fn::sfs_w($owner: string, $mode: int, $me: option<string>) {
    RETURN math::floor(fn::sfs_bits($owner, $mode, $me) / 2) % 2 = 1;
};

-- Writable by `$me`: reachable through every ancestor, and the write bit set.
DEFINE FUNCTION OVERWRITE fn::sfs_can_write(
    $gate: option<string>, $owner: string, $mode: int, $me: option<string>
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

-- Whether `$row` still has children. A DELETE is keyed on the containing
-- folder (`fn::sfs_can_enter`), which says nothing about what is *inside* the
-- row being removed -- and for a top-level row `can_enter` is unconditionally
-- true. Without this, one `DELETE` of a 0777 folder leaves every child holding
-- a `parent` link and a stored `parent_key` pointing at a dead record: absent
-- from every listing, unreachable to `_resolve`, and undeletable, while still
-- occupying its name in `child_unique`.
--
-- `SurrealFs.rm` refuses the same thing with `DirectoryNotEmpty` and deletes a
-- tree deepest-first, which satisfies this on every row it touches.
--
-- The SELECT is *not* permission-filtered -- a statement inside a permission
-- clause runs unrestricted, verified on 3.2.4 and asserted in
-- `tests/test_record_auth.py`. That is the only reason this is sound: were it
-- filtered, a child the deleter cannot select would read as absent and the
-- guard would fail open on precisely the rows it exists for.
DEFINE FUNCTION OVERWRITE fn::sfs_has_children($row: record<file>) {
    RETURN (SELECT VALUE id FROM ONLY file
            WHERE parent_key = <string>$row LIMIT 1) IS NOT NONE;
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
-- They enforce reads *and* writes, and both halves are load-bearing. Ancestry
-- alone on update and delete is not a permission check: it lets any record user
-- rewrite, chmod, chown or delete any row it can *reach*, `/home` included. See
-- `docs/permissions.md`.
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
              AND fn::sfs_home_ok(parent, filename, fn::sfs_me())
              AND !fn::sfs_has_children(id);

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
--
-- `search::score` works, and `fn::sfs_search_text` below ranks on it -- verified
-- on 3.2.4+20260803.93ab219. It was recorded here as broken upstream for a long
-- time because it reads 0.0 far more readily than a bug would: SurrealDB scores
-- with the unsmoothed BM25 IDF, `ln((N - df + 0.5) / (df + 0.5))`, clamped at
-- zero -- so every score collapses to 0.0 for a term held by more than about
-- half the corpus. On a three-file test corpus that is *every* term. Python's
-- `_bm25` uses the smoothed `ln(1 + ...)` variant, which is always positive, so
-- the two rank the same corpus differently at the low end.
--
-- `search::highlight` (returns the content unchanged) and `search::offsets`
-- (returns NONE) really are broken on this build, so snippets stay in Python.
DEFINE ANALYZER IF NOT EXISTS fts_simple
  TOKENIZERS blank,class,camel,punct
  FILTERS snowball(english);

DEFINE INDEX IF NOT EXISTS idx_file_content ON file
    FIELDS content
    FULLTEXT ANALYZER fts_simple
    BM25(1.2,0.75);

-- Search API
--
-- The same retrieval `SurrealFs.search_text`, `search_semantic` and `search`
-- give Python, exposed to anyone holding a connection: a TS client, a raw
-- `surreal sql` session, a live query. The rule lives here once, the way the
-- permission predicates above do -- `fs.py` is not the ceiling on who gets
-- ranked, filtered retrieval, and `fs.py` itself calls these rather than
-- restating them.
--
-- Every one of them is permission-filtered. `search` returns file *content*, so
-- a missed filter here is a disclosure rather than a cosmetic bug -- the same
-- reason `SurrealFs._readable` exists. See `docs/permissions.md`.

-- Who the search runs as.
--
-- `$auth` wins whenever there is one: under record auth the credential *is* the
-- identity, so `$me` is ignored and cannot be used to search as somebody else.
-- `$me` is honoured only under a system credential, where `fn::sfs_me()` is
-- NONE and the caller already holds the database password -- there is nothing to
-- escalate to. That is what `SurrealFs` passes today.
--
-- Written in this order deliberately, and it must never be flipped to
-- `$me ?? fn::sfs_me()`: that lets a record user pass `$me = 'root'`, hit the
-- root short-circuit in the predicates below, and lean on the table's own
-- PERMISSIONS clause to save them. It does save them -- a SELECT in a plain
-- function body *is* filtered, verified on 3.2.4 and asserted in
-- `tests/test_record_auth.py` -- but this order does not need that to be true.
--
-- NONE throws rather than returning nothing. A system-credential caller who
-- forgets `$me` would otherwise get zero rows out of `fn::sfs_can_read`, and a
-- false empty is the expensive failure here: an agent told to search before it
-- writes reads it as "not there" and writes a duplicate. `SurrealFs(db,
-- user=...)` has no default for the same reason.
DEFINE FUNCTION OVERWRITE fn::sfs_searcher($me: option<string>) {
    LET $who = fn::sfs_me() ?? $me;
    IF $who IS NONE {
        THROW 'search needs an identity: pass $me, or connect as a record user'
    };
    RETURN $who;
};

-- Full-text search over file contents, best first.
--
-- `$match` defaults to 'any' (`@1,OR@`), the same deliberate default as
-- `SurrealFs.search_text`: one shared term is enough. This surface returns a
-- ranked list, so a weak hit is cheap to dismiss and an unseen one is not. Pass
-- 'all' for a precise filter that reads an empty result as a real answer.
--
-- 'all' is an `array::intersect` over analyzed tokens rather than the `@1@`
-- operator, because the operator is syntax and cannot come from `$match`. The
-- obvious alternative -- a second match reference, `content @1,OR@ $q AND
-- content @2@ $q` -- parses, plans, and is *silently wrong*: reference 2 is
-- ignored whenever reference 1 is present on the same field, so 'all' returned
-- exactly the 'any' rows (verified on 3.2.4). Comparing analyzed token sets is
-- the same question `@1@` answers, and it runs on the candidates reference 1
-- already produced.
-- ponytail: for 'all' only, that analyzes every candidate's content. Cheap
-- beside the row fetch, and 'all' is the rare mode; if it ever dominates, gate
-- the arm behind a separate function that can use `@1@` as its sole operator.
--
-- `content` is returned whole and the caller cuts its own snippet.
-- `search::highlight` returns the content unchanged, `search::offsets` returns
-- NONE, and there is no `string::index_of` or `string::find` on this build --
-- so a server-side snippet could only ever be "the first 160 characters", which
-- is worse than what a caller holding the content already does (`_snippet` in
-- `fs.py` centres on the first *unstemmed* match).
-- ponytail: a hit therefore costs its whole file on the wire. Cap it with
-- `string::slice(content, 0, 400)` if a corpus ever holds files big enough to
-- notice.
--
-- Root short-circuits the read predicate, keeping its plan free of the per-row
-- `gate` ancestry walk -- what `_readable` does by omitting the clause. Under
-- record auth nobody can *be* root: `record_auth.surql` refuses the name at
-- SIGNIN, and `fn::sfs_searcher` above ignores `$me` for a record user anyway.
--
-- `file` (the raw bytes) is deliberately not projected, unlike Python's
-- `_FIELDS_WITH_CONTENT`: `size` still reads it without shipping it. A full-text
-- hit is a text file by construction, and a remote client does not want a
-- megabyte of bytes per hit the way an in-process caller does not care.
DEFINE FUNCTION OVERWRITE fn::sfs_search_text(
    $q: string, $k: option<int>, $match: option<string>, $me: option<string>
) {
    LET $who = fn::sfs_searcher($me);
    LET $terms = array::distinct(search::analyze('fts_simple', $q));
    RETURN SELECT id, filename, path, content_type, is_folder, owner, mode, gate,
            hash, created_at, updated_at, content,
            IF content IS NOT NONE THEN string::len(content)
            ELSE IF file IS NOT NONE THEN bytes::len(file)
            ELSE 0 END AS size,
            search::score(1) AS score
        FROM file
        WHERE content @1,OR@ $q
          AND ($match != 'all' OR array::len(array::intersect(
                  $terms, search::analyze('fts_simple', content))) = array::len($terms))
          AND ($who = 'root' OR fn::sfs_can_read(gate, owner, mode, $who))
        -- `path` breaks ties, matching `hits.sort(key=(-score, path))` in
        -- `fs.py`. Ties are common, not exotic: see the BM25 note above -- every
        -- score is 0.0 for a term held by more than about half the corpus, and
        -- then this is the whole ordering.
        ORDER BY score DESC, path ASC
        LIMIT $k ?? 20;
};

-- Vector similarity search over the HNSW index, nearest first. Takes a
-- pre-computed embedding: SurrealFS never calls an embedding model and neither
-- does this. `SurrealFs.reindex_embeddings` or `just embed` fills the vectors.
--
-- `<|80,160|>` is a literal, and has to be. Binding k/ef as parameters parses
-- and returns the right rows but silently drops the index for a brute-force scan
-- (see AGENTS.md) -- and a function argument *is* a parameter, so `$k` cannot
-- appear here at all. SurrealQL has no dynamic execution to build it per call
-- with either. `EXPLAIN` must say `KnnScan`; `tests/test_search.py` asserts it,
-- because this is the failure mode that looks exactly like success.
--
-- 80 is the candidate pool, not the answer: HNSW applies its k cut *before* the
-- read predicate filters, so asking for k neighbours returns fewer than k
-- readable ones. 80 is the 20-hit default with a 4x over-fetch on top, which is
-- what `SurrealFs.search_semantic` used to widen `k` by in Python for exactly
-- this reason.
-- ponytail: the pool is fixed, so a `$k` above 20 is capped by whatever survives
-- the filter out of 80 rows, and root pays for an over-fetch it does not need
-- (Python narrows the literal per call, which a function body cannot). Raise the
-- literal if anyone wants deeper results.
DEFINE FUNCTION OVERWRITE fn::sfs_search_semantic(
    $qvec: array<float>, $k: option<int>, $me: option<string>
) {
    LET $who = fn::sfs_searcher($me);
    RETURN SELECT id, filename, path, content_type, is_folder, owner, mode, gate,
            hash, created_at, updated_at, content,
            IF content IS NOT NONE THEN string::len(content)
            ELSE IF file IS NOT NONE THEN bytes::len(file)
            ELSE 0 END AS size,
            vector::distance::knn() AS distance
        FROM file
        WHERE embedding <|80,160|> $qvec
          AND ($who = 'root' OR fn::sfs_can_read(gate, owner, mode, $who))
        ORDER BY distance ASC
        LIMIT $k ?? 20;
};

-- Both arms, fused by rank. `SurrealFs.search`, for the database.
--
-- Calls the two functions above rather than restating their queries: they
-- already return full rows carrying `id`, pre-sorted best first, which is all
-- `search::rrf` needs -- so the projection, the read predicate and the KNN
-- literal each exist exactly once. Without `$qvec` it is the text arm alone,
-- same as the Python -- but still routed through `search::rrf`, so `rrf_score`
-- is present either way. That keeps one promise for callers: the ranking column
-- is always `rrf_score`, and it is always comparable across the whole result
-- set, which a raw BM25 score in the one-arm case would not be.
--
-- `search::rrf` rather than the `array::find_index` summation it replaces: the
-- hand-rolled form collapses each arm to `.id`, fuses, then needs a second
-- `SELECT ... FROM <id array>` to get rows back -- which re-walks every `path`
-- and `gate` and needs the read predicate applied a second time. One more place
-- to forget the filter, in the function that returns file content.
--
-- It dedups on `id` where `_rrf` in `fs.py` fuses on `path`. Same grouping: a
-- path identifies exactly one row, because `child_unique` on
-- (parent_key, filename) makes a name unique at every level. It also ranks from
-- 1 where `_rrf` ranks from 0, so the fused numbers differ by a constant from
-- Python's for the same input -- the *order* is identical, which is all RRF
-- claims.
--
-- A fused row carries `rrf_score` and keeps its own arm's `score` (BM25) or
-- `distance` (cosine). Those two are not comparable with each other, which is
-- the whole reason for fusing on rank; they are left distinct rather than
-- overwritten so a caller sorting on the wrong one notices.
DEFINE FUNCTION OVERWRITE fn::sfs_hybrid_search(
    $q: string, $qvec: option<array<float>>, $k: option<int>,
    $match: option<string>, $me: option<string>
) {
    LET $lim = $k ?? 20;
    LET $text = fn::sfs_search_text($q, $lim, $match, $me);
    RETURN IF $qvec IS NONE
        THEN search::rrf([$text], $lim, 60)
        ELSE search::rrf([$text, fn::sfs_search_semantic($qvec, $lim, $me)], $lim, 60)
    END;
};

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
