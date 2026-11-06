# Blob storage

File content larger than the inline threshold lives in an object store, not in the database. SurrealDB holds the file tree, permissions, versions, chunk manifests, blob metadata and the full-text and vector indexes. The object store holds immutable, content-addressed blob bytes. The gateway moves bytes between clients and the object store directly, and asks SurrealDB only whether it may.

## Responsibilities

| Concern | Owner |
| --- | --- |
| Path resolution, ownership, modes, groups, gates | SurrealDB (`fn::sfs_*`) |
| Chunk boundaries (FastCDC), manifests, dedup bookkeeping, reference counts | SurrealDB |
| Full-text and vector indexes, versions, CRDT state | SurrealDB |
| Blob bytes | Object store (`BlobStore`) |
| Hashing, compression, streaming | The gateway or client |

## Model

A file is inline or chunked. An inline file keeps its bytes in the `file` row (`content` or `file`), as today, and is used for files up to `inline_max_bytes` (default 1 MiB). A chunked file has no inline bytes and one `file_chunk` row per chunk, ordered by `offset`, each pointing at a `blob` row.

A `blob` row is keyed by the BLAKE3 hash of the uncompressed chunk. It records `size`, `stored`, `codec`, `scope`, `refs` and `tier`. Tier `inline` keeps `bytes` in the row. Tier `object` keeps no bytes and names the object in `location`. A reader handles both tiers.

An object is stored under `blobs/<hash[0..2]>/<hash>` in the deployment's blob bucket, optionally beneath an instance prefix. It is written once and never modified.

## BlobStore

`BlobStore` is an async trait in `surrealfs-core`, usable on native and `wasm32` targets.

- `put(key, body, len)` stores a byte stream. It is idempotent: storing a key that exists with the same content succeeds.
- `get(key, range)` returns a byte stream for a whole object or a byte range.
- `head(key)` returns the size or none.
- `delete(key)` removes an object. Deleting a missing key succeeds.

Backends: an in-memory store for tests, a local directory for native single-node use, S3-compatible and cloud stores for native servers, and the R2 binding in the Cloudflare Worker. The backend is chosen by deployment configuration. The database never holds object-store credentials.

## Write path

1. `fn::sfs_upload_begin` checks that the caller may write the path, creates an upload session and returns the chunk ids the scope does not yet hold. Every chunk id it reports as already held is pinned to the session.
2. The gateway chunks the body as it arrives, hashes each chunk, compresses it and puts the missing ones to the object store. Memory use is bounded by the largest chunk. The gateway accepts a chunk only if the bytes hash to the chunk id, so a client proves it holds the bytes by sending them.
3. `fn::sfs_upload_chunk` records a `blob` row of tier `object` for each stored chunk.
4. `fn::sfs_upload_commit` checks the generation, writes the manifest, increments `refs` and updates usage in one transaction.

An object is written before its `blob` row, so a row never names a missing object. A crash between the steps leaves an unreferenced object or an expired session, both reclaimed by garbage collection.

## Read path

1. `fn::sfs_read_manifest(path, offset, length, caller)` checks read permission and returns the covering chunks (`offset`, `length`, `codec`, `size`, tier, key or bytes) with the file's generation and content type. It returns no object bytes.
2. The gateway streams the covering chunks from the object store in order, decompresses each, and trims the first and last to the requested range.

The object keys the gateway reads come only from the manifest SurrealDB returned for that request, so a caller can reach only blobs of a file it was allowed to read. Permission is checked once per request, before any byte moves. The ETag is derived from the generation. HTTP `Range`, `If-None-Match` and `HEAD` are answered from the manifest and need no object reads.

A whole-file read of a chunked file uses this path. No surface assembles a chunked file from an inline column.

## Garbage collection

`fn::sfs_gc_blobs` selects `blob` rows with `refs <= 0` older than the grace period and not pinned by a live upload session, deletes each row and then its object. A sweep also lists objects with no `blob` row older than the grace period and deletes them. The grace period exceeds the upload session lifetime.

## Compression

A chunk's `codec` names how it is stored: `none` or `zstd`. A reader supports every codec a writer in the same deployment produces, so a Worker deployment writes only codecs the Worker can decode.

## Content type

`file.content_type` is set at write time from the request, else from the file name and the leading bytes. The object store records no type of its own, and the gateway serves the type from the `file` row.
