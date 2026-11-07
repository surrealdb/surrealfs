# Blob storage

File content larger than the inline threshold lives in an object store, not in the database. SurrealDB holds the file tree, permissions, versions, chunk manifests, blob metadata and the full-text and vector indexes. The object store holds immutable, content-addressed blob bytes. The gateway moves bytes between clients and the object store directly, and asks SurrealDB only whether it may.

## Responsibilities

| Concern | Owner |
| --- | --- |
| Path resolution, ownership, modes, groups, gates | SurrealDB (`fn::sfs_*`) |
| Chunk boundaries (FastCDC), manifests, dedup bookkeeping | SurrealDB |
| Full-text and vector indexes, versions, CRDT state | SurrealDB |
| Blob bytes | Object store (`BlobStore`) |
| Hashing, compression, streaming | The gateway or client |

## Model

A file is inline or chunked. An inline file keeps its bytes in the `file` row (`content` or `file`), as today, and is used for files up to `inline_max_bytes` (default 1 MiB). A chunked file has no inline bytes and one `file_chunk` row per chunk, ordered by `offset`, each pointing at a `blob` row.

A `blob` row is keyed by the BLAKE3 hash of the uncompressed chunk. It records `size`, `stored`, `codec`, `scope` and `tier`. Tier `inline` keeps `bytes` in the row. Tier `object` keeps no bytes and names the object in `location`. A reader handles both tiers.

An object is stored under `blobs/<hash[0..2]>/<hash>` in the deployment's blob bucket, optionally beneath an instance prefix. It is written once and never modified.

## BlobStore

`BlobStore` is an async trait in `surrealfs-core`, usable on native and `wasm32` targets. A body is at most one chunk, so it is passed whole; a file larger than a chunk is moved chunk by chunk.

- `put(key, data)` stores a body at `key`, replacing any object there. Rewriting a key refreshes its modification time.
- `get(key, range)` returns a whole object or a byte range. A missing key is `NotFound`.
- `head(key)` returns the size and modification time, or none.
- `delete(key)` removes an object. Deleting a missing key succeeds.
- `list(prefix)` returns the objects under a prefix, for the garbage sweep.

Backends: `MemoryBlobStore` for tests and for wasm, `ObjectStoreBlobStore` over any `object_store` backend on native targets (a local directory, or S3, GCS and Azure with the `cloud-stores` feature), and the R2 binding in the Cloudflare Worker. On native targets `SURREALFS_BLOB_STORE` selects one at connect time: `memory`, `dir:<path>` or a bare path, `s3://`, `gs://` or `az://`, with credentials from the environment. With none set, chunk bytes stay in the database (tier `inline`). The database never holds object-store credentials.

## Write path

1. `fn::sfs_upload_begin` checks that the caller may write the path, creates an upload session and returns the chunk ids the scope does not yet hold. Every chunk id it reports as already held is pinned to the session.
2. The gateway chunks the body as it arrives, hashes each chunk, compresses it and puts the missing ones to the object store. Memory use is bounded by the largest chunk. The gateway accepts a chunk only if the bytes hash to the chunk id, so a client proves it holds the bytes by sending them.
3. `fn::sfs_upload_chunk` records a `blob` row of tier `object` for each stored chunk.
4. `fn::sfs_upload_commit` checks the generation, writes the manifest and updates usage.

An object is written before its `blob` row, so a row never names a missing object. A crash between the steps leaves an unreferenced object or an expired session, both reclaimed by garbage collection.

## Read path

1. `read_stream(path, range)` calls `fn::sfs_read_manifest(path, offset, length, caller)`, which checks read permission and returns the file's size, generation and type and the covering chunks (`offset`, `length`, `codec`, tier, key or bytes). It returns no object bytes.
2. The body pulls the manifest a page at a time (64 MiB of file per request) and fetches the covering chunks from the object store one by one, decompressing each and trimming the first and last to the range. Memory use is bounded by a chunk. If the generation changes between pages the body ends with a conflict error, so two versions are never mixed.

The object keys the gateway reads come only from the manifest SurrealDB returned for that request, so a caller can reach only blobs of a file it was allowed to read. Permission is checked before any byte moves, and again for each page. The ETag is derived from the generation. HTTP `Range` (a single `bytes=` range), `If-None-Match` and `HEAD` are answered from the manifest and need no object reads; a range that selects no byte is `416` with the size.

A whole-file read of a chunked file uses this path. No surface assembles a chunked file from an inline column.

## Streamed upload

`upload_stream` reads the body until it exceeds `inline_max_bytes`; a body that ends sooner is written inline. Otherwise it opens an upload session with no chunk list and cuts the body with `StreamChunker`, which produces the chunks one-shot chunking would, holding at most twice the largest chunk. For each chunk it asks `fn::sfs_upload_have`, puts the object when the chunk is new, and appends it with `fn::sfs_upload_part`, which records the blob row and the chunk's place in the session. `fn::sfs_upload_commit` then writes the manifest. A body that stops short ends the stream with an error and nothing is committed.

The HTTP engine hands a `PUT` body longer than `inline_max_bytes` to its handler as a stream, and a response body as a stream, so neither the server nor the Worker holds a file in memory.

Over the JSON RPC transport the Worker uses, SurrealDB reads a string that looks like a record id as one. Chunk ids (`b3-<hex>`) and upload ids therefore contain no colon.

## Garbage collection

A `blob` row is live while a `file_chunk` row names it. Deleting a file deletes its chunk rows, and replacing a file's content deletes the old ones. `fn::sfs_gc_blobs` deletes `blob` rows older than the grace period that no chunk row names and that no live upload session lists. `gc_blobs` then sweeps the object store: it deletes each object last written before the grace period whose chunk id has no `blob` row. The grace period must exceed the upload session lifetime, so an object re-put during a sweep is young and survives it.

## Permissions of the manifest

`file_chunk` is writable only by a service session, so `fn::sfs_upload_commit` refuses a record-user session instead of committing a file whose manifest the permissions would silently drop. Chunked uploads run through a gateway or client session that passes the caller's identity.

## Compression

A chunk's `codec` names how it is stored: `none` or `zstd`. A reader supports every codec a writer in the same deployment produces. The Worker decodes `zstd` with `ruzstd` and writes `none`, since it has no encoder.

## Content type

`file.content_type` is set at write time from the request, else from the file name and the leading bytes. The object store records no type of its own, and the gateway serves the type from the `file` row.
