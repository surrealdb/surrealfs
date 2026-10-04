"""Retrieval evaluations: BM25, Section Search, Token Reduction, and Graph Expansion."""

from __future__ import annotations

import pytest

from surrealfs import SurrealFs

# Realistic multi-topic corpus for agent retrieval evaluation
_AGENT_CORPUS = {
    "/docs/architecture.md": (
        "# System Architecture\n\n"
        "## Overview\n"
        "The system is built on a distributed reactive event store.\n\n"
        "## Authentication Pipeline\n"
        "All incoming requests pass through JWT validation and "
        "record-auth principals.\n"
        "Token verification uses ed25519 signatures and checks "
        "tenant isolation boundaries.\n\n"
        "## Storage Engine\n"
        "Data persistence leverages LSM-trees and RocksDB storage backends.\n"
    ),
    "/docs/deployments.md": (
        "# Deployment Guidelines\n\n"
        "## Kubernetes Setup\n"
        "Deploy the cluster using the Helm chart with 3 replicas and ingress TLS.\n\n"
        "## Monitoring\n"
        "Prometheus scrapes metrics from /metrics every 15 seconds.\n"
    ),
    "/rfcs/0042-auth.md": (
        "# RFC 0042: High-Performance Authentication\n\n"
        "## Abstract\n"
        "Proposes migrating legacy sessions to ed25519 asymmetric token validation.\n"
        "Implemented by /docs/architecture.md and src/auth/jwt.rs.\n"
    ),
    "/memory/agent-notes.md": (
        "# Daily Notes\n\n"
        "## Incident 104\n"
        "Fixed token verification timeout during high load on ed25519 verifier.\n\n"
        "## Next Steps\n"
        "Review Helm deployment scripts for ingress TLS certificates.\n"
    ),
}

# Queries targeting specific knowledge
_EVAL_QUERIES = [
    ("ed25519 token verification", "/docs/architecture.md"),
    ("kubernetes helm chart replicas", "/docs/deployments.md"),
    ("asymmetric token validation RFC", "/rfcs/0042-auth.md"),
    ("token verification timeout incident", "/memory/agent-notes.md"),
]


@pytest.mark.asyncio
async def test_bm25_retrieval_mrr(fs: SurrealFs) -> None:
    """Evaluate Mean Reciprocal Rank (MRR) of full-text retrieval on agent corpus."""
    for path, content in _AGENT_CORPUS.items():
        await fs.write_text(path, content)

    reciprocals = []
    for query, expected_path in _EVAL_QUERIES:
        hits = await fs.search_text(query, limit=10)
        hit_paths = [h.path for h in hits]
        rank = hit_paths.index(expected_path) + 1 if expected_path in hit_paths else 0
        reciprocals.append(1.0 / rank if rank else 0.0)

    mrr = sum(reciprocals) / len(reciprocals)
    # Target MRR >= 0.75
    assert mrr >= 0.75, f"BM25 retrieval MRR below threshold: {mrr:.3f}"


@pytest.mark.asyncio
async def test_section_retrieval_token_reduction(fs: SurrealFs) -> None:
    """Evaluate token/character payload reduction when retrieving precise sections

    instead of full documents.
    """
    for path, content in _AGENT_CORPUS.items():
        await fs.write_text(path, content)

    # Ingest synthetic mock embeddings for sections
    async def mock_embed(text: str) -> list[float]:
        return [0.1] * 1536

    await fs.reindex_embeddings(mock_embed, version="mock-v1")

    # Query for sections with the synthetic vector
    section_hits = await fs.search_sections([0.1] * 1536, limit=10)
    assert len(section_hits) > 0

    # Compare character size of a section hit from a multi-section document
    # vs full document
    hit = next(
        (h for h in section_hits if h.path == "/docs/architecture.md"),
        section_hits[0],
    )
    full_content = _AGENT_CORPUS.get(hit.path, "")
    section_content = hit.content

    reduction_pct = (1.0 - (len(section_content) / len(full_content))) * 100.0
    # Section retrieval should yield significant token reduction (> 40%)
    assert reduction_pct >= 40.0, (
        f"Section token reduction {reduction_pct:.1f}% was lower than expected"
    )


@pytest.mark.asyncio
async def test_graph_expanded_retrieval(fs: SurrealFs) -> None:
    """Evaluate graph expansion where a query finds an implementing document

    and relation traversal uncovers the referenced RFC.
    """
    for path, content in _AGENT_CORPUS.items():
        await fs.write_text(path, content)

    # Establish graph edge: /docs/architecture.md implements /rfcs/0042-auth.md
    await fs.relate("/docs/architecture.md", "implements", "/rfcs/0042-auth.md")

    # Direct query for "ed25519 signatures" matches /docs/architecture.md
    hits = await fs.search_text("ed25519 signatures", limit=1)
    assert len(hits) > 0
    top_path = hits[0].path
    assert top_path == "/docs/architecture.md"

    # Expand 1-hop dependencies via graph
    neighbors = await fs.get_neighbors(top_path, relation="implements")
    assert len(neighbors) == 1
    assert neighbors[0].target_path == "/rfcs/0042-auth.md"

    # Verify reverse traversal via backlinks
    backlinks = await fs.backlinks("/rfcs/0042-auth.md")
    assert any(b.source_path == "/docs/architecture.md" for b in backlinks)
