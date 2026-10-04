"""Structured results returned by :class:`surrealfs.SurrealFs`.

The core API returns these dataclasses rather than pre-formatted strings; the
integration layers in :mod:`surrealfs.tools` turn them into text for a model.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any

from surrealdb import RecordID

__all__ = [
    "FileEntry",
    "FileVersionEntry",
    "GraphRelation",
    "GrepMatch",
    "SearchHit",
    "SectionHit",
    "WatchEvent",
    "WorkspaceEntry",
    "format_mode",
]

FOLDER_CONTENT_TYPE = "inode/directory"


def format_mode(mode: int, *, is_folder: bool = False) -> str:
    """Mode bits as ``ls -l`` writes them: ``drwxr-x---``."""
    out = ["d" if is_folder else "-"]
    for shift in (6, 3, 0):
        digit = (mode >> shift) & 7
        out.append("r" if digit & 4 else "-")
        out.append("w" if digit & 2 else "-")
        out.append("x" if digit & 1 else "-")
    return "".join(out)


@dataclass(frozen=True, slots=True)
class FileEntry:
    """A row of the ``file`` table.

    ``content`` and ``data`` are only populated by the methods that fetch them
    (``read_text``/``read_bytes``); listing methods leave them ``None`` and set
    ``size`` instead.

    ``gate`` is the schema's computed ancestry check: who may traverse down to
    this row -- ``None`` when every directory above it is world-traversable,
    otherwise the owner of the closed ones (see ``fn::sfs_gate``). It is what
    lets a permission check cost no extra queries.
    """

    id: RecordID
    path: str
    filename: str
    content_type: str
    is_folder: bool
    owner: str = "root"
    mode: int = 0o666
    gate: str | None = None
    size: int = 0
    hash: str = ""
    generation: int = 1
    content: str | None = None
    data: bytes | None = None
    meta: dict[str, Any] | None = None
    crdt: bool = False
    created_at: datetime | None = None
    updated_at: datetime | None = None

    @property
    def permissions(self) -> str:
        """Mode bits as ``ls -l`` writes them: ``drwxr-x---``."""
        return format_mode(self.mode, is_folder=self.is_folder)

    @property
    def is_binary(self) -> bool:
        return not self.is_folder and self.content is None and self.data is not None

    @classmethod
    def from_row(cls, row: dict[str, Any]) -> FileEntry:
        content = row.get("content")
        data = row.get("file")
        # `size` may be precomputed by the query (listings) or derived here.
        size = row.get("size")
        if size is None:
            size = len(content) if content is not None else len(data or b"")
        return cls(
            id=row["id"],
            path=row.get("path", ""),
            filename=row.get("filename", ""),
            content_type=row.get("content_type", ""),
            is_folder=bool(row.get("is_folder", False)),
            # Not coalesced: `owner` and `mode` have DEFAULTs, so every row
            # has both, and a row without one is a schema that did not apply.
            # Guessing a permission for it is how a wrong answer stays quiet --
            # `fn::sfs_bits` requires them for the same reason.
            owner=row["owner"],
            mode=int(row["mode"]),
            gate=row.get("gate"),
            size=int(size or 0),
            hash=row.get("hash") or "",
            generation=int(row.get("generation") or 1),
            content=content,
            data=data,
            meta=row.get("meta"),
            crdt=bool(row.get("crdt", False)),
            created_at=row.get("created_at"),
            updated_at=row.get("updated_at"),
        )


@dataclass(frozen=True, slots=True)
class SearchHit:
    """One result from :meth:`SurrealFs.search` or either arm it fuses."""

    entry: FileEntry
    score: float
    snippet: str = ""

    @property
    def path(self) -> str:
        return self.entry.path


@dataclass(frozen=True, slots=True)
class GrepMatch:
    """One line match from :meth:`SurrealFs.grep`."""

    path: str
    line_number: int
    line: str


@dataclass(frozen=True, slots=True)
class FileVersionEntry:
    """A historical snapshot of a file from ``file_version``."""

    id: RecordID | str
    file: RecordID | str
    generation: int
    path: str
    content_type: str = "text/plain"
    hash: str = ""
    author: str = "root"
    owner: str = "root"
    mode: int = 0o666
    op: str = "write"
    size: int = 0
    created_at: datetime | None = None

    @property
    def permissions(self) -> str:
        return format_mode(self.mode, is_folder=False)

    @classmethod
    def from_row(cls, row: dict[str, Any]) -> FileVersionEntry:
        return cls(
            id=row["id"],
            file=row.get("file", ""),
            generation=int(row.get("generation") or 1),
            path=row.get("path", ""),
            content_type=row.get("content_type", "text/plain"),
            hash=row.get("hash") or "",
            author=row.get("author") or "root",
            owner=row.get("owner") or "root",
            mode=int(row.get("mode") or 0o666),
            op=row.get("op") or "write",
            size=int(row.get("size") or 0),
            created_at=row.get("created_at"),
        )


@dataclass(frozen=True, slots=True)
class GraphRelation:
    """An edge between files in the knowledge graph."""

    source_path: str
    target_path: str
    relation: str
    source_name: str = ""
    target_name: str = ""

    @classmethod
    def from_row(cls, row: dict[str, Any]) -> GraphRelation:
        return cls(
            source_path=row.get("source_path") or "",
            target_path=row.get("target_path") or "",
            relation=row.get("relation") or "",
            source_name=row.get("source_name") or "",
            target_name=row.get("target_name") or "",
        )


@dataclass(frozen=True, slots=True)
class SectionHit:
    """A search hit targeting a specific section of a file."""

    path: str
    heading: str
    line_start: int
    line_end: int
    content: str
    score: float

    @property
    def lines(self) -> str:
        return f"{self.line_start}-{self.line_end}"

    @classmethod
    def from_row(cls, row: dict[str, Any]) -> SectionHit:
        return cls(
            path=row.get("path") or "",
            heading=row.get("heading") or "",
            line_start=int(row.get("line_start") or 1),
            line_end=int(row.get("line_end") or 1),
            content=row.get("content") or "",
            score=float(row.get("score") or 0.0),
        )


@dataclass(frozen=True, slots=True)
class WatchEvent:
    """A notification from a live query watch stream."""

    action: str  # "CREATE", "UPDATE", "DELETE"
    path: str
    entry: FileEntry | None = None


@dataclass(frozen=True, slots=True)
class WorkspaceEntry:
    """A row of the ``workspace`` table."""

    id: RecordID
    name: str
    owner: str = "root"
    is_public: bool = False
    created_at: datetime | None = None

    @classmethod
    def from_row(cls, row: dict[str, Any]) -> WorkspaceEntry:
        return cls(
            id=row["id"],
            name=row.get("name") or "",
            owner=row.get("owner") or "root",
            is_public=bool(row.get("is_public") or False),
            created_at=row.get("created_at"),
        )
