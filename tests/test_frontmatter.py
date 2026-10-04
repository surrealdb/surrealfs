"""Tests for YAML/TOML frontmatter parsing, link extraction, and metadata querying."""

from __future__ import annotations

import pytest

from surrealfs import (
    SurrealFs,
    extract_markdown_links,
    parse_frontmatter,
    resolve_link,
)


def test_parse_yaml_frontmatter() -> None:
    text = """---
title: Authentication Spec
status: open
priority: 1
verified: true
tags:
  - auth
  - okta
  - security
---
# Authentication Spec
Details go here.
"""
    meta, body = parse_frontmatter(text)
    assert meta is not None
    assert meta["title"] == "Authentication Spec"
    assert meta["status"] == "open"
    assert meta["priority"] == 1
    assert meta["verified"] is True
    assert meta["tags"] == ["auth", "okta", "security"]
    assert body.strip() == "# Authentication Spec\nDetails go here."


def test_parse_toml_frontmatter() -> None:
    text = """+++
title = "Config"
version = 2
enabled = false
tags = ["core", "infra"]
+++
Content of file.
"""
    meta, body = parse_frontmatter(text)
    assert meta is not None
    assert meta["title"] == "Config"
    assert meta["version"] == 2
    assert meta["enabled"] is False
    assert meta["tags"] == ["core", "infra"]
    assert body.strip() == "Content of file."


def test_parse_no_frontmatter() -> None:
    text = "Just plain markdown content without frontmatter."
    meta, body = parse_frontmatter(text)
    assert meta is None
    assert body == text


def test_extract_markdown_links() -> None:
    text = """
Check out [Architecture Doc](/docs/arch.md) and [RFC 42](../rfcs/0042.md#section-1).
Also see [[concepts/auth]] and [[glossary|Domain Glossary]].
Ignore external [Google](https://google.com) and [Email](mailto:user@example.com).
"""
    links = extract_markdown_links(text)
    assert "/docs/arch.md" in links
    assert "../rfcs/0042.md" in links
    assert "concepts/auth" in links
    assert "glossary" in links
    assert not any("google.com" in link for link in links)
    assert not any("mailto" in link for link in links)


def test_resolve_link() -> None:
    assert resolve_link("/docs/auth/jwt.md", "/rfcs/0042.md") == "/rfcs/0042.md"
    assert resolve_link("/docs/auth/jwt.md", "../rfcs/0042.md") == "/docs/rfcs/0042.md"
    assert resolve_link("/notes.md", "today.md") == "/today.md"


@pytest.mark.asyncio
async def test_frontmatter_stored_and_queried(fs: SurrealFs) -> None:
    content = """---
status: open
priority: 1
tags: [security, okta]
---
# Security Risk
Risk description.
"""
    await fs.write_text("/risks/risk1.md", content)
    entry = await fs.stat("/risks/risk1.md")
    assert entry.meta is not None
    assert entry.meta["status"] == "open"
    assert entry.meta["priority"] == 1
    assert entry.meta["tags"] == ["security", "okta"]

    # Test glob filtering with meta
    open_risks = await fs.glob("/risks/*.md", meta={"status": "open"})
    assert len(open_risks) == 1
    assert open_risks[0].path == "/risks/risk1.md"

    # List element membership filter
    okta_risks = await fs.glob("/risks/*.md", meta={"tags": "okta"})
    assert len(okta_risks) == 1

    closed_risks = await fs.glob("/risks/*.md", meta={"status": "closed"})
    assert len(closed_risks) == 0


@pytest.mark.asyncio
async def test_derived_links_and_backlinks(fs: SurrealFs) -> None:
    await fs.write_text("/rfcs/0042.md", "# RFC 0042\nSpec details.")
    doc_content = """# Auth Spec
Implements [RFC 0042](/rfcs/0042.md).
"""
    await fs.write_text("/docs/auth.md", doc_content)

    # Backlinks for /rfcs/0042.md should find /docs/auth.md
    bl = await fs.backlinks("/rfcs/0042.md")
    assert any(b.source_path == "/docs/auth.md" for b in bl)


@pytest.mark.asyncio
async def test_broken_links_detection(fs: SurrealFs) -> None:
    # Target does not exist yet
    await fs.write_text("/notes/plan.md", "See [Missing](/missing/file.md).")
    broken = await fs.find_broken_links("/notes")
    assert len(broken) == 1
    assert broken[0]["source"] == "/notes/plan.md"
    assert broken[0]["target"] == "/missing/file.md"

    # Now create target and verify broken links is empty
    await fs.write_text("/missing/file.md", "Here now.")
    broken_after = await fs.find_broken_links("/notes")
    assert len(broken_after) == 0
