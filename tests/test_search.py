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


async def test_a_stemmed_match_is_scored_not_just_matched(fs):
    """The index stems, so scoring has to as well.

    "Prefers" matches a query for "prefer" only because `snowball(english)`
    reduces both. Counting raw words instead gave the file 0.000 and sorted it
    below files that did not answer the question, which is why scoring runs on
    `search::analyze` output for the content and the query alike.
    """
    await fs.write_text("/voice.md", "Prefers terse replies.")
    await fs.write_text("/other.md", "Nothing to do with it.")

    hit = (await fs.search_text("prefer"))[0]
    assert hit.path == "/voice.md"
    assert hit.score > 0, "matched by stem but scored zero"


async def test_a_focused_file_beats_one_that_merely_mentions_the_term(fs):
    """Length normalisation: the same match counts for more in a shorter file."""
    await fs.write_text("/focused.md", "Kubernetes ingress notes.")
    await fs.write_text(
        "/sprawling.md",
        "Kubernetes ingress notes. " + "Assorted unrelated meeting minutes. " * 40,
    )

    hits = await fs.search_text("kubernetes ingress")
    assert [h.path for h in hits] == ["/focused.md", "/sprawling.md"], [
        (h.path, round(h.score, 3)) for h in hits
    ]


async def test_repetition_saturates(fs):
    """Saying a word twenty times does not make a file twenty times better."""
    await fs.write_text("/once.md", "kubernetes ingress notes")
    await fs.write_text("/chanted.md", "kubernetes " * 20)

    scores = {h.path: h.score for h in await fs.search_text("kubernetes")}
    assert scores["/chanted.md"] < 3 * scores["/once.md"], scores


async def test_a_rare_term_outweighs_a_common_one(fs):
    for i in range(6):
        await fs.write_text(f"/common{i}.md", "meeting notes about the roadmap")
    await fs.write_text("/rare.md", "meeting notes about the kubernetes migration")

    hits = await fs.search_text("roadmap kubernetes")
    assert hits[0].path == "/rare.md"


async def test_a_query_of_only_stopwords_still_ranks(fs):
    """Dropping every term would leave nothing to score by."""
    await fs.write_text("/a.md", "the who and the what")
    await fs.write_text("/b.md", "unrelated entirely")

    hits = await fs.search_text("the who")
    assert [h.path for h in hits] == ["/a.md"]
    assert hits[0].score > 0


async def test_the_snippet_centres_on_a_content_word(fs):
    """Not on "the", which would anchor the window at character zero."""
    await fs.write_text("/a.md", "the " * 60 + "kubernetes ingress " + "the " * 60)

    hit = (await fs.search_text("what about the kubernetes setup"))[0]
    assert "kubernetes" in hit.snippet


async def test_a_partial_query_still_finds_the_file(fs):
    """The default is `match="any"`: sharing one term is enough to come back.

    An agent told to search before it writes must not be told "nothing matches"
    by a note that is sitting right there under different words.
    """
    await fs.write_text("/voice.md", "Prefers terse replies, no preamble.")
    await fs.write_text("/cats.md", "completely unrelated text about cats")

    hits = await fs.search_text("what tone does the user prefer")
    assert [h.path for h in hits] == ["/voice.md"]
    # Wider matching, not a wider corpus: a file sharing no term stays out.
    assert await fs.search_text("kubernetes ingress") == []


async def test_match_all_is_available_for_a_precise_filter(fs):
    await fs.write_text("/voice.md", "Prefers terse replies, no preamble.")

    assert [h.path for h in await fs.search_text("terse replies", match="all")] == [
        "/voice.md"
    ]
    # "tone" and "user" are absent, so requiring every term misses entirely.
    assert await fs.search_text("what tone does the user prefer", match="all") == []


async def test_match_passes_through_search(fs):
    await fs.write_text("/voice.md", "Prefers terse replies, no preamble.")
    query = "which tone is preferred"

    assert [h.path for h in await fs.search(query)] == ["/voice.md"]
    assert await fs.search(query, match="all") == []


# A small corpus shaped like real agent memory, with one right answer per query.
# The point is not any single ranking but the aggregate: this is the regression
# guard on retrieval quality, and it runs against SurrealDB's own analyzer rather
# than a hand-rolled approximation of it.
_CORPUS = {
    "/preferences/voice.md": (
        "Prefers terse replies with no preamble. Never restate the question."
    ),
    "/preferences/workflow.md": (
        "Wants tests run before any commit. Prefers small pull requests "
        "reviewed quickly."
    ),
    "/projects/acme/notes.md": (
        "The acme user dashboard redesign. The user research showed users want "
        "fewer clicks. The user testing round is next, and the user panel meets Friday."
    ),
    "/projects/acme/todo.md": (
        "Ship the dashboard redesign. Write the migration. Ask about invoicing "
        "the extra scope."
    ),
    "/projects/surrealfs/todo.md": (
        "Add the hermes memory provider. Fix search ranking. Write the docs."
    ),
    "/memory/2026-08-01/s1.md": (
        "## User\nhow do I get paid for the consulting work\n"
        "## Assistant\nYou invoice monthly through the client portal, net thirty."
    ),
    "/memory/2026-08-02/s2.md": (
        "## User\nkeep answers short please\n"
        "## Assistant\nUnderstood, I will keep the tone brief and skip preamble."
    ),
    "/notes/music.md": "The tone of the guitar was warm and round.",
    "/notes/k8s.md": "Kubernetes ingress controllers and TLS termination notes.",
    "/home/hermes/todo.md": "Follow up on the invoice. Check the dashboard deploy.",
}

_QUERIES = [
    ("what tone does the user prefer", "/preferences/voice.md"),
    ("how do I get paid for consulting", "/memory/2026-08-01/s1.md"),
    ("should I run tests before committing", "/preferences/workflow.md"),
    ("dashboard redesign status", "/projects/acme/todo.md"),
    ("invoicing", "/memory/2026-08-01/s1.md"),
    ("what did the user say about preamble", "/memory/2026-08-02/s2.md"),
    ("terse", "/preferences/voice.md"),
    ("hermes memory provider work left", "/projects/surrealfs/todo.md"),
]


async def test_ranking_quality_over_a_realistic_corpus(fs):
    """Mean reciprocal rank of the wanted file across eight queries.

    The raw term-count scorer this replaced managed 0.674 and buried the answer
    to "what tone does the user prefer" at rank 7 of 8. The floor here is 0.75:
    high enough to catch that regression returning, low enough not to break over
    one query changing places.
    """
    for path, content in _CORPUS.items():
        await fs.write_text(path, content)

    reciprocals, report = [], []
    for query, wanted in _QUERIES:
        paths = [hit.path for hit in await fs.search(query, limit=len(_CORPUS))]
        rank = paths.index(wanted) + 1 if wanted in paths else 0
        reciprocals.append(1 / rank if rank else 0.0)
        report.append(f"  {rank or '-'}  {query!r} -> {wanted}")

    mrr = sum(reciprocals) / len(reciprocals)
    detail = "\n".join(report)
    assert mrr >= 0.75, f"retrieval got worse (MRR {mrr:.3f}):\n{detail}"


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
