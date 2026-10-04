"""Tests for hierarchical AST/markdown chunking and section-level semantic search."""

from __future__ import annotations

import pytest

from surrealfs import ROOT, SurrealFs
from surrealfs.chunking import chunk_markdown, chunk_python


def test_chunk_markdown_hierarchy() -> None:
    doc = """This is an overview paragraph.

# Chapter 1: Introduction
Welcome to chapter 1.

## Section 1.1: Getting Started
Here is some start info.

### Subsection 1.1.1: Details
Fine-grained details here.

# Chapter 2: Advanced
Advanced content.
"""
    sections = chunk_markdown(doc)
    assert len(sections) == 5

    assert sections[0].heading == "Overview"
    assert sections[0].line_start == 1

    assert sections[1].heading == "Chapter 1: Introduction"
    assert "Welcome to chapter 1." in sections[1].content

    assert sections[2].heading == (
        "Chapter 1: Introduction > Section 1.1: Getting Started"
    )
    assert sections[3].heading == (
        "Chapter 1: Introduction > Section 1.1: Getting Started > "
        "Subsection 1.1.1: Details"
    )

    assert sections[4].heading == "Chapter 2: Advanced"


def test_chunk_python_functions_and_classes() -> None:
    code = '''"""Module docstring."""
import os

def greet(name: str) -> str:
    """Return greeting."""
    return f"Hello {name}"

class Calculator:
    def add(self, a: int, b: int) -> int:
        return a + b
'''
    sections = chunk_python(code)
    assert len(sections) >= 3

    assert sections[0].heading == "Imports & Module Docs"
    assert sections[1].heading == "def greet"
    assert sections[2].heading == "class Calculator"


@pytest.mark.asyncio
async def test_reindex_sections_and_search(db) -> None:
    fs_root = SurrealFs(db, user=ROOT)
    fs_alice = SurrealFs(db, user="alice")

    doc = """# Authentication Guide
Overview of auth.

## JWT Tokens
JWT token verification details.

## API Keys
API key verification details.
"""
    await fs_alice.write_text("/docs/auth.md", doc)

    # Deterministic mock embedder: 1536 floats
    async def mock_embed(text: str) -> list[float]:
        val = 0.5 if "JWT" in text else 0.1
        vec = [0.0] * 1536
        vec[0] = val
        return vec

    indexed = await fs_root.reindex_embeddings(mock_embed, version="test:v1")
    assert indexed == 1

    # Query sections with a vector close to JWT
    qvec = [0.0] * 1536
    qvec[0] = 0.5
    hits = await fs_alice.search_sections(qvec, limit=5)
    assert len(hits) >= 1
    best = hits[0]
    assert best.path == "/docs/auth.md"
    assert "JWT Tokens" in best.heading
    assert best.line_start > 1
    assert "JWT token verification" in best.content
