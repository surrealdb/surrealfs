"""Structured results returned by :class:`surrealfs.SurrealFs`.

The core API returns these dataclasses rather than pre-formatted strings; the
integration layers in :mod:`surrealfs.tools` turn them into text for a model.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any

from surrealdb import RecordID

__all__ = ["FileEntry", "SearchHit", "format_mode"]

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
    content: str | None = None
    data: bytes | None = None
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
            owner=row.get("owner") or "root",
            # No mode means a row older than the permissions schema, not a
            # permissive one: it reads as no bits at all, so only root sees it
            # until `docs/migrate_permissions.surql` has run. Mirrors the `?? 0` in
            # `fn::sfs_bits`.
            mode=int(row["mode"]) if row.get("mode") is not None else 0,
            gate=row.get("gate"),
            size=int(size or 0),
            hash=row.get("hash") or "",
            content=content,
            data=data,
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
