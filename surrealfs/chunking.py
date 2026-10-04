"""Hierarchical AST and markdown chunking for SurrealFS.

Splits files into semantic sections (functions, classes, markdown headings)
with line ranges and content hashes for the `file_section` table.
"""

from __future__ import annotations

import ast
import hashlib
import re
from dataclasses import dataclass

__all__ = ["FileSection", "chunk_lines", "chunk_markdown", "chunk_python", "chunk_text"]


@dataclass(frozen=True, slots=True)
class FileSection:
    """A chunked section of a file."""

    section_idx: int
    heading: str
    line_start: int
    line_end: int
    content: str
    source_hash: str


def _md5(text: str) -> str:
    return hashlib.md5(text.encode("utf-8")).hexdigest()


def chunk_markdown(text: str) -> list[FileSection]:
    """Split markdown text into sections along heading boundaries (#, ##, ###)."""
    if not text.strip():
        return []

    lines = text.splitlines(keepends=True)
    heading_re = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")

    # Find heading lines: (line_idx_0, level, title)
    headings: list[tuple[int, int, str]] = []
    for idx, line in enumerate(lines):
        m = heading_re.match(line)
        if m:
            headings.append((idx, len(m.group(1)), m.group(2).strip()))

    if not headings:
        return [
            FileSection(
                section_idx=0,
                heading="",
                line_start=1,
                line_end=len(lines),
                content=text,
                source_hash=_md5(text),
            )
        ]

    sections: list[FileSection] = []
    # If there's content before the first heading, treat it as introduction/preamble
    first_h_line = headings[0][0]
    if first_h_line > 0:
        preamble = "".join(lines[:first_h_line])
        if preamble.strip():
            sections.append(
                FileSection(
                    section_idx=len(sections),
                    heading="Overview",
                    line_start=1,
                    line_end=first_h_line,
                    content=preamble,
                    source_hash=_md5(preamble),
                )
            )

    # Heading hierarchy stack: [(level, title)]
    stack: list[tuple[int, str]] = []
    for i, (line_idx, level, title) in enumerate(headings):
        next_line_idx = headings[i + 1][0] if i + 1 < len(headings) else len(lines)
        section_lines = lines[line_idx:next_line_idx]
        section_text = "".join(section_lines)

        # Maintain heading hierarchy stack
        while stack and stack[-1][0] >= level:
            stack.pop()
        stack.append((level, title))
        heading_path = " > ".join(t for _, t in stack)

        sections.append(
            FileSection(
                section_idx=len(sections),
                heading=heading_path,
                line_start=line_idx + 1,
                line_end=next_line_idx,
                content=section_text,
                source_hash=_md5(section_text),
            )
        )

    return sections


def chunk_python(text: str) -> list[FileSection]:
    """Parse python code using AST into classes, methods, and functions."""
    if not text.strip():
        return []

    lines = text.splitlines(keepends=True)
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return chunk_lines(text)

    # Collect top-level functions and classes
    nodes = []
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            nodes.append(node)

    if not nodes:
        return [
            FileSection(
                section_idx=0,
                heading="Module",
                line_start=1,
                line_end=len(lines),
                content=text,
                source_hash=_md5(text),
            )
        ]

    sections: list[FileSection] = []

    # Check for preamble/imports before first node
    first_node_line = nodes[0].lineno
    if first_node_line > 1:
        preamble = "".join(lines[: first_node_line - 1])
        if preamble.strip():
            sections.append(
                FileSection(
                    section_idx=len(sections),
                    heading="Imports & Module Docs",
                    line_start=1,
                    line_end=first_node_line - 1,
                    content=preamble,
                    source_hash=_md5(preamble),
                )
            )

    for node in nodes:
        start_line = node.lineno
        end_line = getattr(node, "end_lineno", start_line) or start_line
        kind = "class" if isinstance(node, ast.ClassDef) else "def"
        name = f"{kind} {node.name}"
        node_text = "".join(lines[start_line - 1 : end_line])
        sections.append(
            FileSection(
                section_idx=len(sections),
                heading=name,
                line_start=start_line,
                line_end=end_line,
                content=node_text,
                source_hash=_md5(node_text),
            )
        )

    return sections


def chunk_lines(
    text: str, window_size: int = 50, overlap: int = 10
) -> list[FileSection]:
    """Chunk text into sliding windows of lines with overlap."""
    if not text.strip():
        return []

    lines = text.splitlines(keepends=True)
    total_lines = len(lines)
    if total_lines <= window_size:
        return [
            FileSection(
                section_idx=0,
                heading=f"Lines 1-{total_lines}",
                line_start=1,
                line_end=total_lines,
                content=text,
                source_hash=_md5(text),
            )
        ]

    sections: list[FileSection] = []
    step = max(1, window_size - overlap)
    idx = 0
    start = 0
    while start < total_lines:
        end = min(total_lines, start + window_size)
        chunk = "".join(lines[start:end])
        sections.append(
            FileSection(
                section_idx=idx,
                heading=f"Lines {start + 1}-{end}",
                line_start=start + 1,
                line_end=end,
                content=chunk,
                source_hash=_md5(chunk),
            )
        )
        idx += 1
        if end >= total_lines:
            break
        start += step

    return sections


def chunk_text(
    text: str, filename: str = "", content_type: str = ""
) -> list[FileSection]:
    """Automatically select and execute the appropriate chunking strategy."""
    lower_fn = filename.lower()
    if (
        lower_fn.endswith((".md", ".markdown", ".mdown"))
        or content_type == "text/markdown"
    ):
        return chunk_markdown(text)
    if lower_fn.endswith(".py") or content_type in ("text/x-python", "text/python"):
        return chunk_python(text)

    lines = text.splitlines()
    if len(lines) <= 30:
        return [
            FileSection(
                section_idx=0,
                heading=filename or "Document",
                line_start=1,
                line_end=len(lines) or 1,
                content=text,
                source_hash=_md5(text),
            )
        ]
    return chunk_lines(text)
