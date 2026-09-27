# Next-Gen SurrealFS: Architecture & Engineering Plan

A comprehensive roadmap and technical specification for evolving **SurrealFS** into the premier **Agent Operating Substrate (AOS)** and filesystem for AI agents and human-agent collaboration.

---

## Revision Notes (v2)

This revision keeps every workstream from v1 and adds to it. The main changes:

1. **Server-side enforcement is now a foundational rule** (new §2). Permissions are enforced by SurrealDB, not by any SurrealFS client. Every client (Python, TypeScript, Rust CLI, FUSE daemon, MCP server, browser, indexer) connects as a principal and the database decides. Client-side checks survive only as a source of better error messages.
2. **CRDTs are fully client-transparent** (§11 rewritten). Agents keep calling `edit` and `write_file`; the SurrealFS binary and SDKs translate those into CRDT updates against SurrealDB, using the Yjs binary format shared by yrs (Rust), Yjs (JS) and pycrdt (Python).
3. **Every new table carries its own PERMISSIONS**, derived from the file it belongs to (§1.1 annotations).
4. **New workstreams**: optimistic concurrency via `generation` (§1.3), server-side history, provenance and undo (§7), derived links and frontmatter metadata (§8.3, §8.4), agent-facing tool additions such as `grep` (§15), and retrieval evals (§3.2).
5. **Design notes** added to branching, FUSE, the control plane, xattrs, the graph, section embeddings and watchers. They record interactions with the existing schema that the implementation has to respect.
6. **Verified engine behaviour** (Appendix A): the SurrealDB 3.2.4 semantics this plan relies on, re-tested on `3.2.4+20260803.93ab219`, including new results: `EVENT`s can abort writes with `THROW` and write history into client-locked tables, `$access` scopes service principals, and a hash-keyed `generation` counter supports compare-and-swap.
7. Upstream contributions (§25) and the roadmap (§26) are extended to cover the above.

## Revision Notes (v3)

v3 keeps everything in v2 and adds six sections, §19 to §24; the upstream and roadmap sections move to §25 and §26.

1. **Large files & content-addressed storage** (§19): content-defined chunking, BLAKE3-addressed chunks with scoped deduplication, tiered storage (inline, SurrealDB buckets, object stores via presigned URLs), resumable parallel uploads, range reads, sparse files, transparent zstd compression, and quotas with instant `du`.
2. **The automatic understanding pipeline** (§20): a job queue driven by writes; content-based type, language and encoding detection; type-aware chunking for code, documents, spreadsheets, notebooks, transcripts and more; rolled-up folder digests that never leak private files; token-aware reads; a `pack` tool for token budgets; auto-tagging and entity extraction; near-duplicate warnings.
3. **Languages & code intelligence** (§21): per-language full-text analyzers, multilingual and code-specific embeddings, a symbol index with navigation tools, broad tree-sitter coverage, git import/history/remote helper, and notebooks and tabular files as queryable data.
4. **Additional interfaces** (§22): WebDAV, NFSv3 loopback, an S3-compatible API, VS Code and JetBrains, Obsidian, a TUI, Raycast/Alfred, Slack/Teams bots, SFTP and a PWA/mobile app, all connecting as the end user's own principal.
5. **Connectors, webhooks & export** (§23): Drive, Notion, Confluence, Slack, email, GitHub and more mirrored as permission-mapped folders; signed webhooks dispatched as their owner; export and import at any point in time.
6. **Trust, sharing & operations** (§24): groups, share links on bearer access, at-rest and per-folder end-to-end encryption (with its search trade-off stated), retention and legal hold, incremental backups and point-in-time restore, and OpenTelemetry observability.
7. New tools in §15, new upstream asks in §25, new roadmap phases 6 to 9 in §26, and four more verified engine behaviours (A.13 to A.16) plus new open questions in Appendix A.

---

```
                                  ┌──────────────────────────────────────────────────────────┐
                                  │               SurrealDB 3.x Multi-Model Core             │
                                  │  • Server-side permissions (record auth, principals)     │
                                  │  • `file` table + computed paths + mode bit traversal    │
                                  │  • File Knowledge Graph (`references`, `implements`, …)  │
                                  │  • Temporal Snapshots & CoW Branching (`file_version`)   │
                                  │  • History, provenance & undo (EVENT-written)            │
                                  │  • Swarm Advisory Leases (`file_lock`) & CRDTs           │
                                  │  • Streaming Blob Storage (`file_chunk`, 512 KB)         │
                                  │  • Content-addressed chunks, dedup & blob tiers          │
                                  │  • Understanding pipeline (detect, chunk, digest, pack)  │
                                  │  • Hierarchical AST Embeddings (`file_section`, HNSW)    │
                                  │  • Per-language analyzers & symbol index                 │
                                  │  • Distributed Event Bus (Live Queries & Global Inotify) │
                                  └──────────────▲─────────────▲─────────────▲───────────────┘
                                                 │             │             │
                    Synthetic Control Plane      │             │             │ High-Fidelity POSIX & Edge
       ┌─────────────────────────────────────────┘             │             └──────────────────────────────────┐
       │                                                       │                                                │
┌──────┴─────────────────────────────────┐   ┌─────────────────┴───────────────────┐    ┌───────────────────────┴──────────────────────┐
│  Synthetic Procfs (`.surrealfs/`)      │   │          Rust CLI (`surrealfs`)     │    │         FUSE Mount Daemon (`fuser`)          │
├────────────────────────────────────────┤   ├─────────────────────────────────────┤    ├──────────────────────────────────────────────┤
│ Zero-tool agent control via Unix shell:│   │ • `mount` / `fork` / `merge`        │    │ • Inode Table Bi-directional Mapping         │
│ • `cat .surrealfs/status`              │   │ • `mcp` (sub-5ms stdio/SSE server)  │    │ • Kernel Page Cache + Chunk Prefetch         │
│ • `cat .surrealfs/locks/lease-1`       │   │ • `cp` / `sync` (parallel CoW)      │    │ • Live Query Kernel Cache Invalidation       │
│ • `rm .surrealfs/locks/stale_lock`     │   │ • `watch` (distributed cross-node)  │    │ • POSIX `xattr` Virtual Attribute Engine     │
│ • `cat .surrealfs/search/query?q=...`  │   │ • `snapshot` (temporal time-travel) │    │ • Synthetic `.surrealfs/` & `.graph/` Trees  │
└────────────────────────────────────────┘   └─────────────────▲───────────────────┘    └──────────────────────▲───────────────────────┘
                                                               │                                               │
                                                               ├───────────────────────────────────────────────┤
                                                               │ Multi-Surface & Zero-Privilege Integration    │
       ┌───────────────────────────────────────────────────────┼───────────────────────────────────────────────┤
       │                                                       │                                               │
┌──────┴─────────────────────────────────┐   ┌─────────────────┴───────────────────┐    ┌───────────────────────┴──────────────────────┐
│      Multi-Language Client SDKs        │   │       macOS Menubar Application     │    │   Zero-Privilege Runtime (WASI / Shim)       │
├──────────────────┬─────────────────────┤   │           (Native SwiftUI)          │    ├──────────────────────────────────────────────┤
│ Python Library   │ TypeScript/JS SDK   │   ├─────────────────────────────────────┤    │ • `libsurrealfs_shim.so` (libc LD_PRELOAD)   │
│ (`pip install`)  │ (`@surrealdb/fs`)   │   │ • Profile management & 1-click mount│    │ • `wasm32-wasip2` in-process virtual FS      │
│ • Hermes plugin  │ • Vercel AI / Node  │   │ • Live activity HUD & agent audit   │    │ • Cloud Run / Lambda / WebContainer support  │
│ • Pydantic-AI    │ • Direct Browser UI │   │ • Global Spotlight Search (⌘⇧S)     │    │ • Offline-first local WAL edge sync          │
└──────────────────┴─────────────────────┘   └─────────────────────────────────────┘    └──────────────────────────────────────────────┘
                                                               │
                                             ┌─────────────────┴───────────────────┐
                                             │     Spatial "Brain Studio" UI       │
                                             │          (Browser UI 2.0)           │
                                             ├─────────────────────────────────────┤
                                             │ • Real-time 2D/3D Multi-Agent Canvas│
                                             │ • Live agent presence & lock radar  │
                                             │ • 24-hour timeline playback scrubber│
                                             │ • Living multi-modal shadow previews│
                                             └─────────────────────────────────────┘
```

Every arrow into the core is an authenticated principal. No surface in this diagram holds a root, namespace or database credential at runtime; see §2.

Not drawn, for space: the v3 surfaces of §22 (WebDAV, NFS loopback, S3 API, SFTP, VS Code, JetBrains, Obsidian, TUI, launchers, chat bots, PWA) and the connectors of §23. Each sits beside the Rust CLI and connects the same way.

---

## Executive Summary

SurrealFS bridges the fundamental impedance mismatch between how AI agents work and how persistent enterprise databases operate:
- **For Agents**: It presents as a familiar Unix filesystem with folders, files, paths, atomic diff edits, and shell-friendly navigation, extended with zero-copy workspace branching, semantic knowledge graph traversal, conflict-free concurrent editing, undo, and an in-mount synthetic control plane (`.surrealfs/`).
- **For Humans & Infrastructure**: It is an ACID-compliant, multi-model, vector-indexed database in SurrealDB 3.x that persists across container lifecycles, branches deterministically, syncs across distributed nodes in real-time, and **enforces unix permissions inside the database itself**, so every client, in every language, is exactly as trustworthy as the schema.

This plan details the full evolution of SurrealFS: a **server-side security model**, a **pure Python client library**, a **native TypeScript/JavaScript SDK**, a **high-performance Rust CLI & FUSE POSIX mounter**, a **Deterministic Simulation Testing (DST)** suite, **Zero-Copy Branching and Time-Travel**, **History, Provenance & Undo**, **Living Multi-Modal Projections**, **Fine-Grained AST Vector Embeddings**, **Client-Transparent Distributed CRDTs**, **Cross-Machine Inotify/Mailboxes**, **Zero-Privilege Runtimes (WASI & `LD_PRELOAD` Shim)**, **Content-Addressed Large-File Storage**, an **Automatic Understanding Pipeline**, **Multi-Language & Code Intelligence**, **WebDAV, NFS, S3, SFTP, editor, Obsidian, chat and mobile interfaces**, **Permission-Mapped Connectors**, **Groups, Sharing, Encryption, Retention & Backups**, a **native macOS Menubar Application**, and the **Spatial Brain Studio Canvas**.

---

## 1. Storage & Schema Contract Evolution (`file.surql`)

SurrealFS's foundational rule is that **the database schema is the single source of truth**. Business rules, **permissions**, traversal restrictions, graph relationships, and rank fusion live in SurrealQL, ensuring all client surfaces behave identically. v2 makes the permissions part of that rule absolute (§2): a client is a convenience layer and never a security boundary.

### 1.1 Extended Tables & Storage Layout

Every table below is `SCHEMAFULL` and carries a `PERMISSIONS` clause. For tables that hang off a file, the clause is derived from that file through a *linked record* (`fn::sfs_can_read(fn::sfs_gate(file.parent), file.owner, file.mode, fn::sfs_me())` and its write equivalents). A permission predicate reads links with permissions off and does compute their COMPUTED fields, which is the same mechanism the `file` table's own clause already relies on (`fn::sfs_gate(parent)`, never `gate`).

#### 1. `file` Table Enhancements
- **Extended Attributes (`xattrs`)**: `option<object>` storing arbitrary key-value metadata for POSIX `xattr` support. Field permission: write requires the file's `w` bit, like `content`.
- **Chunking Flag (`chunked`)**: `bool DEFAULT false` indicating whether binary data is inline or split into `file_chunk` records.
- **File Generation (`generation`)**: `int` incremented on each content mutation (used as cache tag, NFS/FUSE file handle validator, **and the optimistic-concurrency token of §1.3**). Verified on 3.2.4 (Appendix A.6), the counter can be maintained entirely by the schema, keyed on the stored `hash` so that the `evt_file_hash` write-back and metadata-only writes do not bump it:
  ```surrealql
  DEFINE FIELD OVERWRITE generation ON file TYPE int
      VALUE IF $before IS NONE THEN 1
            ELSE IF crypto::md5($this.content ?? '') != $this.hash THEN $before + 1
            ELSE $before END
      PERMISSIONS FOR update WHERE false;
  ```
  In a field `VALUE` clause `$before` is *that field's* previous value, not the previous row, which is why the change is detected against `hash` rather than `$before.content`. Binary content (`file` bytes) needs `hash` to cover bytes too before this counts binary writes.
- **Workspace Branch (`branch`)**: `string DEFAULT 'main'` tagging records to enable zero-copy copy-on-write workspace forks. See the design notes in §6.3: this touches `child_unique`, `_resolve` and `gate`.
- **Collaborative Mode (`crdt`)**: `bool DEFAULT false` indicating whether concurrent writes are merged via CRDT state vectors. Because CRDTs are client-transparent in v2 (§11), this can become `DEFAULT true` for text content once the materialiser is proven.
- **Advisory Lease Fields**: `locked_by: option<string>` and `lock_expires: option<datetime>`. These are a denormalised view of `file_lock` for cheap `stat`/`ls` display; `file_lock` stays authoritative.
- **Frontmatter Metadata (`meta`)** *(new)*: `option<object>` holding parsed YAML frontmatter (tags, status, owner-of-record, dates), maintained by an EVENT on content change and indexable for filtered search. See §8.4.
- **Embedding field permissions** *(new)*: `embedding`, `embedded_at`, `embedded_hash` and `indexer_version` get `PERMISSIONS FOR update WHERE $access = 'indexer'`, closing the "left open" ponytail in today's schema. `hash` gets `FOR update WHERE false`: only `evt_file_hash` writes it, and an event's own writes are not subject to the triggering user's permissions (verified, Appendix A.8).

#### 2. `file_chunk` Table (Large Streaming Blobs)
For files exceeding 2 MB, data is split into 512 KB segments to avoid WebSocket frame choking and enable partial offset reads (`pread`):
```surrealql
DEFINE TABLE OVERWRITE file_chunk SCHEMAFULL
    PERMISSIONS
        FOR select WHERE fn::sfs_can_read(fn::sfs_gate(file_id.parent), file_id.owner, file_id.mode, fn::sfs_me())
        FOR create, update, delete WHERE fn::sfs_reachable(fn::sfs_gate(file_id.parent), fn::sfs_me())
                                     AND fn::sfs_w(file_id.owner, file_id.mode, fn::sfs_me());
DEFINE FIELD file_id     ON file_chunk TYPE record<file>;
DEFINE FIELD chunk_index ON file_chunk TYPE int;
DEFINE FIELD bytes       ON file_chunk TYPE bytes;
DEFINE INDEX idx_file_chunk ON file_chunk FIELDS file_id, chunk_index UNIQUE;
```
`rm` of a chunked file must delete its chunks in the same transaction (or via a `DELETE` EVENT on `file`), otherwise chunks outlive the permission row they derive from. v3 turns this table into a manifest over content-addressed blobs, with content-defined rather than fixed boundaries; see §19.

#### 3. `file_lock` Table (Swarm Advisory Leases)
Prevents concurrent agents in a swarm from overwriting each other's work:
```surrealql
DEFINE TABLE OVERWRITE file_lock SCHEMAFULL
    PERMISSIONS
        -- You can see a lease only on something you can read.
        FOR select WHERE fn::sfs_can_read(fn::sfs_gate(file.parent), file.owner, file.mode, fn::sfs_me())
        -- You can take a lease only on something you can write, and only as yourself.
        FOR create WHERE holder = fn::sfs_me()
                     AND fn::sfs_can_write(fn::sfs_gate(file.parent), file.owner, file.mode, fn::sfs_me())
        -- Renew or release your own; break an expired one.
        FOR update, delete WHERE holder = fn::sfs_me() OR expires_at < time::now();
DEFINE FIELD file        ON file_lock TYPE record<file>;   -- authoritative key (new)
DEFINE FIELD path        ON file_lock TYPE string;         -- display only
DEFINE FIELD holder      ON file_lock TYPE string;
DEFINE FIELD reason      ON file_lock TYPE string;
DEFINE FIELD expires_at  ON file_lock TYPE datetime;
DEFINE INDEX idx_lock_file ON file_lock FIELDS file UNIQUE;
```
v1 keyed the unique index on `path`. `path` is COMPUTED, so it cannot be indexed (SurrealDB rejects it) and a stored copy goes stale on `mv`; the lease is therefore keyed on the record id, and `path` is kept as a display field refreshed on read. A lease on a folder covers its subtree by ancestry, checked in `fn::sfs_acquire_lock`. Because `holder` is pinned to `fn::sfs_me()` by the database, a lease cannot be forged in someone else's name.

#### 4. `file_version` Table (Temporal Snapshots & CoW Differential Patches)
Stores immutable historical revisions for time-travel, audit trails, and instant rollbacks:
```surrealql
DEFINE TABLE OVERWRITE file_version SCHEMAFULL
    PERMISSIONS
        -- Evaluated against the snapshot on the version row, not the live file:
        -- the live file may have been deleted (`rm` is a hard DELETE) or chmod-ed since.
        FOR select WHERE fn::sfs_can_read(gate, owner, mode, fn::sfs_me())
        -- Written only by EVENTs on `file` (see §7). Never by a client.
        FOR create, update, delete NONE;
DEFINE FIELD file_id        ON file_version TYPE record<file>;
DEFINE FIELD version        ON file_version TYPE int;           -- = file.generation
DEFINE FIELD parent_version ON file_version TYPE option<int>;
DEFINE FIELD patch          ON file_version TYPE option<string>; -- diff against previous
DEFINE FIELD full_content   ON file_version TYPE option<string>; -- periodic snapshot
DEFINE FIELD author         ON file_version TYPE string;
DEFINE FIELD reason         ON file_version TYPE option<string>;
DEFINE FIELD created_at     ON file_version TYPE datetime VALUE time::now();
-- Permission snapshot (new): copied from the file at write time.
DEFINE FIELD owner          ON file_version TYPE string;
DEFINE FIELD mode           ON file_version TYPE int;
DEFINE FIELD gate           ON file_version TYPE option<string>;
DEFINE FIELD path           ON file_version TYPE string;
-- Provenance (new): see §7.2.
DEFINE FIELD op             ON file_version TYPE string;        -- write | edit | mv | rm | chmod | restore
DEFINE FIELD agent          ON file_version TYPE option<string>;
DEFINE FIELD session        ON file_version TYPE option<string>;
DEFINE FIELD model          ON file_version TYPE option<string>;
DEFINE INDEX idx_file_ver ON file_version FIELDS file_id, version UNIQUE;
```
A record link does not require its target to exist, so `file_id` survives a hard `rm`. What cannot survive is a permission derived from the deleted row, which is why the version carries its own `owner`/`mode`/`gate` snapshot.

#### 5. `file_section` Table (Hierarchical AST & Markdown Section Embeddings)
Replaces coarse full-file embeddings with surgical, language-aware AST and section chunking:
```surrealql
DEFINE TABLE OVERWRITE file_section SCHEMAFULL
    PERMISSIONS
        FOR select WHERE fn::sfs_can_read(fn::sfs_gate(file_id.parent), file_id.owner, file_id.mode, fn::sfs_me())
        FOR create, update, delete WHERE $access = 'indexer';
DEFINE FIELD file_id     ON file_section TYPE record<file>;
DEFINE FIELD section_idx ON file_section TYPE int;
DEFINE FIELD heading     ON file_section TYPE string;
DEFINE FIELD line_start  ON file_section TYPE int;
DEFINE FIELD line_end    ON file_section TYPE int;
DEFINE FIELD content     ON file_section TYPE string;
DEFINE FIELD embedding   ON file_section TYPE array<float, 1536>;
DEFINE FIELD source_hash ON file_section TYPE string;   -- file.hash at embed time (new)
DEFINE INDEX idx_section_unique ON file_section FIELDS file_id, section_idx UNIQUE;
DEFINE INDEX idx_section_hnsw   ON file_section FIELDS embedding HNSW DIMENSION 1536 DIST COSINE TYPE F32;
```
Staleness keys on `source_hash`, never on a timestamp, for the same reason `embedded_hash` exists on `file` today (the `VALUE time::now()` trap). The fixed `1536` ties the table to one embedder; the current `file.embedding` is `array<float>` plus `indexer_version`, and the section table should follow suit (one HNSW index per model, or a dimension chosen at `apply_schema` time).

#### 6. First-Class Semantic Graph Relations
Enables relational navigation between files as a traversable knowledge graph:
```surrealql
DEFINE TABLE OVERWRITE references   SCHEMAFULL TYPE RELATION IN file OUT file;
DEFINE TABLE OVERWRITE supersedes   SCHEMAFULL TYPE RELATION IN file OUT file;
DEFINE TABLE OVERWRITE derives_from SCHEMAFULL TYPE RELATION IN file OUT file;
DEFINE TABLE OVERWRITE implements   SCHEMAFULL TYPE RELATION IN file OUT file;
DEFINE TABLE OVERWRITE links_to     SCHEMAFULL TYPE RELATION IN file OUT file;  -- derived (new, §8.3)
```
Each relation table gets `PERMISSIONS FOR select WHERE <can read in> AND <can read out>`: an edge is visible only when both ends are, otherwise traversal leaks the existence (and path) of a private file. `FOR create, delete` requires write on `in`. `links_to` is written only by the link-extraction EVENT.

#### 7. `file_crdt` Table (Distributed Collaborative State Vectors)
Backs concurrent multi-agent editing without lock contention. In v2 this table holds each peer's **state vector** (what it has seen, for efficient sync), and a new `file_crdt_update` table holds the **append-only update log** that is the source of truth for a CRDT file (§11):
```surrealql
DEFINE TABLE OVERWRITE file_crdt SCHEMAFULL
    PERMISSIONS
        FOR select WHERE fn::sfs_can_read(fn::sfs_gate(file_id.parent), file_id.owner, file_id.mode, fn::sfs_me())
        FOR create, update, delete WHERE peer_owner = fn::sfs_me();
DEFINE FIELD file_id     ON file_crdt TYPE record<file>;
DEFINE FIELD peer_id     ON file_crdt TYPE string;
DEFINE FIELD peer_owner  ON file_crdt TYPE string;   -- principal running the peer (new)
DEFINE FIELD state_vec   ON file_crdt TYPE bytes;
DEFINE FIELD clock       ON file_crdt TYPE int;
DEFINE FIELD updated_at  ON file_crdt TYPE datetime VALUE time::now();
DEFINE INDEX idx_file_crdt ON file_crdt FIELDS file_id, peer_id UNIQUE;

-- New: the update log. Append-only; compacted into `file_crdt_snapshot`.
DEFINE TABLE OVERWRITE file_crdt_update SCHEMAFULL
    PERMISSIONS
        FOR select WHERE fn::sfs_can_read(fn::sfs_gate(file_id.parent), file_id.owner, file_id.mode, fn::sfs_me())
        FOR create WHERE author = fn::sfs_me()
                     AND fn::sfs_can_write(fn::sfs_gate(file_id.parent), file_id.owner, file_id.mode, fn::sfs_me())
        FOR update, delete NONE;
DEFINE FIELD file_id    ON file_crdt_update TYPE record<file>;
DEFINE FIELD seq        ON file_crdt_update TYPE int;      -- monotonic per file
DEFINE FIELD update     ON file_crdt_update TYPE bytes;    -- Yjs v1 update encoding
DEFINE FIELD author     ON file_crdt_update TYPE string;
DEFINE FIELD base_gen   ON file_crdt_update TYPE int;      -- generation the author had read
DEFINE FIELD created_at ON file_crdt_update TYPE datetime VALUE time::now();
DEFINE INDEX idx_crdt_seq ON file_crdt_update FIELDS file_id, seq UNIQUE;

DEFINE TABLE OVERWRITE file_crdt_snapshot SCHEMAFULL;      -- same select rule; compactor-only writes
DEFINE FIELD file_id ON file_crdt_snapshot TYPE record<file>;
DEFINE FIELD upto    ON file_crdt_snapshot TYPE int;       -- last seq folded in
DEFINE FIELD state   ON file_crdt_snapshot TYPE bytes;
```

### 1.2 Atomic SurrealQL Functions

Functions execute with the **caller's** permissions (verified, Appendix A.5), so exposing them to record users grants nothing the table rules do not. That lets each one also turn the database's *silent* denial (a denied statement returns zero rows, a denied field write is reverted without error) into a real error that every client, in every language, receives the same way.

- **`fn::sfs_append($id, $suffix, $if_generation)`**: Performs atomic in-database string concatenation without client-side read-modify-write roundtrips. v2 adds the optional concurrency token and the explicit failure:
  ```surrealql
  DEFINE FUNCTION OVERWRITE fn::sfs_append(
      $id: record<file>, $suffix: string, $if_generation: option<int>
  ) {
      LET $r = UPDATE $id SET content = (content ?? '') + $suffix
          WHERE $if_generation IS NONE OR generation = $if_generation;
      IF array::len($r) = 0 { THROW 'sfs:denied_or_conflict' };
      RETURN $r[0];
  };
  ```
  `updated_at` needs no explicit set: its `VALUE time::now()` already fires on every write.
- **`fn::sfs_acquire_lock($path, $holder, $ttl_seconds, $reason)`**: Atomic compare-and-swap lease acquisition with expiration check. `$holder` is ignored in favour of `fn::sfs_me()` under record auth; the parameter remains for system-credential admin use.
- **`fn::sfs_release_lock($path, $holder)`**: Safe lease release verifying holder identity.
- **`fn::sfs_fork_workspace($src_branch, $dst_branch)`**: Fast copy-on-write namespace clone tagging new branch generation.
- **`fn::sfs_create_snapshot($label)`**: Atomically captures current state across all files with an immutable snapshot tag.
- **`fn::sfs_restore_version($file_id, $version_id)`**: Restores a file to an exact prior revision with a reversible audit log entry.

**New: the core operations as functions.** Retrieval already lives in the schema (`fn::sfs_search_text`, `fn::sfs_search_semantic`, `fn::sfs_hybrid_search`). v2 moves the rest of `SurrealFs` there too, so the TypeScript SDK, the Rust crate and the Python library are thin callers of one implementation:

| Function | Replaces in `fs.py` | Notes |
|---|---|---|
| `fn::sfs_resolve($path)` | `_resolve` | Segment walk over `child_unique`, one round trip; a missing segment dead-ends (never falls back to `'root'`). |
| `fn::sfs_stat`, `fn::sfs_ls`, `fn::sfs_glob` | `stat`, `ls`, `glob` | Table permissions do the filtering; no `_readable` needed. |
| `fn::sfs_write($path, $content, $if_generation, $create_parents)` | `write_text`, `write_bytes`, `touch` | `touch` must write `content = ""`, never NONE. |
| `fn::sfs_edit($path, $old, $new, $if_generation)` | `edit` | Unique-match check inside the transaction. |
| `fn::sfs_mkdir($path, $parents)` | `mkdir` | `default_owner` / `default_mode` for `/home/<x>` stay schema rules. |
| `fn::sfs_mv`, `fn::sfs_cp`, `fn::sfs_rm`, `fn::sfs_chmod` | same | Recursive variants deepest-first; `FOR x IN` over a `LET`-bound subquery. |
| `fn::sfs_history`, `fn::sfs_diff`, `fn::sfs_restore` | new | §7. |
| `fn::sfs_grep($pattern, $glob, $limit)` | new | §15. |

Every function `THROW`s a stable, machine-readable code (`sfs:not_found`, `sfs:denied_or_missing`, `sfs:conflict`, `sfs:not_empty`, `sfs:exists`) that each SDK maps to its native error type and the FUSE daemon maps to an errno (§4.4).

### 1.3 Optimistic Concurrency *(new)*

The cheapest protection against lost updates in a swarm, and a complement to leases (coordination) and CRDTs (merging):

- Every read (`cat`, `stat`, `ls`) returns `generation`.
- Every mutation (`write_file`, `edit`, `append`, `mv`, `rm`, `chmod`) accepts an optional `if_generation`. On mismatch the function throws `sfs:conflict` with the current generation, and the tool text tells the model to re-read before retrying.
- The MCP server and Hermes plugin pass `if_generation` automatically for `edit` when the same session read the file, so agents get the protection without learning a new argument.
- Verified on 3.2.4: `UPDATE ... WHERE generation = $g` applies once and a stale second attempt returns zero rows (Appendix A.6); the function turns that into `sfs:conflict`.

---

## 2. Server-Side Security Model *(new)*

### 2.1 The rule

**Permissions are enforced by SurrealDB and nowhere else.** No SurrealFS client (the Python library, the TypeScript SDK, the Rust CLI, the FUSE daemon, the MCP server, the browser, the Hermes plugins, the indexer) is part of the trusted computing base. A client may pre-check to produce a friendlier error, but deleting every client-side check must not widen what any principal can read or write.

This reverses a decision recorded in `docs/permissions.md`, which kept enforcement in `SurrealFs` with record auth as an opt-in backstop on the grounds that "no tool exposes raw SurrealQL". v2 adds surfaces that make that argument untenable (a `.surrealfs/sql/` control file, a Rust and a TypeScript client, live-query watchers, a menubar HUD), and multi-language clients cannot share a Python boundary anyway.

### 2.2 What already exists

The database half is largely built. `schema/file.surql` defines the `file` table's `PERMISSIONS` unconditionally, and `schema/record_auth.surql` adds the identity half (a `user` table, `DEFINE ACCESS account`, no SIGNUP, `root` refused by name):

- `FOR select` calls the same `fn::sfs_can_read` the bulk queries use, with ancestry collapsed into `fn::sfs_gate(parent)`.
- `FOR create` / `FOR delete` key on the containing folder via `fn::sfs_can_enter`, plus `fn::sfs_home_ok`, and delete requires `!fn::sfs_has_children(id)`.
- `FOR update` is a coarse union (reachable AND (file `w` OR folder `w`+`x`)), evaluated against the before *and* after state, so a rename checks both folders.
- Field permissions make writes precise: `content`, `file`, `symlink`, `content_type` need `w`; `mode` needs ownership; `owner` is `WHERE false` (field permissions see only the after state, so any self-referencing rule on `owner` would be a takeover).
- `tests/test_record_auth.py` pins the fail-open traps (a four-deep chain against `gate` vs `fn::sfs_gate(parent)`; `fn::sfs_has_children` seeing invisible children).

### 2.3 What changes

1. **Record auth becomes the only runtime path.** `SURREALDB_AUTH_LEVEL=record` is the default in `_connect.py`, the MCP server, the browser, the Hermes surfaces and the Rust/TS clients. A system credential is used only by admin commands (`surrealfs schema apply`, `surrealfs users add`), never by a long-running surface.
2. **Principals.** Three kinds, each a `DEFINE ACCESS` method so that rules can distinguish them with `$access` (verified, Appendix A.4):
   - `account`: people and agents. `fn::sfs_me()` is their username. Exactly today's access method.
   - `indexer`: the embedding daemon. `FOR select` on `file` gains `OR $access = 'indexer'` (it must read every row to embed it), and it is the only principal that may write `embedding*`, `file_section` and projection outputs. It has no `w` on `content`. This replaces "the indexer runs as root".
   - `projector`, `compactor`: the same pattern for §9 shadow files and §11 CRDT compaction, each scoped to exactly the fields and tables it produces.
3. **Close the known ponytails in `file.surql`:**
   - *The loose update union*: a holder of a file's `w` bit can currently *move* it without `w`+`x` on either folder. An `EVENT` sees `$before`, `$after` and `$auth` and can abort with `THROW` (verified, Appendix A.3), so:
     ```surrealql
     DEFINE EVENT OVERWRITE evt_file_move ON file
         WHEN $event = 'UPDATE' AND ($before.parent != $after.parent OR $before.filename != $after.filename)
         THEN {
             IF $auth IS NOT NONE AND !(fn::sfs_can_enter($before.parent, fn::sfs_me())
                                   AND fn::sfs_can_enter($after.parent, fn::sfs_me())) {
                 THROW 'sfs:denied: rename needs w+x on both folders';
             };
         };
     ```
     This also gives a rename a *real error* instead of the silent zero-row denial.
   - *Open derived fields*: `hash` and the embedding fields get field permissions (§1.1).
   - *Content-vs-rename ambiguity*: with the event handling renames precisely, the table's `FOR update` can be tightened toward "reachable AND file `w`", with the event carrying the folder rule.
4. **Zero-provisioning onboarding**, the reason record auth was not the default. The single-agent case must stay one command:
   - `surrealfs init` (Python and Rust CLIs) applies the schema with a system credential, creates the first `account` user, writes `~/.config/surrealfs/env` with the record credential, and never persists the system credential.
   - `surrealfs-mcp --selftest` reports the auth level and fails when a long-running surface is holding a system credential.
   - For SurrealDB Cloud, the same flow runs against the instance's token endpoint.
5. **Error semantics.** A denied statement returns zero rows and a denied field write is reverted without error. Both are silent (Appendix A.2). Every write goes through a `fn::sfs_*` function that `THROW`s (§1.2), so no client can report "wrote /x" when nothing was written. Read-side, an unreadable intermediate folder makes a path look absent, so resolution yields `sfs:not_found` where a system credential would say "permission denied". That leaks less and is the intended behaviour; the FUSE daemon maps it to `ENOENT`, and the tool docs say "not found or not permitted".
6. **Listings hide names.** Under record auth `ls /home` shows only homes you can read. This is already the documented behaviour and becomes the only behaviour.
7. **Search.** The HNSW `k` cut happens before the permission filter, so `fn::sfs_search_semantic` keeps its literal `<|80,160|>` pool and `LIMIT`s after the predicate. The score-channel caveat in `docs/permissions.md` (IDF computed over the whole index) remains; per-tenant statistics still mean a table per tenant.

### 2.4 Consequences for the rest of this plan

- **`.surrealfs/sql/`** (§5.2) is safe: a query runs as the mounting principal and sees exactly what the tools see.
- **Live queries** (§4.3 FUSE invalidation, §12 watchers, §16 presence, §17 HUD) respect table permissions only for record users, so every subscriber must be one. A watcher running as root would stream every user's changes; the daemons therefore connect as the mounting or logged-in principal, never as root.
- **Every new table** in §1.1 carries its own rules, derived via linked records, and `file_version` carries a permission snapshot because its file may be gone.
- **Graph edges** are visible only when both endpoints are readable.
- **Docker/Kubernetes** (§18) mount as a record user; a root credential in a sidecar would re-open the whole tree.

### 2.5 Tests

- Run the full `tests/` suite under record auth as well as a system credential (a parametrised fixture), with system-credential runs limited to admin paths.
- A **deletion test**: delete every `_require` / `_readable` / `_require_writable_dir` call in `fs.py` on a scratch branch and assert the permission suite still passes. That is the definition of done for §2.
- Extend `tests/test_record_auth.py` to every table in §1.1, the rename event, the indexer principal and the silent-denial wrappers.

---

## 3. Deterministic Simulation Testing (DST) Workstream

Following the formal engineering standards of FoundationDB and ShaleDB, concurrent filesystem behavior, crash safety, and multi-agent locking are verified using deterministic simulation.

### 3.1 The Simulator Architecture (`crates/surrealfs-sim`)
- **Deterministic Virtual Time**: Replaces wall-clock time with discrete logical ticks.
- **Fault-Injected Storage & Network Layer**:
  - Reorders, drops, duplicates, and truncates WebSocket packets between client and SurrealDB.
  - Injects simulated disk write stalls, torn page writes, and out-of-order flushes.
  - Simulates sudden process death (`SIGKILL`), broken pipes during FUSE `flush()`, and backpressure stalls.
- **State Invariant Oracles**:
  - **Tree Hierarchy Invariant**: No cycle can exist in `parent` chains; no orphaned records where `parent_key` points to a deleted node.
  - **Security & Ancestry Invariant**: If an ancestor is mode `0700`, no descendant can be reached by a non-owner under any query or search.
  - **Server-Side Enforcement Invariant** *(new)*: for any principal and any sequence of raw SurrealQL statements (not only SurrealFS API calls), the set of rows the principal can read or change equals what the unix model allows. The simulator drives the database directly, bypassing every client.
  - **No Silent Success Invariant** *(new)*: a mutation that reports success changed the database; a denied or conflicting one reports an error.
  - **FUSE / DB Quiescence Invariant**: In-memory inode maps, directory entries, and database rows match byte-for-byte once events settle.
  - **Lock Exclusivity Invariant**: Two simulated agents can never hold an active lease on the same path at the same logical tick.
  - **Branch Isolation Invariant**: Mutations in branch `A` cannot alter the content or metadata of branch `B` prior to an explicit merge.
  - **CRDT Convergence Invariant**: Concurrent agent edits across arbitrary partition interleavings converge to identical file contents.
  - **Materialisation Invariant** *(new)*: for a CRDT file, `content` equals the deterministic merge of its snapshot plus update log, and FULLTEXT/HNSW results reflect that `content`.
  - **History Completeness Invariant** *(new)*: every content change has exactly one `file_version` row, including changes made by raw SurrealQL.
- **Seed-Driven Reproducibility**:
  - Every simulation run is parameterized by a 64-bit seed (`SURREALFS_SIM_SEED=...`).
  - Failures output a minimal shrunk event trace that can be replayed identically in a debugger.
- **Scope note** *(new)*: the SurrealDB server is an external process, so the simulator controls the client, network and FUSE layers deterministically and treats the server as a black box behind the fault-injecting proxy. Server-internal interleavings are covered by the property tests below rather than by the simulator.

### 3.2 Property-Based Tests & Retrieval Evals *(new)*

Available immediately, in the existing Python suite, against a real 3.x server:

- **Property tests (hypothesis)** for the invariants above that do not need virtual time: random trees, random modes, random principals, random operation sequences, each checked against a pure-Python reference model of unix permissions. The Security, Server-Side Enforcement and No Silent Success invariants come first.
- **Retrieval evals** extending `test_ranking_quality_over_a_realistic_corpus` (MRR today: 0.833 server-side BM25). Add an agent-task set ("which file answers this question") scored for whole-file embeddings, section embeddings (§10), hybrid, and graph-expanded retrieval (§8). §10's "up to 80%" token reduction and any change to ranking land with a measured number, in the same way the stemming and stopword experiments were decided.

---

## 4. Rust Core: CLI & FUSE (POSIX) Mounter

A high-performance static binary (`surrealfs`) written in Rust using `clap`, `tokio`, `fuser` (libfuse/macFUSE bindings), and `surrealdb.rs`.

### 4.1 Subcommand Layout
```bash
surrealfs init                   # (new) apply schema, create first user, write record-auth config
surrealfs mount <MOUNTPOINT>     # Mount SurrealFS as a local POSIX drive (--branch, --as-of)
surrealfs fork <SRC> <DST>       # O(1) Copy-on-Write workspace branching
surrealfs merge <SRC> <DST>      # 3-way AST & semantic merge of branch back to parent
surrealfs snapshot <TAG>         # Create immutable point-in-time snapshot
surrealfs history <PATH>         # (new) versions, authors, agents (§7)
surrealfs restore <PATH> <VER>   # (new) undo to a version (§7)
surrealfs watch <PATTERN>        # Distributed cross-node file watcher (--exec <CMD>)
surrealfs mcp                    # Run ultra-fast stdio/SSE MCP server
surrealfs status                 # Healthcheck, latency, auth verification (reports auth level)
surrealfs schema apply           # Apply or migrate schema on target DB (system credential)
surrealfs cp <SRC> <DST>         # Multi-threaded parallel recursive copy
surrealfs sync <LOCAL> <REMOTE>  # Two-way differential rsync-like synchronization
surrealfs lock [acquire|release] # Manage swarm advisory leases from the terminal
surrealfs users [add|list|del]   # Manage record-auth identities (system credential)
```

### 4.2 FUSE Mounter Architecture (`surrealfs mount`)
1. **Inode Map Management**:
   - Bi-directional lock-free mapping between 64-bit numeric POSIX inodes and SurrealDB `RecordID`s.
   - Root directory is mapped to inode 1. Children are allocated monotonically and cached in an LRU arena.
   - Never derive a key string from a `RecordID` with its display form; build it from table and id, as `_parent_key` does in Python (the `file:⟨…⟩` escaping regression).
2. **Read / Write Execution**:
   - **`open()` / `read()`**: Files under 2 MB are cached in memory; larger files fetch 512 KB chunks on demand.
   - **`write()` / `flush()`**: Appends and writes are buffered per file descriptor and flushed as atomic SurrealQL updates on `close()` or `fsync()`.
   - **Base-version tracking** *(new)*: each file descriptor remembers the `generation` it opened. `flush()` sends `if_generation` for non-CRDT files (returning `EAGAIN`/`ESTALE`-style errors on conflict) and diffs against that base for CRDT files (§11).
3. **Live Query Kernel Cache Invalidation**:
   - The mount daemon maintains a persistent `LIVE SELECT DIFF FROM file;` stream.
   - When an agent in Python, an MCP client, or another machine mutates a file, the daemon receives the diff in real time and calls `fuser::Session::invalidate_inode()` / `invalidate_entry()`.
   - **Result**: Remote agent edits appear instantly in local IDEs and terminal windows without stale cache delays.
   - **Design note** *(new)*: the stream runs as the mounting principal (§2.4), so it only ever carries rows that principal can read. Whether COMPUTED fields (`path`, `gate`) appear in live-query diffs is to be verified; invalidation should key on record id and `parent_key`, which are stored.
4. **Identity & errno mapping** *(new)*:
   - The daemon authenticates as a record user; `--allow-other` mounts are refused unless each accessing uid is mapped to its own principal (otherwise every local user acts as the mounting one).
   - `sfs:not_found` → `ENOENT` (including "not permitted to see"), `sfs:denied_or_missing` → `EACCES`, `sfs:conflict` → `ESTALE`, `sfs:not_empty` → `ENOTEMPTY`, `sfs:exists` → `EEXIST`.

---

## 5. POSIX Extended Attributes (`xattr`) & Synthetic Control Plane (`.surrealfs/`)

SurrealFS bridges Unix toolchains and database capabilities through both metadata attributes and an in-mount synthetic filesystem.

### 5.1 Extended Attributes (`xattr`) Engine
Surfaces SurrealDB record metadata and agent controls directly via standard Unix attribute utilities (`xattr` on macOS, `getfattr`/`setfattr` on Linux):
```bash
# Read core SurrealDB record attributes
$ xattr -p user.surrealfs.owner /mnt/brain/projects/todo.md
martin

$ xattr -p user.surrealfs.hash /mnt/brain/projects/todo.md
d41d8cd98f00b204e9800998ecf8427e

# Inspect vector embedding staleness
$ xattr -l /mnt/brain/projects/todo.md
user.surrealfs.id: file:⟨01J8...⟩
user.surrealfs.owner: martin
user.surrealfs.mode: 0666
user.surrealfs.generation: 4
user.surrealfs.embedded_at: 2026-09-25T14:30:00Z
user.surrealfs.indexer_version: openai:text-embedding-3-small

# Set custom metadata directly from shell (persisted in database row)
$ xattr -w user.agent.task_id "TASK-4021" /mnt/brain/projects/todo.md

# Trigger actions via virtual attributes
$ xattr -w user.surrealfs.action "reindex" /mnt/brain/projects/todo.md
$ xattr -w user.surrealfs.lock "lease:60s:refactoring" /mnt/brain/projects/todo.md
```

**Design notes** *(new)*:
- **Control attributes are write-only.** `user.surrealfs.action` and `user.surrealfs.lock` are accepted by `setxattr` but never returned by `listxattr`. Copy tools (`cp -a`, `rsync -X`, Finder, `tar --xattrs`) copy what `listxattr` returns, so without this a plain copy of a file would re-trigger a reindex or take a lease on the destination. Lease *state* is read through `user.surrealfs.locked_by` (read-only) instead.
- **Read-only record attributes** (`id`, `owner`, `hash`, `generation`, `embedded_at`, `indexer_version`) reject `setxattr` with `EPERM`; the database field permissions reject it anyway (§2).
- User attributes (`user.agent.*`) live in `file.xattrs` and need the file's `w` bit.

### 5.2 Synthetic Kernel Control Plane (`.surrealfs/`: "Procfs for AI Agents")
Exposes database internals, swarm leases, and search engines as synthetic virtual files right inside the mount. Any off-the-shelf agent or shell script can inspect and control the database **without needing custom MCP tools or SDKs**:

```
/mnt/brain/.surrealfs/
├── status                      # cat returns JSON health, version, DB ping latency, auth level (new)
├── stats                       # total files, chunk counts, vector indexing queue
├── whoami                      # (new) principal, access method, home
├── locks/                      # Virtual directory of active leases
│   ├── projects_auth           # cat reveals holder, reason, ttl
│   └── incident_42             # rm projects_auth breaks/releases the lease!
├── agents/                     # Live agent presence & heartbeats
│   ├── triage-bot.json
│   └── code-refactor-3.json
├── search/                     # Virtual search query engine
│   └── query?q=auth&type=knn   # Reading this virtual file executes semantic search!
├── grep/                       # (new) cat "grep/TODO" → path:line:text, like grep -rn
├── history/                    # (new) history/<path> → versions of that file (§7)
├── graph/                      # (new) graph/<path>/ → the file's edges (alternative to §8.1's .graph/)
├── branches/                   # Active workspace branches
│   └── agent-42/               # Switch or inspect branch state
└── sql/                        # Echo a SurrealQL query into here, cat output
```

**Why this is revolutionary**: An agent running in a locked-down container with only `sh`, `cat`, and `rm` can query semantic memory with `cat ".surrealfs/search/query?q=database migrations"` and manage swarm locks with standard Unix file operations.

**Design notes** *(new)*:
- Everything here executes as the mounting principal (§2). `locks/` lists only leases on files the principal can read; `rm` of a lease succeeds only for the holder or once expired, enforced by `file_lock`'s own permissions. `sql/` is therefore safe: it is exactly as powerful as the principal.
- Search queries encoded as filenames contain `?`, `&`, `=` and spaces; lookup must accept arbitrary UTF-8 names below `search/` and never cache negative dentries there. A directory form (`search/knn/database migrations`) is friendlier to shells and is offered alongside.
- `rm .surrealfs/locks/<name>` should report whose lease was broken to `stderr`-visible `status`, so a broken lease is never silent to its holder (their next write gets `sfs:conflict` via §1.3).

---

## 6. Zero-Copy Branching, Sandboxed Workspaces & Time-Travel

AI agents frequently hallucinate, execute speculative refactorings, or need isolated sandboxes to run test suites. SurrealFS provides native copy-on-write branching and temporal time-travel directly in the filesystem.

### 6.1 O(1) Agent Workspace Forks (`surrealfs fork`)
Instead of duplicating gigabytes of files, SurrealFS creates lightweight isolated branches using record pointers:
```bash
# Fork main workspace into a sandboxed agent branch
$ surrealfs fork main agent-task-402
Created branch 'agent-task-402' in 12ms (O(1) pointer fork)

# Mount the isolated branch
$ surrealfs mount /mnt/sandbox --branch agent-task-402
```
- **Copy-on-Write (CoW)**: Reads fall back to the base branch (`main`); writes allocate new `file` and `file_chunk` records tagged with `branch = 'agent-task-402'`.
- **Atomic 3-Way Merge**:
  ```bash
  $ surrealfs merge agent-task-402 main --strategy=3way
  ```
  SurrealFS leverages AST-aware tree-sitter diffing to merge code files cleanly, detecting true semantic conflicts. For files in CRDT mode (§11), the merge is the CRDT merge of the two update logs since the fork point.
- **Instant Rollback / Discard**: If the agent fails verification tests, deleting the branch takes a single `DELETE file WHERE branch = 'agent-task-402'`.

### 6.2 Temporal Time-Travel & Magic `.snapshots/`
- **Mount Historical Points in Time**:
  ```bash
  $ surrealfs mount /mnt/brain-yesterday --as-of "2026-09-26T10:00:00Z"
  ```
- **Magic `.snapshots/` Directory**: Inode lookup intercepts `.snapshots/` to expose virtual folders for every named checkpoint:
  ```bash
  $ ls -la /mnt/brain/.snapshots/
  drwxr-xr-x  2 root root 2026-09-25-1800-checkpoint
  drwxr-xr-x  2 root root 2026-09-26-0930-pre-refactor
  $ diff /mnt/brain/main.rs /mnt/brain/.snapshots/pre-refactor/main.rs
  ```
- `--as-of` and `.snapshots/` read from `file_version` (§7), and are filtered by each version's permission snapshot: a file that was private at the time stays private in the past.

### 6.3 Design Notes *(new)*
The `branch` field interacts with four existing mechanisms, each of which the implementation must carry:
1. **Uniqueness**: `child_unique` becomes `(branch, parent_key, filename)`; keep `parent_key` before `filename` so `WHERE branch = $b AND parent_key = $k` stays an index scan (verify with `EXPLAIN`, as was done for today's order).
2. **Resolution with fallback**: the segment walk in `fn::sfs_resolve` looks up `(branch, parent_key, filename)` and falls back to the base branch per segment, which needs **whiteout rows** (a branch-local tombstone) so that a file deleted in the branch does not reappear from `main`. `rm` stays a hard DELETE in `main`; whiteouts exist only in branches and are removed by merge or discard.
3. **Listing**: `ls`, `glob` and both search arms return the union of branch and base rows minus whiteouts and minus base rows shadowed by a branch row of the same name.
4. **Permissions**: `gate` recurses through `parent`. A branch-local copy of a folder must point its children's `parent` at the branch copy for `gate` and `path` to stay correct, so a CoW write to a deep file copies its ancestor chain into the branch (O(depth), not O(1)). A `chmod` in a branch is a branch-local change.

An alternative with far fewer moving parts: **a branch is a SurrealDB database** (`USE DB brain__agent_task_402`), forked by export/import or by copying only changed rows from `file_version`. Discard is `REMOVE DATABASE`. Isolation is absolute and every existing query works unchanged; the cost is a real copy on fork. Prototype both behind `fn::sfs_fork_workspace` and choose on measurements.

---

## 7. History, Provenance & Undo *(new)*

### 7.1 History written by the database
An `EVENT` on `file` writes a `file_version` row for every content change, rename, chmod and delete. Because it is an event rather than client code, no client, raw query or future SDK can skip it. Verified on 3.2.4: events see `$before`, `$after` and `$auth` (Appendix A.3), and an event triggered by a record user can write to `file_version` even though its `FOR create` is `NONE`, with `$auth` still the triggering user (Appendix A.8).

One trap, also verified: an event's own writes trigger events. `evt_file_hash` writes `hash` back after every content change, so a history event keyed on `hash` would fire on that write-back, when `$before.content` is already the *new* content. The event below therefore keys on `content` itself, which the write-back does not change.

```surrealql
DEFINE EVENT OVERWRITE evt_file_version ON file
    WHEN $event = 'DELETE'
      OR $before.content != $after.content OR $before.file != $after.file
      OR $before.parent != $after.parent OR $before.filename != $after.filename
      OR $before.mode != $after.mode
    THEN {
        LET $row = IF $event = 'DELETE' THEN $before ELSE $after END;
        CREATE file_version SET
            file_id = $row.id, version = $row.generation,
            full_content = $before.content,           -- patch/snapshot policy in 7.3
            author = fn::sfs_me() ?? 'root',
            owner = $before.owner, mode = $before.mode,
            gate = fn::sfs_gate($before.parent), path = $before.path,
            op = <derived from which fields changed>,
            session = $session.id,
            agent = $sfs_agent, model = $sfs_model;   -- see 7.2
    };
```

### 7.2 Provenance
Every version should answer *who, which agent, which model, when, why*. `$session` has a fixed shape (`ac`, `db`, `id`, `ip`, `ns`, `or`, `rd`, `tk`) and cannot carry custom fields (verified, Appendix A.11), so the MCP/Hermes/Pydantic-AI surfaces supply `agent` and `model` either as connection-level parameters (`db.let('sfs_agent', ...)`, if they prove visible inside events) or as explicit arguments to the `fn::sfs_*` mutations, which set them for the event. Per-agent access methods with custom token claims (read through `$token`) are the stronger option where each agent has its own credential. This gives:
- `blame <path>`: per-line attribution from the version chain.
- An **agent audit log** for the menubar HUD (§17) and Brain Studio (§16) without a separate telemetry pipeline.

### 7.3 Storage policy
Full snapshot every N versions (or when a patch exceeds a size ratio), unified-diff patches in between, and a retention setting per folder. Binary files store snapshots only.

### 7.4 Tools
- `history(path, limit)`: versions with author, agent, op, size, timestamp.
- `diff(path, from, to)`: unified diff between two versions or a version and current.
- `restore(path, version)`: writes the old content as a *new* version (never rewrites history), with `op = 'restore'`.
- `undelete(path)`: restores the last version of a deleted path, subject to write permission on its folder. This gives `rm` a safety net while keeping `rm` a hard DELETE (a soft delete would block the name in `child_unique`).

---

## 8. Multi-Model File Knowledge Graph (Relational Filesystem)

Standard filesystems limit relationships to parent/child directories and symlinks. SurrealDB is natively a graph database. SurrealFS turns file storage into a **traversable, bidirectional semantic knowledge graph**.

### 8.1 Virtual Relation Directories in FUSE
Surface graph edges directly as virtual filesystem links under each file's `.graph/` subfolder:
```bash
$ ls -l /mnt/brain/docs/architecture.md/.graph/
lrwxr-xr-x 1 agent agent -> /mnt/brain/src/auth/jwt.rs (implements)
lrwxr-xr-x 1 agent agent -> /mnt/brain/rfcs/0042.md (references)
lrwxr-xr-x 1 agent agent <- /mnt/brain/docs/v1_arch.md (supersedes)
```
- Agents can traverse dependencies using standard shell utilities (`cd`, `ls -l`, `cat`).
- Creating a symlink inside `.graph/` creates the corresponding graph relationship edge in SurrealDB:
  ```bash
  $ ln -s /mnt/brain/src/db.rs /mnt/brain/docs/schema.md/.graph/implements
  ```
- **Design note** *(new)*: `architecture.md/.graph/` makes a regular file also enterable as a directory. `stat` reports `S_IFREG`, so `find`, `rsync`, editors' file watchers and Finder will either never see `.graph/` or error with `ENOTDIR`. Keep it as an opt-in mount flag (`--graph-dirs`), and expose the same edges by default at `.surrealfs/graph/<path>/` (§5.2), which is plain POSIX. Edge symlinks are named `<relation>--<target-basename>` so several edges of one relation can coexist in one directory.

### 8.2 Dependency Closure Queries in SDKs
```python
# Pull the exact contextual dependency closure of a file in one round trip
dependencies = await fs.get_neighbors("/docs/architecture.md", relation="implements", depth=2)
```
Traversal filters every hop with the relation tables' permissions (§1.1), so a closure never passes through a file the caller cannot read.

### 8.3 Derived Links & Backlinks *(new)*
Hand-curated edges drift. An `EVENT` on content change parses markdown links (`[text](../rfcs/0042.md)`), wikilinks (`[[0042]]`) and, for code, import statements, resolves them against the tree, and maintains `links_to` edges. This gives:
- `backlinks(path)`: everything that points at a file, the question agents ask before editing or deleting one.
- **Broken-link detection** after `mv`/`rm` (an edge whose target no longer resolves), surfaced in `.surrealfs/stats` and `history`.
- A graph that exists from day one on any existing corpus, which makes §8.1 and §16's canvas useful before anyone curates `implements`.

### 8.4 Frontmatter as Queryable Metadata *(new)*
The same event parses YAML frontmatter into `file.meta`. Search and `glob` gain filters (`meta.status = 'open'`, `'okta' IN meta.tags`), and the company-brain skills can ask for "all open risks" as a query rather than a full-text guess.

---

## 9. Living Multi-Modal Projections (Auto-Derived Shadow Files)

When an agent or human drops unstructured or binary data into SurrealFS, the filesystem does not treat it as a black box.

### 9.1 Reactive Projections Driven by Events
Using SurrealDB `EVENT`s and an asynchronous background projection worker, non-text files project virtual shadow views:
- **Drop `spec.pdf`** → automatically projects:
  - `spec.pdf.md` (clean extracted markdown for text-only LLMs)
  - `spec.pdf.summary.md` (high-level executive briefing)
  - `spec.pdf.entities.json` (extracted graph nodes & keywords)
- **Drop `dataset.parquet` or `transactions.sqlite`** → exposes:
  - `dataset.parquet/` as a virtual directory with `schema.sql`, `head.csv`, and `stats.json`.
- **Drop `recording.mp3`** → exposes:
  - `recording.mp3.transcript.md` with speaker diarization timestamps.

Shadow files are exposed via FUSE as read-only virtual files generated on first access or cached eagerly.

### 9.2 Design Notes *(new)*
- The worker runs as the `projector` principal (§2.3), which may read sources and write only projection rows.
- A projection **inherits the source's readability**: its permission clause is derived from the source file via a linked record, so a summary of a private PDF is never more visible than the PDF.
- Projections key staleness on the source `hash`, like embeddings, and are themselves indexed for FULLTEXT and HNSW search, which is where most of their value to agents comes from.

---

## 10. Hierarchical Multi-Vector Chunking & AST Embeddings

Full-file embeddings degrade search precision on large codebases and documents. SurrealFS implements hierarchical AST and markdown section embeddings.

### 10.1 Language-Aware Tree-Sitter & Markdown Chunking
- Code files (`.rs`, `.py`, `.ts`, `.go`) are parsed with Tree-sitter into syntactic units (functions, structs, classes, modules).
- Markdown files are split along heading boundaries (`#`, `##`, `###`).
- Chunks populate the `file_section` table with exact line ranges (`line_start`, `line_end`) and vector embeddings.
- v3 generalises this to every content type, chosen automatically from the detected type and language; see §20.3 and §21.4.

### 10.2 Surgical Retrieval in Search & MCP
Instead of handing 50KB files to an agent, `search_semantic` returns the exact section match with surrounding context, slashing LLM prompt token consumption by up to 80%:
```json
{
  "path": "/src/auth/jwt.rs",
  "heading": "fn verify_token",
  "lines": "142-178",
  "score": 0.892,
  "snippet": "pub async fn verify_token(token: &str) -> Result<Claims> { ... }"
}
```

### 10.3 Design Notes *(new)*
- **Pool starvation.** HNSW applies `k` before the permission filter, which is why `fn::sfs_search_semantic` searches a literal pool of 80. With one row per section, a single large file can occupy the whole pool. Section search needs a larger literal pool, per-file deduplication after the permission filter (best section per file, plus its neighbours), and a test that asserts a full `k` of *distinct readable files* comes back when the nearest sections all belong to one unreadable file.
- **Hybrid.** `fn::sfs_hybrid_search` fuses the section arm with the existing BM25 arm by RRF at the file level, returning the best section as the snippet.
- **Line ranges** pair with the new `read_range` tool (§15), so a hit is directly actionable.
- **Adoption gate.** Ship behind a flag and switch the default when the §3.2 evals show the gain.

---

## 11. Distributed Collaborative CRDTs (Real-Time Swarm Documents)

Advisory leases serialize access, but when multiple agents collaborate in parallel on shared files (e.g. incident reports, task backlogs, or living specs), lock contention creates bottlenecks.

### 11.1 Conflict-Free Replicated State Vectors
- Files can declare `crdt = true` to enable collaborative multi-agent editing.
- Backed by **Automerge** or **Yjs** binary state vectors stored in `file_crdt`. v2 selects **Yjs** (see 11.3).
- Multiple agents can concurrently append, insert, or modify sections:
  - Agent A writes the "Root Cause Analysis" section.
  - Agent B writes the "Action Items" table.
- Changes propagate over SurrealDB Live Queries and converge deterministically without merge conflicts or lost updates.

### 11.2 Client-Transparent CRDTs *(new)*
**An agent never writes a CRDT mutation.** It calls `edit`, `write_file` or `append`, or writes through the FUSE mount, exactly as for any other file. The SurrealFS binary and SDKs translate those calls into CRDT updates against SurrealDB:

| Agent operation | Translated to |
|---|---|
| `edit(path, old, new)` | Locate `old` in the **base version** the agent read, then a Y.Text delete + insert at that position. This is naturally a local operation, so it merges cleanly with concurrent edits elsewhere in the file. |
| `write_file(path, content)` | Diff the base version against `content` (Myers/patience at line granularity) and emit the equivalent inserts/deletes. |
| `append(path, text)` | Insert at the end of the document. |
| FUSE `write()`+`flush()` | Diff the per-fd base version (§4.2) against the flushed buffer. |

**The base version is essential.** Diffing against the *current* content instead would silently revert every concurrent change made since the agent read the file. Each surface therefore records what the agent saw: per session and path in the MCP server and Hermes plugin (populated by `cat`, `tail`, `read_range`), per file descriptor in FUSE. If no base is known (a blind `write_file`), the write is treated as "replace the whole document" and recorded as such in history, so the loss is explicit rather than silent.

### 11.3 Why Yjs
The Yjs v1 update encoding is shared by **yrs** (Rust: CLI, FUSE, MCP), **Yjs** (TypeScript SDK, Brain Studio) and **pycrdt** (Python library, Hermes, Pydantic-AI). Every client produces and merges byte-identical updates, and the browser editor in §16 can join a document as one more peer with no translation layer.

### 11.4 Storage & Materialisation
- **The update log is the source of truth**; `content` is a projection. Clients append to `file_crdt_update` (§1.1), permission-checked like a write to the file.
- **Materialisation is deterministic**, so any client can do it: load the latest `file_crdt_snapshot`, apply updates with `seq > upto`, render text, and write `content` with `if_generation`. Two clients materialising concurrently produce identical text; one compare-and-swap wins and the other's is a no-op. No coordinator is needed.
- `content` stays materialised because FULLTEXT, `hash`, `generation`, embeddings, history and every non-CRDT reader depend on it. A reader never needs to understand Yjs.
- **Compaction** runs as the `compactor` principal: fold the log into a new snapshot past a size or age threshold, then delete the folded updates.
- **Where merges run.** v2 materialises in clients. If SurrealDB gains in-database extension functions that can run yrs, materialisation moves into an `EVENT` on `file_crdt_update` and `content` becomes impossible for clients to get out of step with.

### 11.5 Semantics & Granularity
- A CRDT guarantees convergence, not meaning. Two agents rewriting the same sentence converge to an interleaving of both. For code, **line granularity** (each line an atomic unit) makes overlapping edits visible instead of producing well-formed nonsense.
- When two updates with the same `base_gen` touch overlapping ranges, the materialiser records an **overlap marker** in `file_version` (§7) and the next `cat` by either agent carries a one-line notice, so the agents can review the merged region.
- Leases (§1.1, `file_lock`) remain for **coordination** ("I am restructuring this file"), not for protecting bytes.

### 11.6 Default-On *(new)*
Because nothing about the agent's interface changes, CRDT mode can become the default for text files once the materialiser passes the §3.1 Convergence and Materialisation invariants. Lost updates then become impossible by construction, and `if_generation` (§1.3) remains for operations that are inherently non-mergeable (`mv`, `rm`, `chmod`, binary writes).

---

## 12. Global Inotify, Cross-Machine Watchers & Agent Mailboxes

Local `inotify` (Linux) and `FSEvents` (macOS) only fire within a single OS kernel. SurrealFS provides **Global Cross-Cluster Inotify**.

### 12.1 Cross-Machine Distributed Watchers (`surrealfs watch`)
```bash
# Agent A running in Tokyo:
$ surrealfs watch "/inbox/*.task" --exec "python run_task.py"

# Agent B running in London:
$ surrealfs cp bug_triage.task /mnt/brain/inbox/
```
The watch handler triggers instantly over the SurrealDB Live Query channel, regardless of network locality, cloud provider, or container boundaries. The watcher runs as a record user, so it only ever receives events for files that principal can read (§2.4).

### 12.2 Filesystem-Native Agent Actor Mailboxes
- Standardizes `/agents/{agent_id}/inbox/` as an atomic FIFO queue.
- Dropping a task file into an agent's inbox wakes the agent's event loop via push notification.
- Reading and deleting the file confirms message processing, providing a complete actor-model message bus built directly on POSIX filesystem semantics.
- **Design notes** *(new)*:
  - **Claim by rename.** With several consumers, "read then delete" double-processes. A consumer claims a message with `mv inbox/x.task inbox/.claimed/<consumer>/x.task`; the unique index guarantees exactly one `mv` wins, and the loser gets `sfs:exists`/`sfs:not_found`. Unclaimed-for-too-long messages are returned by a sweeper.
  - **Permissions.** An inbox folder is `0733`-style: anyone may drop, only the owner may list and read. Under server-side enforcement that is ordinary mode bits.
  - **Ordering.** FIFO by `created_at` with the record id as tie-break.
  - History (§7) gives every message a delivery and consumption record for free.

---

## 13. Zero-Privilege Runtimes (WASI Driver & `LD_PRELOAD` POSIX Shim)

FUSE requires Linux host permissions (`--cap-add SYS_ADMIN`, `/dev/fuse`), which are blocked in serverless environments (AWS Lambda, Google Cloud Run, unprivileged Kubernetes pods, WebContainers, browser runtimes).

### 13.1 User-Space `LD_PRELOAD` POSIX Interceptor (`libsurrealfs_shim.so`)
- A lightweight shared library written in Rust that intercepts standard libc calls (`open`, `read`, `write`, `stat`, `opendir`, `unlink`).
- When a path starts with `/surrealfs/...`, the shim redirects calls over in-process memory or WebSocket to SurrealDB.
- **Result**: Works in **100% unprivileged Docker containers** without FUSE, root access, or kernel modules.
- **Known limits to design around** *(new)*: statically linked binaries (Go, some Rust) and anything issuing raw syscalls bypass libc; the modern entry points (`openat`, `openat2`, `statx`, `newfstatat`, `getdents64`, `renameat2`) must be intercepted, not only the classic names; `io_uring` bypasses libc entirely; musl and glibc need separate builds; `vfork`/`exec` must re-inject the environment. A compatibility matrix (Python, Node, git, ripgrep, coreutils) is the acceptance test. Where available, a user-namespace FUSE mount (`fusermount3` without `SYS_ADMIN`) or `ptrace`/seccomp-notify interception covers what the shim cannot.

### 13.2 WASI / WebAssembly Virtual Filesystem
- Compiles the core SurrealFS engine to `wasm32-wasip2`.
- Agents running inside Wasm runtimes (Wasmtime, Extism) or browser WebContainers can mount SurrealFS directly into their virtual filesystem.
- Because the logic lives in `fn::sfs_*` (§1.2), the Wasm component is a thin WASI filesystem adapter over the Rust client, not a second implementation.

### 13.3 Edge Offline-First Local WAL Sync
- For nomadic agents or edge devices with intermittent connectivity, the Rust client maintains a local Write-Ahead Log (WAL) backed by SQLite/RocksDB.
- Mutations apply locally and sync differentials to the central SurrealDB cluster upon reconnection with automatic conflict resolution.
- **Design note** *(new)*: conflict resolution reuses the machinery above: CRDT files replay their queued Yjs updates (which merge by construction), non-CRDT writes replay with their recorded `if_generation` and surface `sfs:conflict` as a local conflict file. Permission failures on replay are reported, never dropped, since the server may have revoked access while offline.

---

## 14. Multi-Language SDKs

### 14.1 Pure Python Library (`surrealfs`)
- 100% pure Python wheel (`py3-none-any.whl`) with zero compilation requirements.
- Uses `surrealdb-py` directly over WebSockets.
- **Key Enhancements**:
  - Atomic `fs.append_text(path, suffix)` and `fs.head(path, n=10)`.
  - `fs.mkdir(path, exist_ok=True)` matching standard `pathlib.Path`.
  - Reconnect resilience in `embed.py` daemon via `_Reconnecting` wrapper.
  - *(new)* `fs.read_range(path, start, end)`, `fs.grep(pattern, glob=...)`, `fs.history(path)`, `fs.diff(path, a, b)`, `fs.restore(path, version)`, `fs.backlinks(path)`.
  - *(new)* `if_generation=` on every mutation; `SurrealFs` methods become thin wrappers over `fn::sfs_*`, and the Python permission checks are removed or kept only for error text (§2.5).
  - *(new)* CRDT mode via `pycrdt` (§11.3); optional extra, so the base wheel stays pure Python.
  - Python context manager for swarm leases and branch sandboxes:
    ```python
    async with fs.lease("/projects/acme", ttl=60, reason="Migrating auth"):
        await fs.write_text("/projects/acme/status.md", "In progress")

    async with fs.sandbox(branch="agent-experiment") as sandbox_fs:
        await sandbox_fs.write_text("/src/model.py", new_code)
        passed = await run_tests()
        if passed:
            await sandbox_fs.merge(strategy="3way")
    ```

### 14.2 TypeScript / JavaScript SDK (`@surrealdb/fs`)
- Native TypeScript package using `surrealdb.js` for Node.js, Bun, Deno, and modern browser runtimes.
- Implements single-round-trip path resolution (`_resolve`) and SurrealQL query mappings. In v2 these are calls to `fn::sfs_resolve` and friends (§1.2), so the SDK contains no filesystem logic of its own.
- **Surface Adapters**:
  - Vercel AI SDK and LangChain tool definitions (Zod schemas), generated from the same `tools/docs/*.md` prompt text the Python surfaces use.
  - Direct WebSocket connection mode for `surrealfs-browser` UI, eliminating the Starlette middleman proxy. This is safe only under §2: the browser authenticates as the user's record principal, since any credential shipped to a browser is visible to that user.
- A cross-language conformance suite (the §2.5 permission tests plus golden tool outputs) runs against Python, TypeScript and Rust in CI.

### 14.3 Rust Crate (`surrealfs-core`)
- High-performance asynchronous Rust client library shared between the CLI, FUSE daemon, and WASI shims.
- Same contract as the other SDKs: a typed wrapper over `fn::sfs_*` plus the client-side pieces that cannot live in the database (FUSE caching, base-version tracking, Yjs diffing via yrs, the offline WAL).

---

## 15. Next-Gen SurrealFS MCP Server (`surrealfs mcp`)

Implemented natively in the Rust CLI for sub-5ms cold starts, while keeping full backward compatibility with the Python implementation. Both implementations read `~/.config/surrealfs/env`, refuse to start without a `SURREALDB_URL`, write nothing but JSON-RPC to stdout, and keep `--selftest` (now also reporting the auth level and failing on a system credential).

1. **MCP Resources & Resource Templates**:
   - Exposes `surrealfs://{path}` as first-class read-only resources.
   - Claude Desktop, Cursor, and Zed can natively attach SurrealFS files as context pills without consuming agent tool-call turns.
   - *(new)* `surrealfs://{path}@{version}` for historical versions.
2. **Context-Optimized Tools**:
   - `head`: Read first $N$ lines.
   - `read_range`: Read lines $M$ through $N$ with optional line numbering.
   - `append_file`: Atomic log/journal writes.
   - `tree`: Hierarchical ASCII directory visualization.
   - *(new)* `grep`: exact or regex match with `path:line:text` output, a `glob` filter and a result cap. Today's `search` is ranked BM25 over stemmed tokens; agents also need the literal, exhaustive answer ("every file that mentions `OKTA-4412`"). Server-side via `fn::sfs_grep` using `string::matches` over rows pre-filtered by the FULLTEXT index when the pattern has a literal token, and a permission-filtered scan otherwise.
3. **Swarm & Knowledge Tools**:
   - `acquire_lease(path, ttl_seconds, reason)` & `release_lease(path)`.
   - `fork_workspace(src_branch, dst_branch)` & `merge_workspace(src_branch, dst_branch)`.
   - `search_graph(path, relation, depth)`: Query semantic file dependencies.
   - *(new)* `backlinks(path)` (§8.3).
4. **History Tools** *(new)*: `history`, `diff`, `restore`, `undelete` (§7.4).
5. **Understanding, code & data tools** *(new in v3)*:
   - `pack(question, budget, scope)`: the best material that fits a token budget (§20.6).
   - `digest(path)`: the folder's rolled-up summary (§20.4).
   - `outline(path)`: sections and symbols with line ranges and token counts (§20.5).
   - `symbols(path)`, `definition(name)`, `references(name)`, `callers(fn)`, `implementations(trait)` (§21.3).
   - `entities(query)` and `mentions(entity)` (§20.7).
   - `query_table(path, where, limit)` for tabular files (§21.6).
   - `du(path)` and `quota()` (§19.6).
   - `share(path, expires, mode)` and `revoke_share(id)` (§24.2).
   - Write results carry near-duplicate warnings (§20.8); `cat` of an oversized file returns its outline (§20.5).
6. **Tool contract changes** *(new)*:
   - Reads return `generation`; `edit` and `write_file` accept `if_generation`, filled in automatically from the session's last read.
   - Errors carry the `sfs:*` codes, and each tool's `docs/*.md` explains what to do on each (re-read on `sfs:conflict`; "not found or not permitted" on `sfs:not_found`).
   - Adding tools follows the existing convention: an entry in `surrealfs/tools/__init__.py`, an args model, a handler, a `docs/<kebab-name>.md`, and `provides_tools` in `integrations/hermes/plugin.yaml`; `tests/test_tools.py` keeps every surface in sync. The Hermes `surrealfs_` prefix applies to every new name.

---

## 16. Spatial "Brain Studio" & Multi-Agent Canvas (Browser UI 2.0)

Transforms the existing `surrealfs-browser` web UI into a collaborative command center for human-agent teams.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  🧠 SurrealFS Brain Studio                      [ Branch: main ▾ ] [ Live ] │
├─────────────────────────┬───────────────────────────────────────────────────┤
│ Filesystem Tree         │ 2D/3D Multi-Agent Knowledge Canvas                │
│ 📁 docs/                │                                                   │
│   📄 arch.md            │      [docs/arch.md] ──implements──> [src/auth.rs] │
│ 📁 src/                 │            │                              ▲       │
│   📄 auth.rs            │            │ references                   │       │
│   📄 db.rs              │            ▼                              │       │
│ 📁 agents/              │       [rfcs/0042.md] ─────────────────────┘       │
│   🤖 triage-bot (idle)  │                                                   │
│   🤖 refactor-2 (active)│  🤖 refactor-2 editing /src/auth.rs (lines 40-85) │
│                         │  👤 martin viewing /docs/arch.md                  │
├─────────────────────────┴───────────────────────────────────────────────────┤
│ Timeline Playback: [⏮ 24h ago] ──●────────────────────────────── [Now (Live)]│
└─────────────────────────────────────────────────────────────────────────────┘
```

1. **Live Agent Presence**: Colored avatars and focus indicators showing where agents are reading, writing, or holding leases in real time.
2. **Interactive 2D/3D Graph Canvas**: Visualizes file relations (`implements`, `references`, `supersedes`, and derived `links_to`) with clickable nodes and dependency paths.
3. **Timeline Playback Scrubber**: Scrub back in time across the last 24 hours to watch the file tree grow, see refactor waves propagate, and identify when regressions were introduced. Backed by `file_version` (§7), filtered by each version's permission snapshot.
4. **Live Working Buffer Inspector**: Inspect uncommitted agent buffers before they flush to disk.
5. **Collaborative editor** *(new)*: the editor pane joins CRDT documents as a Yjs peer (§11.3), so a person and several agents can edit one file live.
6. **History & blame panel** *(new)*: per-file versions, diffs, agent/model attribution and one-click restore.
7. **Identity** *(new)*: the browser signs in as the viewer's record principal (today it defaults to root with a `--user` flag); an admin view is an explicit, separately authenticated mode.

---

## 17. macOS Menubar Application (`SurrealFS Menu`)

A lightweight, native SwiftUI application for the macOS status bar.

```
┌──────────────────────────────────────────────┐
│  🟢 SurrealFS: Connected (production-brain)  │
├──────────────────────────────────────────────┤
│  Mount Status: Mounted at ~/mnt/surrealfs    │
│  Branch: main (Clean)                        │
│  [ Open in Finder ]    [ Open in Terminal ]  │
├──────────────────────────────────────────────┤
│  Profiles                                    │
│  ✓ Production Cloud (wss://cloud.surreal.io) │
│    Local Dev (ws://localhost:8000)           │
│    + Add New Connection...                   │
├──────────────────────────────────────────────┤
│  Live Agent Activity                         │
│  🤖 hermes wrote /brain/acme/risks/okta.md   │
│  🤖 triage-bot appended /incidents/log.md    │
│  👤 martin locked /projects/auth/ (32s left) │
├──────────────────────────────────────────────┤
│  [ Quick Search (⌘⇧S) ]                      │
│  [ Preferences... ]              [ Unmount ] │
└──────────────────────────────────────────────┘
```

1. **One-Click FUSE Mount**: Embeds the compiled `surrealfs` Rust binary and manages the mount lifecycle with automatic unmount on system sleep or app exit.
2. **Live Activity HUD**: Subscribes to the database Live Query stream to show streaming visual alerts as agents read and write files. Runs as the logged-in principal, so it shows only activity on files that principal can read (§2.4); each entry links to the §7 version it produced, with an "undo" action.
3. **Global Spotlight Search (`Cmd+Shift+S`)**: Quick floating search bar to fuzzy- and vector-search the entire agent brain from anywhere in macOS, with syntax-highlighted previews.
4. **Credentials** *(new)*: profiles store record-auth credentials in the macOS Keychain; the app never stores a system credential.

---

## 18. Docker & Cloud Infrastructure Utilities

1. **Official Multi-Arch Images (`surrealdb/surrealfs:latest`)**:
   - Linux `amd64` and `arm64` container images containing the Rust CLI, FUSE mounter, and `libsurrealfs_shim.so`.
   - Pre-configured with `libfuse3` support.
2. **Docker Compose Volume Sidecar Pattern** (v2 mounts as a record user; the schema is applied once by an init job holding the system credential):
   ```yaml
   services:
     surrealfs-init:
       image: surrealdb/surrealfs:latest
       environment:
         - SURREALDB_URL=ws://db:8000/rpc
         - SURREALDB_USER=root          # admin credential, used only here
         - SURREALDB_PASS_FILE=/run/secrets/db_root
       command: ["surrealfs", "init", "--user", "agent-worker", "--password-file", "/run/secrets/agent_pass"]
       secrets: [db_root, agent_pass]

     surrealfs-mount:
       image: surrealdb/surrealfs:latest
       cap_add:
         - SYS_ADMIN
       devices:
         - /dev/fuse
       environment:
         - SURREALDB_URL=ws://db:8000/rpc
         - SURREALDB_AUTH_LEVEL=record
         - SURREALDB_USER=agent-worker
         - SURREALDB_PASS_FILE=/run/secrets/agent_pass
       secrets: [agent_pass]
       volumes:
         - shared-brain:/mnt/brain:rshared
       command: ["surrealfs", "mount", "--allow-other", "/mnt/brain"]
       depends_on:
         - surrealfs-init

     agent-worker:
       image: my-agent:latest
       volumes:
         - shared-brain:/mnt/brain
       depends_on:
         - surrealfs-mount
   ```
   With `--allow-other`, every process that can see the volume acts as `agent-worker`; one mount per principal is the multi-tenant pattern.
3. **Kubernetes CSI (Container Storage Interface) Driver**:
   - Provides a native Kubernetes storage class (`storageClassName: surrealfs`), allowing pods across any node in the cluster to mount shared SurrealFS volumes natively.
   - *(new)* Each PersistentVolumeClaim binds to a record principal via a Kubernetes Secret, so two pods on one node with different claims get different permissions.

---

## 19. Large Files & Content-Addressed Storage *(new)*

§1.1's `file_chunk` splits large files into fixed 512 KB segments. v3 turns that into a content-addressed chunk store, so large files, history (§7) and branches (§6) cost only the bytes that actually changed.

### 19.1 Content-Defined Chunking
- Chunk boundaries come from a rolling hash over the content (**FastCDC**, min 256 KB / average 1 MB / max 4 MB), not from fixed offsets. Inserting a byte near the start of a file changes one or two chunks instead of shifting every chunk after it.
- Small text files (below 256 KB) stay inline in `file.content`/`file.file` as today, so the common agent case is unchanged and FULLTEXT indexing keeps working on `content`.
- Chunking happens in the client (Rust crate, TS SDK, Python library) before upload, which is also where compression (§19.5) and hashing happen. The server never needs to re-chunk.

### 19.2 Content Addressing & Deduplication
A chunk's id is the hash of its uncompressed bytes (BLAKE3), and a file is a manifest of chunk ids:
```surrealql
-- One row per distinct chunk. The id *is* the hash: blob:⟨b3:9f2c…⟩.
DEFINE TABLE OVERWRITE blob SCHEMAFULL
    -- Never readable directly: bytes are reached only through a manifest the caller can read.
    PERMISSIONS FOR select, create, update, delete NONE;
DEFINE FIELD size       ON blob TYPE int;
DEFINE FIELD stored     ON blob TYPE int;            -- bytes after compression
DEFINE FIELD codec      ON blob TYPE string;         -- 'none' | 'zstd'
DEFINE FIELD tier       ON blob TYPE string;         -- 'inline' | 'bucket' | 'object'
DEFINE FIELD bytes      ON blob TYPE option<bytes>;  -- inline tier only
DEFINE FIELD location   ON blob TYPE option<string>; -- bucket path or object key
DEFINE FIELD refs       ON blob TYPE int DEFAULT 0;  -- maintained by manifest events
DEFINE FIELD scope      ON blob TYPE string;         -- dedup scope, see below

-- v1's file_chunk becomes the manifest: which blob holds which byte range.
DEFINE FIELD offset     ON file_chunk TYPE int;
DEFINE FIELD length     ON file_chunk TYPE int;
DEFINE FIELD blob       ON file_chunk TYPE record<blob>;
DEFINE INDEX idx_chunk_offset ON file_chunk FIELDS file_id, offset;
```
- **History and branches become cheap.** A new version whose manifest shares 99% of its chunks with the previous one stores 1% new bytes. A branch fork (§6) copies manifests, not bytes.
- **Whole-file hash.** `file.hash` for a chunked file is the hash over its ordered chunk ids (a Merkle root), so `generation` (§1.1) and embedding staleness work for large and binary files too. This closes the "`hash` covers only `content`" gap noted in §1.1.
- **Dedup is a side channel, so it is scoped.** Cross-user deduplication lets anyone test whether some exact content exists anywhere (upload it and watch whether the server already had it). `blob.scope` is the owner (or the tenant, by configuration), and dedup happens only within a scope. The upload protocol never tells a client that a chunk already exists outside its own scope; a client can skip uploading only chunks its own scope already holds.
- **Garbage collection.** `refs` is kept by `CREATE`/`DELETE` events on `file_chunk` and on `file_version` manifests (A.8: event writes bypass the caller's permissions, so a user can release a reference without being able to touch `blob`). A collector principal deletes zero-ref blobs after a grace period, which also protects in-flight uploads.

### 19.3 Tiered Blob Storage
| Tier | Where | When |
|---|---|---|
| `inline` | `blob.bytes` in SurrealDB | chunks of files below a size threshold, and every chunk in small deployments |
| `bucket` | a SurrealDB `DEFINE BUCKET` | the default for large files where the deployment's bucket backend is suitable |
| `object` | S3, R2, GCS or Azure Blob, with the key in `blob.location` | very large files, cold history, and deployments that already run an object store |

- `DEFINE BUCKET` works on 3.2.4 behind `--allow-experimental files` (A.13): `f'b:/path'.put(...)`, `.get()` and `.head()` behave as expected on the `memory` backend. Which persistent backends are production-ready, and whether bucket permissions can be derived from a `file` row, needs verifying before it becomes the default tier.
- For the `object` tier, the database stores only keys. Reads and writes go through **presigned URLs** issued by a small blob gateway that authenticates the end user, asks SurrealDB (as that user) for the manifest, and signs URLs only for chunks in a manifest the user could read. The object store therefore needs no permission rules of its own, and a URL leaks one chunk for a few minutes at most.
- A background mover migrates blobs between tiers by age and access frequency, with no change visible to readers.

### 19.4 Uploads, Range Reads & Sparse Files
- **Resumable, parallel uploads.** `fn::sfs_upload_begin(path, size, chunk_ids)` returns which chunks are missing (within the caller's dedup scope); clients upload missing chunks in parallel and in any order; `fn::sfs_upload_commit(upload_id, if_generation)` swaps the manifest in one transaction. Readers see the old file until commit and the new one after, never a mix. An `upload` table records progress, so a dropped connection resumes where it stopped; abandoned uploads expire.
- **Range reads.** `fn::sfs_read_bytes(path, offset, length)` finds the covering chunks through `idx_chunk_offset` and returns only those, which is what FUSE `read()`, WebDAV/S3 `Range:` requests and video seeking need.
- **Sparse files.** A hole is a range with no manifest entry, and reads return zeros for it. `stat` reports both the logical size and the allocated size, so a 10 GB file with holes stores only its data.

### 19.5 Transparent Compression
- Each chunk is compressed with **zstd** before upload, unless its content is already compressed (detected by magic bytes and entropy: images, video, audio, archives, most office formats). The hash is taken over the *uncompressed* bytes, so dedup is independent of the codec.
- Compression is invisible: every read path decompresses, and `stat` reports the logical size.

### 19.6 Quotas & Usage
- A `usage` table keeps running counters per owner and per folder (files, logical bytes, stored bytes after dedup and compression), updated by events on `file` and `file_chunk`. `du` is then a single row read instead of a tree walk.
- A quota is a row per owner or folder. `FOR create` and the upload commit check `fn::sfs_quota_ok`, so a quota is enforced by the database like everything else in §2. Deduplicated bytes count once per scope.
- `.surrealfs/stats` and `surrealfs du` expose the counters; Brain Studio and the menubar app show them.

---

## 20. The Automatic Understanding Pipeline *(new)*

Everything in this section happens without anybody asking for it. A write enqueues work; workers running as scoped principals (§2.3) do it; results land as derived rows that inherit the source file's permissions.

### 20.1 The Job Queue
```surrealql
DEFINE TABLE OVERWRITE sfs_job SCHEMAFULL PERMISSIONS FOR select, create, update, delete WHERE $access IN ['indexer', 'projector'];
DEFINE FIELD file_id    ON sfs_job TYPE record<file>;
DEFINE FIELD kind       ON sfs_job TYPE string;  -- detect | chunk | embed | digest | extract | dedup | symbols
DEFINE FIELD source_hash ON sfs_job TYPE string;
DEFINE FIELD leased_by  ON sfs_job TYPE option<string>;
DEFINE FIELD lease_until ON sfs_job TYPE option<datetime>;
DEFINE FIELD attempts   ON sfs_job TYPE int DEFAULT 0;
DEFINE INDEX idx_job_unique ON sfs_job FIELDS file_id, kind, source_hash UNIQUE;
```
- An event on `file` creates jobs when `hash` changes. The unique index makes enqueueing idempotent, so a burst of writes to one file queues one job per kind per content version, and a job for a superseded hash is skipped.
- Workers lease jobs with an expiry (the `file_lock` pattern), retry with backoff, and record failures on the file (`meta.pipeline_errors`) rather than dropping them.
- The pipeline stages run in dependency order: detect, then chunk, then embed, symbols and extract in parallel, then digest and dedup.

### 20.2 Type & Language Detection
- Detect from **content**, not the extension: magic bytes first (PDF, images, archives, office formats, SQLite, Parquet), then text heuristics (shebangs, modelines, structure), then a code classifier for ambiguous text, with the extension as a tie-break only.
- For natural-language text, detect the **language** (fastText-style `lid` model) and the **encoding** (normalising to UTF-8 on the way in, keeping the original bytes when they were not UTF-8).
- Store the results on the row: `content_type`, `language` (for example `de`, `ja`, `python`, `rust`), `encoding`, `is_generated` (§21.5). Every later stage keys off these fields.

### 20.3 Automatic, Type-Aware Chunking
The chunker picks a strategy from `content_type` and `language`, and fills `file_section` (§1.1) with line or byte ranges and, where it applies, a heading path:

| Content | Strategy |
|---|---|
| Source code | tree-sitter: functions, methods, classes, modules; oversized units split at nested boundaries; a file-level unit carries imports and top-level docs |
| Markdown, reStructuredText, AsciiDoc | heading hierarchy, with the heading path (`Architecture > Auth > Tokens`) stored for display and search |
| HTML | readability extraction, then headings |
| PDF | pages, then the extracted structure (headings, tables) from the §9 projection |
| Word, PowerPoint, Google-export formats | sections and slides, through the §9 projection |
| Spreadsheets | one unit per sheet plus row windows with the header row repeated in each |
| CSV, TSV, Parquet | schema and statistics as one unit, then row windows with headers |
| Jupyter notebooks | cells, with outputs excluded from embeddings and kept for display |
| Transcripts | speaker turns, merged up to a target size |
| JSON, YAML, TOML | top-level keys, recursively for large documents |
| Logs | time windows |
| Anything else | line windows with overlap |

- Targets are measured in tokens (§20.5), not characters, so chunks fit embedding models and context windows predictably.
- Re-chunking is incremental: unchanged units keep their `file_section` rows and embeddings (matched by content hash), so editing one function re-embeds one function.

### 20.4 Rolled-Up Folder Digests
- Each folder has a generated **digest**: what the folder is for, its notable files, recent changes and open questions, built from its files' summaries and its subfolders' digests (the RAPTOR pattern). An agent landing in `/projects/acme` reads one digest instead of fifty files.
- Staleness is a Merkle hash over the children's hashes, so a change deep in the tree marks every ancestor digest stale. Rebuilds are debounced and run bottom-up, so a burst of writes rebuilds each ancestor once.
- **A digest must not leak what its readers cannot see.** A folder's readers may not be able to read every file in it. A digest therefore summarises only children readable by every principal who can read the folder (in a shared folder, the world-readable children; in a folder private to one owner, all of them), and states "N private files not summarised". Private material appears only in digests of folders that are themselves private to the same owner.
- Digests are exposed as `.surrealfs/digest/<path>`, as the `digest(path)` tool (§15), and at the top of `ls` output when the folder has one.

### 20.5 Token-Aware Reads
- The pipeline records token counts per file and per section for each tokenizer family in use (stored as `meta.tokens`).
- `stat` and `ls` report them. `cat` on a file larger than a configurable budget returns its **outline** (sections, symbols, sizes) and tells the model to use `read_range`, instead of truncating silently or flooding the context.

### 20.6 Context Packing
- `pack(question, budget, scope)` returns the best material that fits a token budget: hybrid search over sections, expanded with backlinks and derived links (§8.3), symbols (§21.3) and the relevant folder digests, deduplicated, ordered by relevance, and cut to fit.
- Output is a list of `path:line_start-line_end` blocks with their text, so every claim stays traceable to a file, as the `brain` skill requires.
- Implemented as `fn::sfs_pack` where possible, so every client gets the same packing; the permission filter applies to every candidate, as in search.

### 20.7 Auto-Tagging & Entity Extraction
- The pipeline extracts tags into `file.meta.tags` and entities (people, companies, systems, projects, tickets) into an `entity` table, linked by a `mentions` relation.
- An entity is visible only through files the caller can read: `entity` rows carry no content of their own, and `mentions` edges use the both-ends rule from §1.1. Otherwise a name mentioned only in a private file would leak through the shared entity list.
- Entities make the graph (§8) useful on day one ("everything that mentions Okta") and feed `pack` and Brain Studio's canvas.

### 20.8 Near-Duplicate Detection
- On every write, compare the file's MinHash/SimHash signature (cheap, lexical) and its embedding (semantic) against existing files.
- Close matches come back as a **warning** in the write result ("this looks 94% like `/notes/okta-risk.md`"), never a rejection. This matters because the bundled skill tells agents to search before writing, and a duplicate that slips past search pollutes the brain.
- The warning names only files the writer can read.

---

## 21. Languages & Code Intelligence *(new)*

### 21.1 Per-Language Full-Text Analyzers
Today `idx_file_content` stems every file with `snowball(english)`, so German, French or Japanese text is tokenised as if it were English. v3 routes each row to an analyzer that matches its detected language (§20.2):
```surrealql
DEFINE ANALYZER OVERWRITE sfs_de   TOKENIZERS blank, class FILTERS lowercase, snowball(german);
DEFINE ANALYZER OVERWRITE sfs_cjk  TOKENIZERS blank FILTERS ngram(1,2);
DEFINE ANALYZER OVERWRITE sfs_code TOKENIZERS class, camel FILTERS lowercase;

-- One stored field per language family, populated only for rows in that language.
DEFINE FIELD OVERWRITE text_de ON file TYPE option<string>
    VALUE IF language = 'de' THEN content ELSE NONE END;
DEFINE INDEX OVERWRITE idx_text_de ON file FIELDS text_de FULLTEXT ANALYZER sfs_de BM25;
```
- Verified on 3.2.4 (A.14, A.15): the German analyzer stems "Häuser" to `haus`; the CJK analyzer emits uni- and bi-grams for 東京都; the code analyzer splits `getUserById` into `get user by id`; and a language field populated only for matching rows gives each index exactly its own rows.
- `fn::sfs_search_text` queries the index for the query's detected language plus the language-neutral `sfs_code` and `simple` indexes, and fuses them with RRF, so a mixed-language corpus is searched correctly without the caller choosing.
- Each row stores one extra copy of its text, in its own language's field. Avoiding that copy needs per-row analyzer selection upstream (§25).
- Code gets its own analyzer with no stemming: identifiers split on case and underscores, so `grep`-like lookups of `verify_token` and `verifyToken` both work in ranked search.

### 21.2 Multilingual Embeddings
- The default embedder is multilingual, so a question in English finds a design doc in German. The model is recorded in `indexer_version` as today, and a model change triggers re-embedding through the job queue.
- Code sections can use a code-specific embedding model, recorded per section, with the semantic arm querying each model's HNSW index and fusing results.

### 21.3 Symbol Index & Code Navigation Tools
```surrealql
DEFINE TABLE OVERWRITE symbol SCHEMAFULL
    PERMISSIONS
        FOR select WHERE fn::sfs_can_read(fn::sfs_gate(file_id.parent), file_id.owner, file_id.mode, fn::sfs_me())
        FOR create, update, delete WHERE $access = 'indexer';
DEFINE FIELD file_id    ON symbol TYPE record<file>;
DEFINE FIELD name       ON symbol TYPE string;
DEFINE FIELD qualified  ON symbol TYPE string;     -- auth.jwt.verify_token
DEFINE FIELD kind       ON symbol TYPE string;     -- function | method | class | struct | trait | const | ...
DEFINE FIELD language   ON symbol TYPE string;
DEFINE FIELD signature  ON symbol TYPE option<string>;
DEFINE FIELD doc        ON symbol TYPE option<string>;
DEFINE FIELD line_start ON symbol TYPE int;
DEFINE FIELD line_end   ON symbol TYPE int;
DEFINE INDEX idx_symbol_name ON symbol FIELDS name;
DEFINE INDEX idx_symbol_qualified ON symbol FIELDS qualified;
-- Relations: calls, references_symbol, imports, inherits (IN symbol|file OUT symbol|file)
```
- Built from tree-sitter tags queries for every supported grammar (§21.4), so it works with no build and no toolchain.
- Optional **language-server enrichment**: for a folder marked as a project, a worker materialises a checkout and runs the language's server to resolve cross-file references precisely. It is opt-in per project because it is heavy, and it writes to the same tables.
- Tools (§15): `symbols(path)` (outline), `definition(name)`, `references(name)`, `callers(fn)`, `implementations(trait)`. Agents navigate code by symbol far more effectively than by grep, and `imports` edges feed the derived links of §8.3.

### 21.4 Broad Grammar Coverage
- Bundled tree-sitter grammars: Python, TypeScript, JavaScript, TSX/JSX, Rust, Go, Java, Kotlin, Scala, C, C++, C#, Swift, Objective-C, Ruby, PHP, Elixir, Erlang, Haskell, OCaml, Lua, R, Julia, Dart, Zig, SQL, SurrealQL, shell (bash, zsh, fish), PowerShell, HTML, CSS/SCSS, Vue, Svelte, Markdown, JSON, YAML, TOML, XML, HCL/Terraform, Dockerfile, Makefile, Nix, Protobuf, GraphQL.
- Grammars load as WebAssembly modules, so the Rust, TypeScript and Python workers share one set and new languages ship without a release of every client.
- Anything without a grammar falls back to line-window chunking (§20.3) and still gets full-text and semantic search.

### 21.5 Git Awareness
- **Import** a repository (`surrealfs import git <url> /projects/x`), respecting `.gitignore` and `.gitattributes`.
- **History mapping**: each commit becomes a set of `file_version` rows (§7) with the commit sha, author, date and message, so `history`, `blame` and the timeline work on imported code with its real history.
- **`git-remote-surrealfs`**: a git remote helper so `git clone surrealfs://brain/projects/x`, `git fetch` and `git push surrealfs main` work, with pushes applied as ordinary writes (and therefore permission-checked, versioned and indexed).
- **Generated files**: lockfiles, build output, vendored code and minified bundles are classified (linguist-style rules plus `.gitattributes` `linguist-generated`), stay searchable by `grep`, and are excluded from embeddings, digests and entity extraction.

### 21.6 Notebooks & Structured Data
- `.ipynb`: chunked by cell (§20.3); outputs are stripped from embeddings and kept for display; `read_range` addresses cells.
- CSV, TSV, Parquet, spreadsheets and SQLite: the §9 projector produces schema, statistics and a sample. Tabular files below a row limit are also loaded into a `file_row` table (file id, sheet, row index, data object) with permissions derived from the file, so an agent can ask `query_table(path, where, limit)` instead of reading the whole file. Larger files keep projector summaries and range reads.

---

## 22. Additional Interfaces *(new)*

**One rule for every surface below:** each one authenticates the end user and connects to SurrealDB as that user's own principal (§2). No gateway holds a shared root credential, since that would turn the gateway into a permission bypass. External credentials (SSH keys, S3 access keys, WebDAV passwords, Slack user ids) map to principals through a `credential` table and a dedicated access method per kind, and revoking the credential row revokes the surface.

### 22.1 WebDAV Server
- `surrealfs serve webdav` (in the Rust binary). Finder, Windows Explorer, iOS and iPadOS Files, GNOME and KDE mount WebDAV natively, with no kernel extension, no macFUSE and no admin rights. This is the fastest route to "it's just a drive" for people.
- WebDAV `LOCK`/`UNLOCK` map to `file_lock` (§1.1); ETags are `hash`; `If-Match` maps to `if_generation` (§1.3); `Range` maps to §19.4.
- Client quirks to absorb: macOS writes `._*` AppleDouble files and `.DS_Store` (store their data as xattrs or in a hidden per-folder row, never as visible files); the Windows client caps file size by default and needs HTTPS for basic auth. Both are documented in the setup guide.

### 22.2 NFSv3 Loopback Server
- `surrealfs mount --nfs <dir>` runs a user-space NFSv3 server on localhost and mounts it with the OS's built-in NFS client, which gives a real Unix mount on macOS and Linux without FUSE or kernel extensions. Several user-space filesystems already take this route on macOS.
- NFS is stateless, so file handles encode the record id plus `generation`; a handle to a replaced file returns `ESTALE`, which clients handle natively.

### 22.3 S3-Compatible API
- `surrealfs serve s3` exposes the tree through the S3 API: a bucket is a top-level folder (or a mount root), and keys are paths. Every data tool, backup system and SDK that speaks S3 can then read and write the brain.
- SigV4 access keys map to principals (`credential` of kind `s3`), so permissions are still enforced by the database. Multipart uploads map to resumable uploads (§19.4); `ETag` is `hash`; `ListObjectsV2` is `ls`/`glob` with the permission filter.

### 22.4 VS Code & JetBrains
- A **VS Code extension** implementing `FileSystemProvider` for a `surrealfs://` scheme: browse, open, edit, rename and delete; file watching over live queries; a `TextSearchProvider` backed by `fn::sfs_search`; live collaborative editing through Yjs (§11); history and diff in the timeline view. It is built on the TypeScript SDK, so it also works in `vscode.dev` and GitHub Codespaces.
- A **JetBrains** plugin through the IntelliJ virtual file system API, with the same features.

### 22.5 Obsidian Plugin
- Obsidian's model (a folder of markdown, wikilinks, backlinks, frontmatter) lines up almost exactly with the brain use case, and §8.3 and §8.4 already provide backlinks and frontmatter natively.
- The plugin keeps a vault and a SurrealFS folder in live two-way sync (in the manner of Obsidian Sync), merging concurrent edits through Yjs (§11) so agents and a person can work on the same notes at once. It adds commands for semantic search, `pack`, history and restore inside Obsidian.

### 22.6 Terminal UI & Launchers
- `surrealfs ui`: a terminal UI (ratatui) for servers and SSH sessions, with a tree, preview, search, history, diff, leases and live activity.
- **Raycast** and **Alfred** extensions: search the brain, open a file, and quick-capture a note into an inbox folder.

### 22.7 Chat Bots (Slack, Teams)
- Ask "what do we know about Okta?" in a channel and get an answer built with `pack` (§20.6), citing files by path.
- Dropping a file in a channel (or reacting to a message with a chosen emoji) files it into a configured folder, and the pipeline indexes it.
- **The bot must act as the asking user.** A bot with one service credential would answer anyone in the channel from files only some of them may read. Each Slack or Teams user maps to a principal (`credential` of kind `slack`), unmapped users get nothing, and answers in a shared channel are limited to what every channel member can read.

### 22.8 SFTP Server
- `surrealfs serve sftp`, with SSH keys mapped to principals, for legacy tools, scripts and partners who need a drop box. Large uploads use the chunked path (§19).

### 22.9 Mobile App / PWA
- Brain Studio (§16) as an installable **PWA** first: search, reading, history and digests, with an offline read cache.
- **Voice notes** recorded in the app land as audio files, and the §9 projector turns them into transcripts, so they are searchable within seconds.
- A native app later if push notifications, share-sheet capture and background sync justify it.

---

## 23. Connectors, Webhooks & Export *(new)*

### 23.1 Connectors That Mirror External Sources as Files
- Google Drive, Notion, Confluence, SharePoint/OneDrive, Slack exports, email (IMAP/Gmail), GitHub/GitLab issues and pull requests, Linear and Jira each appear as a folder under `/sources/<connector>/...`, kept in sync incrementally through each service's change feed.
- Documents are stored in their most useful text form (Google Docs as markdown, Sheets as CSV plus sheet sections, issues as markdown with frontmatter), with the original link in `meta.source_url` and the source's id and version in `meta.source`.
- Mirrors are **read-only** by default; a connector may opt in to write-back later.
- **Source permissions carry over.** Each connector maps the source's ACLs onto owners, modes and groups (§24.1), per document, so a private Google Doc stays private to the same people in SurrealFS. Where an ACL cannot be represented, the connector falls back to the most restrictive mapping and records that in `meta`.
- Connectors run as a `connector` principal that may write only under `/sources/<its name>/`. Deletions in the source propagate, and appear in history (§7) like any delete.
- One interface for everything: agents use `ls`, `cat`, `search` and `pack` across all sources at once.

### 23.2 Webhooks & Outbound Events
- A `webhook` table (owner, path pattern, events, URL, secret). A dispatcher subscribes to the live-query stream **as the webhook's owner**, so a webhook only ever fires for files its owner can read.
- Payloads are signed with HMAC and delivered with retries, backoff and a dead-letter record visible to the owner.
- Delivery is never done with `http::` calls inside an `EVENT`: that would put a remote server's latency and failures inside every write transaction.

### 23.3 Import & Export
- `surrealfs export <path> [--as-of <time> | --version <tag>] --format tar|zip|git|dir` streams any folder at any point in time, so nothing is locked in. `git` export includes history as commits.
- `surrealfs import` is symmetrical (directory, archive, git repository, or another SurrealFS), preserving timestamps and, where present, owners and modes.

---

## 24. Trust, Sharing & Operations *(new)*

### 24.1 Groups
- Group bits are stored today but never consulted (a ponytail in `docs/permissions.md`). Teams need a folder outsiders cannot see.
- Schema: a `group` table, a `member_of` relation from `user` to `group` (admin-managed like users), and `file.group`. `fn::sfs_bits` consults the group digit when the caller is a member.
- The hard part is `gate`, which today encodes "closed by this owner". It has to become "reachable by this owner or members of this group" without losing the single-column design; the proposal is to encode the gate as a small set of `owner:<x>`/`group:<y>` tokens and check membership with a set intersection. The permission tests (§2.5) must cover nested folders closed by different groups.

### 24.2 Share Links
- Expiring, read-only (or append-only, for drop boxes) links to one file or folder, for people without an account.
- Implemented with SurrealDB bearer access (`DEFINE ACCESS share ON DATABASE TYPE BEARER FOR RECORD`), which issues revocable, expiring grants on 3.2.4 (A.16). The grant's subject is a `share` record naming the path and mode, and `fn::sfs_can_read` gains "or the caller is a share principal whose path is an ancestor of this row".
- Link access is logged in history (§7); revoking the grant ends it immediately.

### 24.3 Encryption
- **At rest**: rely on encrypted storage for SurrealDB and server-side encryption for the object tier (§19.3). Transparent, and no feature is lost.
- **End-to-end, per folder**, for material the server must not read: the client encrypts content (and optionally names) before upload, with a per-folder key wrapped for each member's public key.
- The trade-off is explicit and shown in the UI: the server cannot search, embed, chunk, digest, deduplicate or extract from an encrypted folder. The pipeline skips such folders, and search results say "N encrypted folders not searched".

### 24.4 Retention & Legal Hold
- Retention policies per folder: keep every version for N days, prune older versions to daily or weekly snapshots, or keep everything.
- A **legal hold** on a folder blocks deletion and version pruning under it, enforced in `FOR delete` and in the pruning job, and is itself recorded in history.

### 24.5 Backups & Point-in-Time Restore
- Because data is content-addressed chunks plus manifests plus versions, a backup is incremental by construction: ship new blobs and new rows since the last backup.
- `surrealfs backup` / `surrealfs restore --as-of <time>` restores a folder or the whole tree to any point covered by retention, into place or into a new folder.

### 24.6 Observability
- OpenTelemetry traces and metrics from every client and worker: request latency, pipeline lag (time from write to searchable), queue depth, embedding cost, cache hit rates, upload throughput.
- Server-side counters in `.surrealfs/stats` and per-principal usage for operators, and a health view in Brain Studio.

---

## 25. Upstream SurrealDB Engine Contributions

To optimize SurrealFS further, the following enhancements should be contributed to the upstream `surrealdb/` engine:

1. **Dynamic Parameters in HNSW Operators**:
   - Update parser and planner so `<|$k, $ef|>` accepts query parameters (e.g. `<|$k, 160|>`), eliminating hardcoded literal integer pools in `DEFINE FUNCTION`. Today a bound `k`/`ef` parses and returns correct rows but silently drops to a brute-force scan.
2. **Native Server-Side Snippet & Offset Functions**:
   - Fix `search::offsets()` and `search::highlight()` in the core search engine to return exact character offsets for stemmed terms, avoiding full-file wire transfer for snippet generation. (On 3.2.x `highlight` returns the content unchanged and `offsets` returns NONE.)
3. **Built-in String Search Helpers**:
   - Add `string::find($str, $sub)` and `string::index_of($str, $sub)` to SurrealQL standard library.
4. *(new)* **HNSW filtered search**: apply a `WHERE` predicate during graph traversal (or let the planner widen `ef` until `k` filtered results are found), removing the fixed permission pool of §2.3 and §10.3.
5. *(new)* **`$before`/`$after` in PERMISSIONS clauses**: today they are NONE in a table permission and field permissions see only the after state (Appendix A.1). Binding them would let `FOR update` distinguish a content write from a rename without an EVENT.
6. *(new)* **Optional loud denials**: a per-table or per-session option for a denied statement to raise instead of returning zero rows, so clients do not need wrapper functions to tell "denied" from "matched nothing".
7. *(new)* **A second match reference on one field**: `content @1,OR@ $q AND content @2@ $q` silently ignores reference 2; either support it or reject it at parse time.
8. *(new)* **Embedded engine computed fields**: the `surrealdb[embedded]` core returns `null` for COMPUTED fields on indexed reads, which blocks `mem://` for tests and for the WASI target (§13.2).
9. *(new)* **In-database extension functions** that can run a Yjs merge, so CRDT materialisation (§11.4) can move into an `EVENT`.
10. *(new in v3)* **Per-row analyzer selection** for a FULLTEXT index (an analyzer chosen by a field value such as `language`), removing the per-language copy of the text in §21.1.
11. *(new in v3)* **Production bucket backends and derived permissions**: stable persistent and S3-compatible `DEFINE BUCKET` backends, and bucket permissions that can reference a record (so a chunk's readability can follow its `file` row), which would let §19.3's bucket tier replace the presigned-URL gateway.
12. *(new in v3)* **Streaming byte ranges over RPC**, so range reads (§19.4) do not buffer whole chunks in the SDKs.
13. *(new in v3)* **BLAKE3 in `crypto::`**, so the server can verify content-addressed chunk ids (§19.2) instead of trusting the client's hash.

---

## 26. Execution Roadmap & Milestones

```mermaid
gantt
    title SurrealFS Engineering Roadmap (Next-Gen)
    dateFormat  YYYY-MM-DD
    section Phase 0: Server-Side Security
    Record auth default & surrealfs init      :p0_1, 2026-10-01, 6d
    Principals (indexer, projector, compactor):p0_2, after p0_1, 4d
    Rename EVENT & field-permission gaps      :p0_3, after p0_2, 4d
    Core ops as fn::sfs_* with THROW codes    :p0_4, after p0_1, 12d
    Record-auth test matrix & deletion test   :p0_5, after p0_4, 5d
    section Phase 1: Python Core & MCP Wins
    Atomic append_text & head               :p1_1, 2026-10-01, 5d
    mkdir(exist_ok=True) & Sniffing         :p1_2, after p1_1, 3d
    Embed daemon reconnection & .env        :p1_3, after p1_2, 4d
    MCP line-range & context tools          :p1_4, after p1_3, 4d
    grep, tree & MCP resources              :p1_5, after p1_4, 5d
    generation & if_generation              :p1_6, after p1_1, 5d
    History EVENT, provenance & undo tools  :p1_7, after p1_6, 8d
    Derived links, backlinks & frontmatter  :p1_8, after p1_7, 6d
    Property tests & retrieval evals        :p1_9, after p1_2, 8d
    section Phase 2: Rust CLI, FUSE & Procfs
    Rust CLI scaffolding (clap, client)     :p2_1, 2026-10-15, 7d
    FUSE filesystem core (fuser)            :p2_2, after p2_1, 12d
    xattr & .surrealfs/ synthetic procfs    :p2_3, after p2_2, 7d
    Live query kernel cache invalidation    :p2_4, after p2_3, 6d
    section Phase 3: Testing & Swarm Leases
    Deterministic Simulator (DST)           :p3_1, 2026-11-12, 14d
    Swarm advisory locking & leases         :p3_2, after p3_1, 7d
    TypeScript SDK (@surrealdb/fs)          :p3_3, after p3_2, 10d
    Cross-language conformance suite        :p3_4, after p3_3, 5d
    section Phase 4: Branching, Graph, AST & CRDT
    Zero-copy branching (surrealfs fork)    :p4_1, 2026-12-05, 12d
    Semantic graph (.graph/ & relations)    :p4_2, after p4_1, 8d
    AST Treesitter & section embeddings     :p4_3, after p4_2, 10d
    Global inotify & agent mailboxes        :p4_4, after p4_3, 7d
    Client-transparent CRDTs (Yjs)          :p4_5, 2026-12-05, 18d
    CRDT compaction & default-on for text   :p4_6, after p4_5, 7d
    section Phase 5: Runtimes & Canvas Studio
    Zero-privilege LD_PRELOAD shim & WASI   :p5_1, 2027-01-10, 14d
    Native SwiftUI macOS Menubar App        :p5_2, after p5_1, 14d
    Spatial Brain Studio 2.0 (Canvas)       :p5_3, after p5_2, 14d
    section Phase 6: Storage at Scale
    FastCDC chunking & blob manifests       :p6_1, 2026-11-02, 10d
    Scoped dedup, refcounts & GC            :p6_2, after p6_1, 6d
    Resumable uploads & range reads         :p6_3, after p6_1, 7d
    zstd, sparse files, usage & quotas      :p6_4, after p6_3, 6d
    Tiering: buckets, object store, gateway :p6_5, after p6_2, 10d
    section Phase 7: Understanding & Languages
    Job queue & type/language detection     :p7_1, 2026-11-09, 6d
    Type-aware chunking (all formats)       :p7_2, after p7_1, 12d
    Per-language analyzers & multilingual   :p7_3, after p7_1, 8d
    Token counts, outlines & pack           :p7_4, after p7_2, 8d
    Folder digests (permission-aware)       :p7_5, after p7_4, 7d
    Entities, tags & near-duplicates        :p7_6, after p7_4, 8d
    Symbol index & code tools               :p7_7, after p7_2, 10d
    Git import, history & remote helper     :p7_8, after p7_7, 8d
    Notebooks & tabular files               :p7_9, after p7_2, 6d
    section Phase 8: Interfaces
    WebDAV server                           :p8_1, 2026-12-01, 10d
    VS Code FileSystemProvider              :p8_2, after p3_3, 10d
    S3-compatible API                       :p8_3, after p8_1, 10d
    NFSv3 loopback mount                    :p8_4, after p8_1, 10d
    Obsidian sync plugin                    :p8_5, after p4_5, 10d
    TUI, Raycast & Alfred                   :p8_6, after p8_1, 8d
    Slack & Teams bots                      :p8_7, after p7_4, 8d
    SFTP server                             :p8_8, after p8_3, 5d
    JetBrains plugin                        :p8_9, after p8_2, 8d
    PWA & voice notes                       :p8_10, after p5_3, 10d
    section Phase 9: Connectors & Trust
    Groups (gate token sets)                :p9_1, 2026-12-14, 10d
    Share links (bearer access)             :p9_2, after p9_1, 5d
    Connectors: Drive, Notion, Slack, email :p9_3, after p9_1, 20d
    Webhooks & export/import                :p9_4, after p9_2, 8d
    Retention, legal hold & backups         :p9_5, after p9_4, 8d
    E2E encrypted folders                   :p9_6, after p9_5, 10d
    OpenTelemetry & operator views          :p9_7, after p9_4, 6d
```

**Dependencies** *(new)*:
- Phase 0 gates every new surface: the Rust CLI (p2_1), the TypeScript SDK (p3_3), the FUSE daemon's live queries (p2_4), watchers (p4_4) and the menubar HUD (p5_2) all assume record-auth principals and `fn::sfs_*`.
- `generation` (p1_6) precedes history (p1_7), leases (p3_2), FUSE base-version tracking (p2_2) and CRDTs (p4_5).
- History (p1_7) precedes `--as-of`, `.snapshots/`, branching via versions (p4_1) and the timeline scrubber (p5_3).
- Property tests (p1_9) start before the simulator (p3_1) and share its invariant definitions.
- Content-addressed chunks (p6_1, p6_2) precede large-file support in FUSE (p2_2), WebDAV/S3/SFTP (p8_1, p8_3, p8_8) and backups (p9_5), and make history (p1_7) and branching (p4_1) cheap for binary files.
- The job queue and detection (p7_1) precede every pipeline stage; type-aware chunking (p7_2) supersedes and absorbs the markdown/tree-sitter work in p4_3.
- `pack` (p7_4) precedes the chat bots (p8_7); the TypeScript SDK (p3_3) precedes the VS Code extension (p8_2); CRDTs (p4_5) precede Obsidian sync (p8_5).
- Groups (p9_1) precede connectors (p9_3), because source ACLs need groups to map onto.
- Every Phase 8 surface depends on Phase 0: each maps its external credential to a record principal.

---

## Appendix A: Verified Engine Behaviour

Tested on SurrealDB `3.2.4+20260803.93ab219`, as a record user signed in through `DEFINE ACCESS ... TYPE RECORD`, over HTTP `/sql`. A.1 and A.2 re-confirm what `schema/file.surql` and `docs/permissions.md` already record; the rest are new for this plan.

| # | Behaviour | Result | Used by |
|---|---|---|---|
| A.1 | `FOR update WHERE <pred>` on a table | Evaluated against both the stored row and the proposed row; the write applies only if both pass. `$before` and `$after` are NONE inside the clause. | §2.2, §25.5 |
| A.2 | A denied `UPDATE`/`CREATE` | Returns `OK` with zero rows. No error. | §1.2, §2.3.5 |
| A.3 | `DEFINE EVENT ... WHEN $event = 'UPDATE' ... THEN { THROW ... }` | The event sees `$before`, `$after` and `$auth`; `THROW` aborts the write and returns the message as an error. | §2.3.3, §7.1 |
| A.4 | Field `PERMISSIONS FOR update WHERE $access = 'svc'` | A user signed in via access method `svc` can write the field; the same user shape via `acc` is silently reverted. | §2.3.2, §1.1 |
| A.5 | A `DEFINE FUNCTION` body that `SELECT`s a table the caller cannot read | Returns no rows: functions run with the caller's permissions. | §1.2 |
| A.6 | `generation` as `VALUE IF $before IS NONE THEN 1 ELSE IF crypto::md5($this.content ?? '') != $this.hash THEN $before + 1 ELSE $before END`, with the existing hash EVENT | Increments on content change only (not on a same-content write, a metadata write, or the event's own `hash` write-back). `UPDATE ... WHERE generation = $g` applies once; the stale retry returns zero rows. In a field `VALUE`, `$before`/`$after`/`$value` are that field's previous value, and `$this` is the new row. | §1.1, §1.3 |
| A.7 | Record links to deleted rows | A `record<...>` field keeps its value after the target is deleted; dereferencing it yields NONE. | §1.1 (`file_version`) |
| A.8 | An `EVENT` fired by a record user writing to a table with `FOR create NONE`, and to a field with `FOR update WHERE false` | Both writes succeed; `$auth` inside the event is still the triggering user. An event's own writes fire events again (the `hash` write-back re-triggers `ON file` events). | §1.1, §7.1 |
| A.13 | `DEFINE BUCKET b BACKEND 'memory'` with `--allow-experimental files`; `f'b:/x.txt'.put('hello')`, `.get()`, `.head()` | Works: `get` returns the bytes, `head` returns size and timestamp. | §19.3 |
| A.14 | `search::analyze` with `snowball(german)`, `ngram(1,2)` and `TOKENIZERS class, camel` | `Die Häuser wurden gebaut` → `die haus wurd gebaut`; `東京都` → `東 東京 京 京都 都`; `getUserById parse_json` → `get user by id parse _ json`. | §21.1 |
| A.15 | Per-language FULLTEXT indexes on `option<string>` fields populated only for matching rows | `text_de @@ 'Haus'` finds only the German row and `text_en @@ 'house'` only the English one. | §21.1 |
| A.16 | `DEFINE ACCESS share ON DATABASE TYPE BEARER FOR RECORD DURATION FOR GRANT 7d, FOR SESSION 1h` and `ACCESS share GRANT FOR RECORD person:alice` | Issues a grant with a bearer key, an expiry seven days out and a revocation field. | §24.2 |
| A.11 | `$session` contents | Fixed fields only: `ac`, `db`, `id`, `ip`, `ns`, `or`, `rd`, `tk` (the token claims). No custom fields. | §7.2 |

**To verify before relying on it:**
- **A.9** Whether COMPUTED fields (`path`, `gate`, `is_folder`) are present in `LIVE SELECT DIFF` payloads (§4.2).
- **A.10** `EXPLAIN` for `(branch, parent_key, filename)` lookups (§6.3) and for section search with the permission predicate (§10.3).
- **A.12** Whether connection-level parameters set with `let` are visible inside an `EVENT` body (§7.2).
- **A.17** Which persistent `DEFINE BUCKET` backends are production-ready, and whether bucket `PERMISSIONS` can reference a `file` row (§19.3).
- **A.18** Signing in with a bearer grant and reading its subject from a permission clause, and revocation taking effect on an open session (§24.2).
- **A.19** The cost of `fn::sfs_quota_ok` inside `FOR create` on large trees, and whether `usage` counters updated by events conflict under concurrent writes (§19.6).
- **A.20** That NONE-valued language fields contribute nothing to their FULLTEXT index's statistics (document count and length), so each language's BM25 is computed over its own rows only (§21.1).
