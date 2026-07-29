"""Structured results returned by :class:`surrealfs.SurrealFs`.

The core API returns these dataclasses rather than pre-formatted strings; the
integration layers in :mod:`surrealfs.tools` turn them into text for a model.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any

from surrealdb import RecordID

__all__ = ["FileEntry", "SearchHit"]

FOLDER_CONTENT_TYPE = "inode/directory"


@dataclass(frozen=True, slots=True)
class FileEntry:
    """A row of the ``file`` table.

    ``content`` and ``data`` are only populated by the methods that fetch them
    (``read_text``/``read_bytes``); listing methods leave them ``None`` and set
    ``size`` instead.
    """

    id: RecordID
    path: str
    filename: str
    content_type: str
    is_folder: bool
    size: int = 0
    hash: str = ""
    content: str | None = None
    data: bytes | None = None
    created_at: datetime | None = None
    updated_at: datetime | None = None

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
            size=int(size or 0),
            hash=row.get("hash") or "",
            content=content,
            data=data,
            created_at=row.get("created_at"),
            updated_at=row.get("updated_at"),
        )


@dataclass(frozen=True, slots=True)
class SearchHit:
    """One result from :meth:`SurrealFs.search_text` or ``search_semantic``."""

    entry: FileEntry
    score: float
    snippet: str = ""

    @property
    def path(self) -> str:
        return self.entry.path
