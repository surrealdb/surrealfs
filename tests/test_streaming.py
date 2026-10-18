"""Chunked files: written and read as sequences of pieces, never whole."""

from __future__ import annotations

import json
import math
import random

import httpx
import pytest

from surrealfs import (
    DEFAULT_CHUNK_SIZE,
    NotATextFile,
    NotFound,
    PermissionDenied,
    SurrealFs,
    apply_schema,
)
from surrealfs import browser as browser_app
from surrealfs.fs import raise_for_status
from surrealfs.schema import gc_chunks
from surrealfs.tools import get_tool
from surrealfs.users import add as add_user

CHUNK = 1000
# Big enough for several chunks at CHUNK, and not a multiple of it.
PAYLOAD = random.Random(7).randbytes(3 * CHUNK + 321)


async def rows(connection, sql, variables=None):
    raw = await connection.query_raw(sql, variables or {})
    return raise_for_status(raw)[-1] or []


async def chunk_count(db) -> int:
    counted = await rows(db, "SELECT count() FROM file_chunk GROUP ALL")
    return counted[0]["count"] if counted else 0


async def pieces(data: bytes, step: int):
    """`data` as an async generator of pieces that do not line up with chunks."""
    for start in range(0, len(data), step):
        yield data[start : start + step]


async def collect(stream) -> bytes:
    return b"".join([piece async for piece in stream])


async def put(fs, path="/big.bin", data=PAYLOAD, **kwargs):
    return await fs.write_stream(path, pieces(data, 437), chunk_size=CHUNK, **kwargs)


# ------------------------------------------------------------------- writing


async def test_a_stream_is_stored_as_chunks_and_reads_back_whole(fs, db):
    data = random.Random(1).randbytes(1_100_000)

    entry = await fs.write_stream("/big.bin", pieces(data, 77_777))

    assert entry.size == len(data)
    assert entry.is_chunked and entry.is_binary and not entry.is_folder
    assert await fs.read_bytes("/big.bin") == data
    assert (await fs.stat("/big.bin")).size == len(data)
    assert await chunk_count(db) == math.ceil(len(data) / DEFAULT_CHUNK_SIZE)


async def test_chunks_are_exactly_chunk_size_but_the_last(fs, db):
    await put(fs)

    stored = await rows(db, "SELECT idx, data FROM file_chunk ORDER BY idx")
    assert [len(row["data"]) for row in stored] == [CHUNK, CHUNK, CHUNK, 321]
    assert b"".join(row["data"] for row in stored) == PAYLOAD
    row = (await rows(db, "SELECT * FROM file WHERE blob IS NOT NONE"))[0]
    assert (row["blob_size"], row["blob_chunk_size"]) == (len(PAYLOAD), CHUNK)


async def test_a_sync_iterable_is_accepted(fs):
    await fs.write_stream(
        "/s.bin",
        [PAYLOAD[:1500], PAYLOAD[1500:]],
        chunk_size=CHUNK,
    )
    assert await fs.read_bytes("/s.bin") == PAYLOAD


async def test_a_stream_of_exactly_one_chunk_is_stored_inline(fs, db):
    entry = await fs.write_stream("/one.bin", [PAYLOAD[:CHUNK]], chunk_size=CHUNK)
    assert not entry.is_chunked
    assert await chunk_count(db) == 0
    assert await fs.read_bytes("/one.bin") == PAYLOAD[:CHUNK]


async def test_an_empty_stream_is_an_empty_inline_file(fs, db):
    entry = await fs.write_stream("/empty.bin", [])
    assert (entry.size, entry.is_chunked, entry.is_folder) == (0, False, False)
    assert await fs.read_bytes("/empty.bin") == b""
    assert await chunk_count(db) == 0


async def test_a_small_text_stream_is_stored_as_text(fs):
    text = "# Notes\n\nhéllo\n"
    entry = await fs.write_stream(
        "/notes/a.md", [text.encode()], content_type="text/markdown"
    )

    assert not entry.is_chunked
    assert await fs.read_text("/notes/a.md") == text
    assert (await fs.stat("/notes/a.md")).hash
    await fs.edit("/notes/a.md", "héllo", "hello")
    assert await fs.read_text("/notes/a.md") == "# Notes\n\nhello\n"
    assert await fs.tail("/notes/a.md", 1) == "hello"


async def test_a_small_stream_that_is_not_text_stays_bytes(fs):
    await fs.write_stream("/a.bin", [b"abc"])
    await fs.write_stream("/b.md", [b"\xff\xfe"], content_type="text/markdown")

    for path in ("/a.bin", "/b.md"):
        with pytest.raises(NotATextFile):
            await fs.read_text(path)
    assert await fs.read_bytes("/b.md") == b"\xff\xfe"


async def test_a_text_stream_longer_than_a_chunk_is_chunked(fs):
    text = ("line of text\n" * 200).encode()
    entry = await fs.write_stream(
        "/long.md", pieces(text, 300), content_type="text/markdown", chunk_size=CHUNK
    )
    assert entry.is_chunked
    assert await fs.read_bytes("/long.md") == text
    with pytest.raises(NotATextFile):
        await fs.read_text("/long.md")


async def test_write_stream_rejects_what_is_not_bytes(fs, db):
    with pytest.raises(TypeError):
        await fs.write_stream("/x.bin", ["text"])
    with pytest.raises(ValueError):
        await fs.write_stream("/x.bin", [b"x"], chunk_size=0)
    assert await chunk_count(db) == 0


# ------------------------------------------------------------------- reading

INLINE_TEXT = "héllo wörld, once more"
KINDS = {
    "chunked": PAYLOAD,
    "bytes": PAYLOAD[:700],
    "text": INLINE_TEXT.encode(),
}


@pytest.fixture
async def kinds(fs):
    await put(fs, "/chunked")
    await fs.write_bytes("/bytes", KINDS["bytes"])
    await fs.write_text("/text", INLINE_TEXT)
    return fs


RANGES = {
    "start": lambda n: (0, 10),
    "middle spanning a chunk boundary": lambda n: (CHUNK - 5, 11),
    "several whole chunks": lambda n: (CHUNK - 1, 2 * CHUNK + 2),
    "tail": lambda n: (n - 7, None),
    "to the end, and past it": lambda n: (n - 7, 1000),
    "past EOF": lambda n: (n + 5, 10),
    "at EOF": lambda n: (n, None),
    "zero length": lambda n: (3, 0),
    "the whole file": lambda n: (0, None),
}


@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("case", RANGES)
async def test_open_stream_yields_exactly_the_range(kinds, kind, case):
    data = KINDS[kind]
    offset, length = RANGES[case](len(data))
    expected = data[offset : None if length is None else offset + length]

    got = await collect(kinds.open_stream(f"/{kind}", offset=offset, length=length))

    assert got == expected


@pytest.mark.parametrize("kind", KINDS)
async def test_open_stream_pieces_are_at_most_chunk_size(kinds, kind):
    pieces_seen = [
        p async for p in kinds.open_stream(f"/{kind}", chunk_size=300, offset=100)
    ]
    assert all(0 < len(p) <= 300 for p in pieces_seen)
    assert b"".join(pieces_seen) == KINDS[kind][100:]


@pytest.mark.parametrize("kind", KINDS)
async def test_read_range_is_the_join_of_the_stream(kinds, kind):
    data = KINDS[kind]
    assert await kinds.read_range(f"/{kind}", 5, 2000) == data[5:2005]
    assert await kinds.read_range(f"/{kind}", len(data) + 1, 5) == b""


async def test_read_slice_reports_the_size_in_bytes(kinds):
    data, size = await kinds.read_slice("/text", 0, 3)
    assert data == INLINE_TEXT.encode()[:3]
    # Not `stat().size`, which counts the text in characters.
    assert size == len(INLINE_TEXT.encode()) > len(INLINE_TEXT)
    assert (await kinds.read_slice("/chunked", 0, 1))[1] == len(PAYLOAD)


async def test_a_range_must_not_be_negative(kinds):
    with pytest.raises(ValueError):
        await collect(kinds.open_stream("/chunked", offset=-1))
    with pytest.raises(ValueError):
        await collect(kinds.open_stream("/chunked", length=-1))
    with pytest.raises(ValueError):
        await kinds.read_range("/chunked", 0, -1)
    with pytest.raises(ValueError):
        await kinds.read_range("/chunked", -1, 1)


async def test_a_stream_of_a_missing_path_or_folder_raises(fs):
    await fs.mkdir("/d")
    with pytest.raises(NotFound):
        await collect(fs.open_stream("/nope"))
    with pytest.raises(OSError):
        await collect(fs.open_stream("/d"))


async def test_a_file_rewritten_mid_read_raises_instead_of_mixing(fs):
    await put(fs)
    stream = fs.open_stream("/big.bin", chunk_size=CHUNK)
    assert len(await anext(stream)) == CHUNK

    await put(fs, data=bytes(len(PAYLOAD)))

    with pytest.raises(NotFound):
        await collect(stream)


# ------------------------------------------------------------------ replacing


async def test_chunked_to_inline_to_chunked_leaves_no_orphans(fs, db):
    await put(fs)
    assert await chunk_count(db) == 4

    await fs.write_text("/big.bin", "now small")
    assert await chunk_count(db) == 0
    assert await fs.read_text("/big.bin") == "now small"
    assert not (await fs.stat("/big.bin")).is_chunked

    await put(fs)
    assert await chunk_count(db) == 4
    stat = await fs.stat("/big.bin")
    assert stat.is_chunked and not stat.hash
    assert await fs.read_bytes("/big.bin") == PAYLOAD

    await fs.write_bytes("/big.bin", b"\x00\x01")
    assert await chunk_count(db) == 0
    assert await fs.read_bytes("/big.bin") == b"\x00\x01"


async def test_chunked_over_chunked_replaces_the_blob(fs, db):
    await put(fs)
    old = (await rows(db, "SELECT VALUE blob FROM file WHERE blob IS NOT NONE"))[0]
    other = random.Random(2).randbytes(2 * CHUNK + 5)

    await put(fs, data=other)

    assert await fs.read_bytes("/big.bin") == other
    assert await chunk_count(db) == 3
    assert not await rows(db, "SELECT * FROM file_chunk WHERE blob = $b", {"b": old})


async def test_a_text_file_overwritten_by_a_chunked_one_loses_its_hash(fs):
    """The hash event must not be handed text that is no longer there."""
    await fs.write_text("/f.md", "hello")
    assert (await fs.stat("/f.md")).hash

    await put(fs, "/f.md")

    assert (await fs.stat("/f.md")).hash == ""
    assert await fs.read_bytes("/f.md") == PAYLOAD


async def test_a_failing_stream_leaves_the_original_intact(fs, db):
    async def failing():
        yield b"x" * (2 * CHUNK + 10)
        raise RuntimeError("source died")

    await put(fs)
    with pytest.raises(RuntimeError, match="source died"):
        await fs.write_stream("/big.bin", failing(), chunk_size=CHUNK)
    assert await fs.read_bytes("/big.bin") == PAYLOAD
    assert await chunk_count(db) == 4

    await fs.write_text("/note.md", "original")
    with pytest.raises(RuntimeError, match="source died"):
        await fs.write_stream("/note.md", failing(), chunk_size=CHUNK)
    assert await fs.read_text("/note.md") == "original"
    assert await chunk_count(db) == 4

    with pytest.raises(RuntimeError, match="source died"):
        await fs.write_stream("/fresh.bin", failing(), chunk_size=CHUNK)
    assert not await fs.exists("/fresh.bin")
    assert await chunk_count(db) == 4


async def test_a_failed_commit_deletes_the_staged_chunks(fs, db):
    await fs.mkdir("/d")
    with pytest.raises(OSError):
        await fs.write_stream("/d", pieces(PAYLOAD, 500), chunk_size=CHUNK)
    assert await chunk_count(db) == 0


async def test_a_racing_writer_cannot_orphan_the_blob_it_replaced(fs, db):
    """Both writers read the same old blob; only the last swap's chunks survive."""
    await put(fs)
    other = random.Random(5).randbytes(2 * CHUNK + 7)

    async def racing():
        yield other[:CHUNK]
        yield other[CHUNK : 2 * CHUNK]
        # Lands between this stream's staging and its swap.
        await put(fs, data=b"z" * (CHUNK + 1))
        yield other[2 * CHUNK :]

    await fs.write_stream("/big.bin", racing(), chunk_size=CHUNK)

    assert await fs.read_bytes("/big.bin") == other
    assert await chunk_count(db) == 3


async def test_a_creator_that_loses_a_race_leaves_the_winner_intact(fs, db):
    async def racing():
        yield b"a" * (2 * CHUNK)
        await fs.write_text("/race.md", "winner")
        yield b"a"

    with pytest.raises(Exception, match="already contains"):
        await fs.write_stream("/race.md", racing(), chunk_size=CHUNK)

    assert await fs.read_text("/race.md") == "winner"
    assert await chunk_count(db) == 0


async def test_an_error_after_the_swap_committed_keeps_the_new_content(
    fs, db, monkeypatch
):
    original = fs._replace

    async def commit_then_fail(*args, **kwargs):
        await original(*args, **kwargs)
        raise ConnectionError("the reply was lost")

    await put(fs)
    monkeypatch.setattr(fs, "_replace", commit_then_fail)
    other = random.Random(6).randbytes(2 * CHUNK + 1)
    with pytest.raises(ConnectionError):
        await put(fs, data=other)

    assert await fs.read_bytes("/big.bin") == other
    assert await chunk_count(db) == 3


async def test_overlapping_writers_all_succeed(fs, db):
    """Every content write is a transaction, and conflicts between them retry."""
    import asyncio

    await fs.write_text("/hot.md", "start")

    await asyncio.gather(*(fs.write_text("/hot.md", f"v{i}") for i in range(20)))

    assert (await fs.read_text("/hot.md")).startswith("v")


async def test_a_file_that_stops_being_text_stops_matching_vector_search(fs, db):
    await fs.write_text("/note.md", "about zebras")
    vector = [0.5] + [0.01] * 1535
    await rows(
        db, "UPDATE file SET embedding = $v WHERE filename = 'note.md'", {"v": vector}
    )
    assert [h.path for h in await fs.search_semantic(vector, k=5)] == ["/note.md"]

    await put(fs, "/note.md")

    row = (
        await rows(
            db, "SELECT embedding, embedded_hash FROM file WHERE filename='note.md'"
        )
    )[0]
    assert row["embedding"] is None and row["embedded_hash"] is None
    assert await fs.search_semantic(vector, k=5) == []
    assert await fs.search_text("zebras") == []

    await fs.write_text("/note.md", "about zebras")
    await rows(
        db, "UPDATE file SET embedding = $v WHERE filename = 'note.md'", {"v": vector}
    )
    await fs.write_bytes("/note.md", b"\x00")
    assert await fs.search_semantic(vector, k=5) == []


async def test_gc_removes_chunks_no_file_points_at(fs, db):
    await put(fs)
    for index in range(2):
        await rows(
            db,
            "CREATE type::record('file_chunk', ['dead', $i]) CONTENT "
            "{ blob: 'dead', idx: $i, data: <bytes>'x', owner: 'root' }",
            {"i": index},
        )
    assert await chunk_count(db) == 6

    # A chunk this young might belong to an upload that is still running.
    assert await gc_chunks(db) == 0
    assert await gc_chunks(db, grace=0) == 2

    assert await chunk_count(db) == 4
    assert await fs.read_bytes("/big.bin") == PAYLOAD


# ------------------------------------------------------- rm, cp, mv, text ops


async def test_rm_deletes_the_chunks(fs, db):
    await put(fs)
    assert await fs.rm("/big.bin") == 1
    assert await chunk_count(db) == 0
    assert not await fs.exists("/big.bin")


async def test_recursive_rm_deletes_every_chunk_below(fs, db):
    await put(fs, "/tree/a.bin")
    await put(fs, "/tree/deep/b.bin")
    await fs.write_text("/tree/c.md", "text")
    await put(fs, "/keep.bin")
    assert await chunk_count(db) == 12

    assert await fs.rm("/tree", recursive=True) == 5

    assert await chunk_count(db) == 4
    assert await fs.read_bytes("/keep.bin") == PAYLOAD
    assert [e.path for e in await fs.ls("/")] == ["/home", "/keep.bin"]


async def test_cp_makes_an_independent_blob(fs, db):
    await put(fs, "/a.bin", content_type="image/png")

    copied = await fs.cp("/a.bin", "/b/c.bin")

    assert copied.is_chunked and copied.size == len(PAYLOAD)
    assert copied.content_type == "image/png"
    assert await chunk_count(db) == 8
    blobs = await rows(db, "SELECT VALUE blob FROM file WHERE blob IS NOT NONE")
    assert len(set(blobs)) == 2
    await fs.rm("/a.bin")
    assert await chunk_count(db) == 4
    assert await fs.read_bytes("/b/c.bin") == PAYLOAD


async def test_recursive_cp_copies_chunked_children(fs, db):
    await put(fs, "/src/deep/a.bin")
    await fs.write_text("/src/b.md", "text")

    await fs.cp("/src", "/dst", recursive=True)

    assert await fs.read_bytes("/dst/deep/a.bin") == PAYLOAD
    assert await fs.read_text("/dst/b.md") == "text"
    assert await chunk_count(db) == 8


async def test_mv_keeps_the_content(fs, db):
    await put(fs, "/a.bin")
    await fs.mkdir("/x")
    await fs.mv("/a.bin", "/x/b.bin")
    assert await fs.read_bytes("/x/b.bin") == PAYLOAD
    assert await chunk_count(db) == 4


async def test_a_chunked_file_is_binary_to_the_text_operations(fs):
    await put(fs)

    with pytest.raises(NotATextFile):
        await fs.read_text("/big.bin")
    with pytest.raises(NotATextFile):
        await fs.tail("/big.bin")
    with pytest.raises(NotATextFile):
        await fs.edit("/big.bin", "a", "b")
    listing = {e.path: e for e in await fs.ls("/")}
    assert not listing["/big.bin"].is_folder
    assert listing["/big.bin"].size == len(PAYLOAD)
    assert not (await fs.stat("/big.bin")).is_folder
    assert [e.path for e in await fs.glob("/*.bin")] == ["/big.bin"]


async def test_a_chunked_file_is_not_searchable(fs):
    await fs.write_stream(
        "/long.md",
        pieces(b"zebra stripes " * 200, 100),
        content_type="text/markdown",
        chunk_size=CHUNK,
    )
    assert await fs.search_text("zebra") == []


async def test_one_blob_cannot_name_two_files(fs, db):
    await put(fs, "/a.bin")
    blob = (await rows(db, "SELECT VALUE blob FROM file WHERE blob IS NOT NONE"))[0]
    with pytest.raises(Exception, match="already contains"):
        await rows(
            db,
            "CREATE file SET filename = 'twin', parent = NONE, "
            "content_type = 'x', blob = $b, blob_size = 1, blob_chunk_size = 1",
            {"b": blob},
        )


# ------------------------------------------------------------------ the tool


async def read_tool(ctx, **args) -> dict:
    return json.loads(await get_tool("read_bytes")(ctx, args))


BIG = random.Random(3).randbytes(150_000)


@pytest.fixture
async def big(ctx):
    await ctx.fs.write_stream("/big.bin", pieces(BIG, 40_000), chunk_size=50_000)
    await ctx.fs.write_bytes("/inline.bin", BIG)
    return ctx


@pytest.mark.parametrize("path", ["/big.bin", "/inline.bin"])
async def test_read_bytes_returns_at_most_64kib_a_call(big, path):
    import base64

    first = await read_tool(big, path=path)
    assert (first["size"], first["offset"]) == (len(BIG), 0)
    assert first["next_offset"] == 64 * 1024
    assert base64.b64decode(first["data_base64"]) == BIG[: 64 * 1024]

    # Asking for more than the cap gets the cap.
    greedy = await read_tool(big, path=path, offset=10, length=10**9)
    assert base64.b64decode(greedy["data_base64"]) == BIG[10 : 10 + 64 * 1024]
    assert greedy["next_offset"] == 10 + 64 * 1024

    got, offset = b"", 0
    while offset is not None:
        page = await read_tool(big, path=path, offset=offset)
        got += base64.b64decode(page["data_base64"])
        offset = page["next_offset"]
    assert got == BIG


async def test_read_bytes_honours_a_short_length_and_the_end(big):
    import base64

    page = await read_tool(big, path="/big.bin", offset=100, length=5)
    assert base64.b64decode(page["data_base64"]) == BIG[100:105]
    assert page["next_offset"] == 105

    tail = await read_tool(big, path="/big.bin", offset=len(BIG) - 3)
    assert base64.b64decode(tail["data_base64"]) == BIG[-3:]
    assert tail["next_offset"] is None

    beyond = await read_tool(big, path="/big.bin", offset=len(BIG) + 9)
    assert beyond == {
        "data_base64": "",
        "size": len(BIG),
        "offset": len(BIG) + 9,
        "next_offset": None,
    }


async def test_a_small_file_comes_back_whole_in_one_call(ctx):
    import base64

    await ctx.fs.write_bytes("/s.bin", b"\x89PNG\x00\xff")
    page = await read_tool(ctx, path="/s.bin")
    assert base64.b64decode(page["data_base64"]) == b"\x89PNG\x00\xff"
    assert (page["size"], page["offset"], page["next_offset"]) == (6, 0, None)


def test_the_tool_schema_advertises_the_range():
    schema = get_tool("read_bytes").json_schema
    assert {"path", "offset", "length"} == set(schema["properties"])
    assert schema["required"] == ["path"]
    assert "64 KiB" in get_tool("read_bytes").description


async def test_reading_a_file_the_tool_cannot_use_is_an_error(ctx):
    with pytest.raises(NotFound):
        await get_tool("read_bytes")(ctx, {"path": "/nope"})
    with pytest.raises(ValueError):
        await get_tool("read_bytes")(ctx, {"path": "/x", "offset": -1})


# --------------------------------------------------------------- the browser


def client(fs) -> httpx.AsyncClient:
    return httpx.AsyncClient(
        transport=httpx.ASGITransport(app=browser_app.build_app(fs)),
        base_url="http://browser",
    )


async def test_raw_serves_a_chunked_file_whole(fs):
    await put(fs, "/big.bin", content_type="image/png")
    async with client(fs) as http:
        response = await http.get("/raw", params={"path": "/big.bin"})

    assert response.status_code == 200
    assert response.content == PAYLOAD
    assert response.headers["content-length"] == str(len(PAYLOAD))
    assert response.headers["accept-ranges"] == "bytes"
    assert response.headers["content-type"] == "image/png"
    assert response.headers["x-content-type-options"] == "nosniff"
    assert response.headers["content-security-policy"] == "sandbox"


@pytest.mark.parametrize(
    "header, first, last",
    [
        ("bytes=0-9", 0, 9),
        (f"bytes={CHUNK - 3}-{CHUNK + 4}", CHUNK - 3, CHUNK + 4),
        ("bytes=3000-", 3000, len(PAYLOAD) - 1),
        ("bytes=-100", len(PAYLOAD) - 100, len(PAYLOAD) - 1),
        (f"bytes=5-{10**9}", 5, len(PAYLOAD) - 1),
    ],
)
async def test_raw_honours_a_single_range(fs, header, first, last):
    await put(fs)
    async with client(fs) as http:
        response = await http.get(
            "/raw", params={"path": "/big.bin"}, headers={"Range": header}
        )

    assert response.status_code == 206
    assert response.content == PAYLOAD[first : last + 1]
    assert response.headers["content-range"] == (f"bytes {first}-{last}/{len(PAYLOAD)}")
    assert response.headers["content-length"] == str(last - first + 1)


async def test_raw_refuses_a_range_past_the_end(fs):
    await put(fs)
    async with client(fs) as http:
        for header in (f"bytes={len(PAYLOAD)}-", f"bytes={len(PAYLOAD) + 50}-60"):
            response = await http.get(
                "/raw", params={"path": "/big.bin"}, headers={"Range": header}
            )
            assert response.status_code == 416
            assert response.headers["content-range"] == f"bytes */{len(PAYLOAD)}"


async def test_raw_serves_a_multi_range_request_in_full(fs):
    await put(fs)
    async with client(fs) as http:
        response = await http.get(
            "/raw",
            params={"path": "/big.bin"},
            headers={"Range": "bytes=0-1,5-6"},
        )
    assert response.status_code == 200
    assert response.content == PAYLOAD


async def test_raw_ranges_an_inline_text_file_in_bytes(fs):
    await fs.write_text("/t.md", INLINE_TEXT)
    data = INLINE_TEXT.encode()
    async with client(fs) as http:
        whole = await http.get("/raw", params={"path": "/t.md"})
        part = await http.get(
            "/raw", params={"path": "/t.md"}, headers={"Range": "bytes=1-4"}
        )
    assert whole.content == data
    assert whole.headers["content-length"] == str(len(data))
    assert (part.status_code, part.content) == (206, data[1:5])
    assert part.headers["content-range"] == f"bytes 1-4/{len(data)}"


async def test_raw_of_a_missing_file_is_a_404_and_of_a_folder_an_error(fs):
    await fs.mkdir("/d")
    async with client(fs) as http:
        assert (await http.get("/raw", params={"path": "/nope"})).status_code == 404
        assert (await http.get("/raw", params={"path": "/d"})).status_code >= 400


async def test_upload_streams_the_body_into_a_chunked_file(fs, db):
    data = random.Random(4).randbytes(DEFAULT_CHUNK_SIZE * 2 + 99)

    async def body():
        for start in range(0, len(data), 50_000):
            yield data[start : start + 50_000]

    async with client(fs) as http:
        response = await http.put(
            "/api/upload",
            params={"path": "/up/big.bin"},
            content=body(),
            headers={"Content-Type": "image/png"},
        )
        assert response.status_code == 200, response.text
        assert response.json()["size"] == len(data)
        got = await http.get("/raw", params={"path": "/up/big.bin"})

    assert got.content == data
    assert got.headers["content-type"] == "image/png"
    assert (await fs.stat("/up/big.bin")).is_chunked
    assert await chunk_count(db) == 3


async def test_upload_of_a_small_text_body_is_an_ordinary_note(fs):
    async with client(fs) as http:
        response = await http.put(
            "/api/upload", params={"path": "/n.md"}, content=b"# hi\n"
        )
        assert response.status_code == 200, response.text
    assert await fs.read_text("/n.md") == "# hi\n"
    assert (await fs.stat("/n.md")).content_type == "text/markdown"


# ------------------------------------------------------------- record auth


async def blob_of(db, filename: str) -> str:
    found = await rows(
        db, "SELECT VALUE blob FROM file WHERE filename = $f", {"f": filename}
    )
    return found[0]


@pytest.fixture
async def tenants(db, signed_in):
    """Alice and bob over record connections, and three of alice's chunked files.

    `/home/alice/secret.bin` is behind her private home, `/shared.bin` is in the
    shared root, and `/pub/locked.bin` is readable by bob in a folder he cannot
    write.
    """
    await apply_schema(db, record_auth=True)
    await add_user(db, "alice", "pw-alice")
    await add_user(db, "bob", "pw-bob")
    alice_raw = await signed_in("alice", "pw-alice")
    bob_raw = await signed_in("bob", "pw-bob")
    alice = SurrealFs(alice_raw, user="alice")
    bob = SurrealFs(bob_raw, user="bob")
    await put(alice, "/home/alice/secret.bin")
    await put(alice, "/shared.bin")
    await put(alice, "/pub/locked.bin")
    await alice.chmod("/pub", 0o755)
    await alice.chmod("/pub/locked.bin", 0o644)
    return alice, bob, alice_raw, bob_raw


async def test_chunks_of_a_private_file_are_invisible_to_raw_selects(tenants, db):
    alice, bob, alice_raw, bob_raw = tenants
    secret = await blob_of(db, "secret.bin")
    point = "SELECT data FROM ONLY type::record('file_chunk', [$b, 0])"

    assert (
        await rows(bob_raw, "SELECT * FROM file_chunk WHERE blob=$b", {"b": secret})
        == []
    )
    assert not await rows(bob_raw, point, {"b": secret})
    assert await rows(alice_raw, point, {"b": secret})
    # Bob sees the chunks of what he may read, and only those.
    visible = {
        row["blob"] for row in await rows(bob_raw, "SELECT blob FROM file_chunk")
    }
    assert visible == {await blob_of(db, "shared.bin"), await blob_of(db, "locked.bin")}
    assert await bob.read_bytes("/shared.bin") == PAYLOAD
    assert await bob.read_bytes("/pub/locked.bin") == PAYLOAD
    with pytest.raises((NotFound, PermissionDenied)):
        await bob.read_bytes("/home/alice/secret.bin")
    with pytest.raises((NotFound, PermissionDenied)):
        await collect(bob.open_stream("/home/alice/secret.bin"))


async def test_a_record_user_cannot_overwrite_another_users_file(tenants, db):
    alice, bob, _, _ = tenants

    for path in ("/pub/locked.bin", "/home/alice/secret.bin"):
        with pytest.raises(PermissionDenied):
            await put(bob, path, data=b"x" * 5000)

    assert await alice.read_bytes("/pub/locked.bin") == PAYLOAD
    assert await alice.read_bytes("/home/alice/secret.bin") == PAYLOAD
    assert await chunk_count(db) == 12


async def test_raw_writes_cannot_touch_chunks_of_a_file_they_may_not_write(tenants, db):
    alice, bob, alice_raw, bob_raw = tenants
    blob = await blob_of(db, "locked.bin")
    create = (
        "CREATE type::record('file_chunk', [$b, $i]) CONTENT "
        "{ blob: $b, idx: $i, data: <bytes>'zz', owner: $o } RETURN id"
    )

    assert await rows(bob_raw, create, {"b": blob, "i": 9, "o": "bob"}) == []
    assert await rows(bob_raw, "DELETE file_chunk WHERE blob = $b", {"b": blob}) == []
    assert await rows(bob_raw, "UPDATE file_chunk SET data = <bytes>'zz'") == []
    assert await chunk_count(db) == 12
    assert await bob.read_bytes("/pub/locked.bin") == PAYLOAD


async def test_a_chunk_cannot_be_filed_under_another_blobs_id(tenants):
    _, _, _, bob_raw = tenants

    forged = await rows(
        bob_raw,
        "CREATE type::record('file_chunk', ['theirs', 99]) CONTENT "
        "{ blob: 'mine', idx: 99, data: <bytes>'zz', owner: 'bob' } RETURN id",
    )

    assert forged == []


async def test_staged_chunks_belong_to_whoever_staged_them(tenants, db):
    _, _, alice_raw, bob_raw = tenants
    create = (
        "CREATE type::record('file_chunk', [$b, 0]) CONTENT "
        "{ blob: $b, idx: 0, data: <bytes>'zz', owner: $o } RETURN id"
    )

    assert await rows(bob_raw, create, {"b": "fresh", "o": "bob"})
    # Not under somebody else's name.
    assert await rows(bob_raw, create, {"b": "forged", "o": "alice"}) == []
    # And nobody else may read or remove them.
    assert await rows(alice_raw, "SELECT * FROM file_chunk WHERE blob='fresh'") == []
    assert await rows(alice_raw, "DELETE file_chunk WHERE blob='fresh'") == []
    assert await rows(db, "SELECT * FROM file_chunk WHERE blob='fresh'")
    assert await rows(bob_raw, "DELETE file_chunk WHERE blob='fresh' RETURN id")


async def test_a_record_user_can_rewrite_replace_copy_and_remove_their_own(tenants, db):
    alice, _, _, _ = tenants

    await put(alice, "/shared.bin", data=b"y" * (2 * CHUNK + 1))
    assert await alice.read_bytes("/shared.bin") == b"y" * (2 * CHUNK + 1)
    assert await chunk_count(db) == 11

    await alice.write_text("/shared.bin", "small")
    assert await chunk_count(db) == 8

    await alice.cp("/home/alice/secret.bin", "/home/alice/copy.bin")
    assert await alice.read_bytes("/home/alice/copy.bin") == PAYLOAD
    assert await chunk_count(db) == 12

    await alice.rm("/home/alice", recursive=True)
    assert await chunk_count(db) == 4


async def test_rm_of_a_read_only_file_in_a_writable_folder_takes_its_chunks(
    tenants, db
):
    alice, bob, _, _ = tenants
    await alice.chmod("/shared.bin", 0o444)

    await bob.rm("/shared.bin")

    assert not await alice.exists("/shared.bin")
    assert await chunk_count(db) == 8


async def test_a_refused_rm_keeps_the_chunks(tenants, db):
    alice, bob, _, bob_raw = tenants

    with pytest.raises(PermissionDenied):
        await bob.rm("/pub/locked.bin")
    # The database refuses it on its own too, and rolls the whole thing back.
    with pytest.raises(PermissionDenied):
        await bob._delete_rows(
            [
                (
                    await rows(
                        bob_raw, "SELECT VALUE id FROM file WHERE filename='locked.bin'"
                    )
                )[0]
            ],
            [await blob_of(db, "locked.bin")],
            "/pub/locked.bin",
        )

    assert await chunk_count(db) == 12
    assert await alice.read_bytes("/pub/locked.bin") == PAYLOAD
