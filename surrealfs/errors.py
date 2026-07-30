"""Exceptions raised by :class:`surrealfs.SurrealFs`.

Each error subclasses both :class:`SurrealFsError` and the closest builtin OS
error, so callers can catch whichever fits their code:

    try:
        await fs.read_text("/missing.md")
    except FileNotFoundError:      # builtin
        ...
    except surrealfs.NotFound:     # library-specific
        ...
"""

from __future__ import annotations

__all__ = [
    "AlreadyExists",
    "DirectoryNotEmpty",
    "InvalidPath",
    "IsADirectory",
    "NotADirectory",
    "NotATextFile",
    "NotFound",
    "QueryError",
    "SurrealFsError",
]


class SurrealFsError(Exception):
    """Base class for every error raised by this library."""


class NotFound(SurrealFsError, FileNotFoundError):
    """No file or folder exists at the given path."""


class AlreadyExists(SurrealFsError, FileExistsError):
    """Something already exists at the destination path."""


class IsADirectory(SurrealFsError, IsADirectoryError):
    """The path names a folder but the operation requires a file."""


class NotADirectory(SurrealFsError, NotADirectoryError):
    """The path names a file but the operation requires a folder."""


class DirectoryNotEmpty(SurrealFsError, OSError):
    """Refusing to remove a non-empty folder without ``recursive=True``."""


class NotATextFile(SurrealFsError, ValueError):
    """The file holds binary content and cannot be read as text."""


class InvalidPath(SurrealFsError, ValueError):
    """The path is empty, malformed, or escapes the root."""


class QueryError(SurrealFsError, RuntimeError):
    """A SurrealQL statement returned an error status.

    Raised by ``SurrealFs._query``. The SDK's ``query()`` silently discards
    per-statement failures, so the library checks every statement itself.

    ``retryable`` marks conflicts the store says can simply be retried -- most
    often a write racing the schema's ``evt_file_hash`` event, which updates a
    row shortly after its content changes.
    """

    def __init__(self, message: str, *, retryable: bool = False) -> None:
        super().__init__(message)
        self.retryable = retryable
