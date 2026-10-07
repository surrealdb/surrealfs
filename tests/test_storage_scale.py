"""Tests for content-addressed blob storage, range reads, instant du, and GC."""

from __future__ import annotations

import hashlib

import pytest

from surrealfs import SurrealFs


@pytest.mark.asyncio
async def test_upload_and_range_read(fs: SurrealFs) -> None:
    path = "/archive/large.dat"
    # Create 4 distinct chunks of 4096 bytes each
    chunks = [f"CHUNK_{i:02d}_".encode() * (4096 // 9) for i in range(4)]
    data = b"".join(chunks)
    chunk_size = 4096
    chunk_ids = [f"b3:{hashlib.sha256(c).hexdigest()}" for c in chunks]

    # 1. Begin upload
    begin_res = await fs._query(
        "RETURN fn::sfs_upload_begin($path, $size, $chunk_ids, $caller);",
        {
            "path": path,
            "size": len(data),
            "chunk_ids": chunk_ids,
            "caller": "root",
        },
    )
    session = begin_res
    upload_id = session["upload_id"]
    assert len(session["missing_chunks"]) == len(chunks)

    # 2. Upload chunks
    for cid, chunk_bytes in zip(chunk_ids, chunks, strict=True):
        await fs._query(
            """
            RETURN fn::sfs_upload_chunk(
                $upload_id,
                $chunk_id,
                $uncompressed_size,
                $stored_bytes,
                $codec,
                <bytes>$data,
                $caller
            );
            """,
            {
                "upload_id": upload_id,
                "chunk_id": cid,
                "uncompressed_size": len(chunk_bytes),
                "stored_bytes": len(chunk_bytes),
                "codec": "none",
                "data": list(chunk_bytes),
                "caller": "root",
            },
        )

    # 3. Commit upload
    offsets = [i * chunk_size for i in range(len(chunks))]
    lengths = [len(c) for c in chunks]
    commit_res = await fs._query(
        """
        RETURN fn::sfs_upload_commit(
            $upload_id,
            $if_generation,
            $offsets,
            $lengths,
            $caller
        );
        """,
        {
            "upload_id": upload_id,
            "if_generation": None,
            "offsets": offsets,
            "lengths": lengths,
            "caller": "root",
        },
    )
    entry = commit_res
    assert entry["path"] == path
    assert entry["size"] == len(data)

    # 4. Range read
    range_res = await fs._query(
        "RETURN fn::sfs_read_bytes_range($path, $offset, $length, $caller);",
        {
            "path": path,
            "offset": 100,
            "length": 50,
            "caller": "root",
        },
    )
    assert range_res["chunked"] is True
    assert len(range_res["chunks"]) > 0

    # 5. Du
    du_res = await fs._query(
        "RETURN fn::sfs_du($path, $caller);",
        {"path": "/archive", "caller": "root"},
    )
    usage = du_res
    assert usage["files"] >= 1
    assert usage["logical_bytes"] >= len(data)

    # 6. GC blobs
    gc_res = await fs._query(
        "RETURN fn::sfs_gc_blobs($max_age_secs);",
        {"max_age_secs": 0},
    )
    assert isinstance(gc_res, int)
