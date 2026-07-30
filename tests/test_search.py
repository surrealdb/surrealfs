"""Full-text and vector search."""

from __future__ import annotations

import math

import pytest

from surrealfs.fs import _rrf


async def test_search_text_finds_and_ranks(fs):
    await fs.write_text("/a.md", "the lazy dog sleeps")
    await fs.write_text("/b.md", "a dog, another dog, and a third dog")
    await fs.write_text("/c.md", "completely unrelated text about cats")

    hits = await fs.search_text("dog")
    assert [h.path for h in hits] == ["/b.md", "/a.md"]
    # Ranked in Python: server-side search::score returns 0.0 on SurrealDB 3.2.
    assert hits[0].score > hits[1].score


async def test_search_text_uses_the_analyzer_stemming(fs):
    await fs.write_text("/a.md", "the dogs were running quickly")
    assert [h.path for h in await fs.search_text("run")] == ["/a.md"]


async def test_search_text_includes_a_snippet(fs):
    await fs.write_text("/a.md", "prelude " * 40 + "the needle is here " + "tail " * 40)
    hit = (await fs.search_text("needle"))[0]
    assert "needle" in hit.snippet
    assert len(hit.snippet) < 300


async def test_search_text_respects_limit(fs):
    for i in range(5):
        await fs.write_text(f"/f{i}.md", "shared keyword")
    assert len(await fs.search_text("keyword", limit=2)) == 2


async def test_search_text_no_matches(fs):
    await fs.write_text("/a.md", "hello")
    assert await fs.search_text("nonexistentwordxyz") == []


async def test_search_text_ignores_empty_query(fs):
    assert await fs.search_text("   ") == []


def _unit_vector(index: int, dimensions: int = 1536) -> list[float]:
    vector = [0.0] * dimensions
    vector[index] = 1.0
    return vector


async def test_search_semantic_orders_by_distance(fs, db):
    """Hand-written vectors — no embedding model involved."""
    for i in range(3):
        entry = await fs.write_text(f"/v{i}.md", f"document {i}")
        await db.query(
            "UPDATE $id SET embedding = $v, embedded_at = time::now()",
            {"id": entry.id, "v": _unit_vector(i)},
        )

    hits = await fs.search_semantic(_unit_vector(1), k=3)
    assert hits[0].path == "/v1.md"
    assert math.isclose(hits[0].score, 0.0, abs_tol=1e-6)
    assert hits[0].score <= hits[1].score <= hits[2].score


async def test_search_semantic_respects_k(fs, db):
    for i in range(5):
        entry = await fs.write_text(f"/v{i}.md", f"doc {i}")
        await db.query(
            "UPDATE $id SET embedding = $v", {"id": entry.id, "v": _unit_vector(i)}
        )
    assert len(await fs.search_semantic(_unit_vector(0), k=2)) == 2


async def test_search_semantic_rejects_bad_k(fs):
    with pytest.raises(ValueError):
        await fs.search_semantic(_unit_vector(0), k=0)


async def test_search_semantic_actually_uses_the_hnsw_index(fs):
    """`<|k,COSINE|>`, or binding k/ef, silently degrades to a table scan."""
    plan = await fs._query(
        f"SELECT id FROM {fs.table} WHERE embedding <|5,40|> $vector EXPLAIN",
        {"vector": _unit_vector(0)},
    )
    assert "KnnScan" in str(plan), f"KNN query is not index-backed: {plan}"


async def test_search_semantic_carries_a_snippet(fs, db):
    """A vector-only hit with no snippet renders as a bare path to the model."""
    entry = await fs.write_text("/v.md", "invoicing and getting paid")
    await db.query(
        "UPDATE $id SET embedding = $v", {"id": entry.id, "v": _unit_vector(0)}
    )
    assert "invoicing" in (await fs.search_semantic(_unit_vector(0), k=1))[0].snippet


def test_rrf_lets_agreement_beat_a_single_top_hit():
    fused = _rrf(["text-only", "both", "x"], ["vector-only", "both", "y"])
    assert next(iter(fused)) == "both"
    assert set(fused) == {"text-only", "vector-only", "both", "x", "y"}


def test_rrf_degrades_to_the_single_arm():
    assert list(_rrf(["a", "b"], [])) == ["a", "b"]
    assert _rrf() == {}


async def test_search_fuses_both_arms(fs, db):
    """The vector arm finds what shares no words with the query."""
    text = await fs.write_text("/text.md", "the keyword appears here")
    vector = await fs.write_text("/vector.md", "utterly different prose")
    for entry, index in ((text, 5), (vector, 0)):
        await db.query(
            "UPDATE $id SET embedding = $v", {"id": entry.id, "v": _unit_vector(index)}
        )

    assert [h.path for h in await fs.search("keyword")] == ["/text.md"]

    fused = await fs.search("keyword", vector=_unit_vector(0))
    assert {h.path for h in fused} == {"/text.md", "/vector.md"}
    # Fused scores, not arm scores: comparable, and every hit describes itself.
    assert all(hit.score > 0 for hit in fused)
    assert all(hit.snippet for hit in fused)


async def test_search_respects_limit_across_both_arms(fs, db):
    for i in range(4):
        entry = await fs.write_text(f"/f{i}.md", "shared keyword")
        await db.query(
            "UPDATE $id SET embedding = $v", {"id": entry.id, "v": _unit_vector(i)}
        )
    assert len(await fs.search("keyword", vector=_unit_vector(0), limit=2)) == 2


async def test_reindex_embeddings_only_touches_stale_rows(fs):
    calls: list[str] = []

    async def embed(text: str) -> list[float]:
        calls.append(text)
        return _unit_vector(len(calls) % 1536)

    await fs.write_text("/a.md", "alpha")
    await fs.write_text("/b.md", "beta")
    await fs.mkdir("/folder")  # no content: must be skipped
    await fs.touch("/empty.txt")  # empty content: must be skipped

    assert await fs.reindex_embeddings(embed, version="v1") == 2
    assert sorted(calls) == ["alpha", "beta"]

    # Nothing changed, so a second pass is a no-op.
    assert await fs.reindex_embeddings(embed, version="v1") == 0

    # Editing content makes that row stale again.
    await fs.write_text("/a.md", "alpha revised")
    assert await fs.reindex_embeddings(embed, version="v1") == 1

    # A new indexer version invalidates everything.
    assert await fs.reindex_embeddings(embed, version="v2") == 2
