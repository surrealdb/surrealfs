"""Argument models for the SurrealFS tools.

These are the single source of truth for every tool's JSON Schema: the
pydantic-ai toolset, the raw JSON tool definitions, and the Hermes plugin all
derive from ``model_json_schema()``, so the surfaces cannot drift apart.
"""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict, Field

__all__ = [
    "AppendArgs",
    "BacklinksArgs",
    "CatArgs",
    "ChmodArgs",
    "CpArgs",
    "DiffArgs",
    "EditArgs",
    "GlobArgs",
    "GrepArgs",
    "HeadArgs",
    "HistoryArgs",
    "LsArgs",
    "MkdirArgs",
    "MvArgs",
    "ReadBytesArgs",
    "ReadRangeArgs",
    "RelateArgs",
    "RestoreArgs",
    "RmArgs",
    "SearchArgs",
    "TailArgs",
    "TouchArgs",
    "TreeArgs",
    "UndeleteArgs",
    "WriteBytesArgs",
    "WriteFileArgs",
]


class _Args(BaseModel):
    model_config = ConfigDict(extra="forbid")


class LsArgs(_Args):
    path: str = Field("/", description="Folder to list. Absolute, defaults to /")
    recursive: bool = Field(False, description="Descend into subfolders")
    long: bool = Field(False, description="Also show mode and owner, like `ls -l`")


class GlobArgs(_Args):
    pattern: str = Field(
        ..., description="Glob pattern, e.g. /notes/**/*.md", min_length=1
    )


class CatArgs(_Args):
    path: str = Field(..., description="Text file to read", min_length=1)


class HeadArgs(_Args):
    path: str = Field(..., description="Text file to read", min_length=1)
    n: int = Field(10, description="Number of leading lines", ge=1, le=10_000)


class ReadBytesArgs(_Args):
    path: str = Field(..., description="Binary file to read", min_length=1)


class TailArgs(_Args):
    path: str = Field(..., description="Text file to read", min_length=1)
    n: int = Field(10, description="Number of trailing lines", ge=1, le=10_000)


class ReadRangeArgs(_Args):
    path: str = Field(..., description="Text file to read", min_length=1)
    start: int = Field(1, description="1-indexed starting line number", ge=1)
    end: int = Field(
        -1,
        description="1-indexed ending line number (inclusive), or -1 for end of file",
    )
    numbers: bool = Field(False, description="Include line numbers in output")


class WriteFileArgs(_Args):
    path: str = Field(..., description="Destination path", min_length=1)
    content: str = Field(..., description="Full text content to write")
    content_type: str | None = Field(
        None, description="Media type; inferred from the extension when omitted"
    )
    if_generation: int | None = Field(
        None, description="Expected generation counter for optimistic concurrency"
    )


class AppendArgs(_Args):
    path: str = Field(..., description="Destination path", min_length=1)
    content: str = Field(..., description="Text content to append")
    if_generation: int | None = Field(
        None, description="Expected generation counter for optimistic concurrency"
    )


class WriteBytesArgs(_Args):
    path: str = Field(..., description="Destination path", min_length=1)
    data_base64: str = Field(..., description="Base64-encoded file contents")
    content_type: str = Field(
        "application/octet-stream", description="Media type, e.g. image/png"
    )


class EditArgs(_Args):
    path: str = Field(..., description="File to edit", min_length=1)
    old: str = Field(..., description="Exact text to find", min_length=1)
    new: str = Field(..., description="Replacement text")
    replace_all: bool = Field(
        False, description="Replace every occurrence instead of only the first"
    )
    if_generation: int | None = Field(
        None, description="Expected generation counter for optimistic concurrency"
    )


class TouchArgs(_Args):
    path: str = Field(..., description="File to create if missing", min_length=1)


class MkdirArgs(_Args):
    path: str = Field(..., description="Folder to create", min_length=1)
    parents: bool = Field(False, description="Create missing parent folders")
    exist_ok: bool = Field(False, description="Do not error if folder already exists")


class CpArgs(_Args):
    src: str = Field(..., description="Path to copy from", min_length=1)
    dst: str = Field(..., description="Path to copy to", min_length=1)
    recursive: bool = Field(False, description="Copy a folder and its contents")


class MvArgs(_Args):
    src: str = Field(..., description="Path to move from", min_length=1)
    dst: str = Field(..., description="Path to move to", min_length=1)


class RmArgs(_Args):
    path: str = Field(..., description="Path to delete", min_length=1)
    recursive: bool = Field(False, description="Delete a folder and its contents")


class ChmodArgs(_Args):
    path: str = Field(..., description="Path to change", min_length=1)
    mode: str = Field(
        ...,
        description=(
            "Three octal digits, as in `chmod`: owner, group, other. "
            "700 = private to you, 777 = shared with everyone, "
            "744 = everyone can read, only you can write"
        ),
        pattern=r"^[0-7]{3}$",
    )
    recursive: bool = Field(
        False, description="Apply to a folder and everything inside it"
    )


class SearchArgs(_Args):
    query: str = Field(
        ...,
        description="What you are looking for, in words or plain language",
        min_length=1,
    )
    limit: int = Field(20, description="Maximum results", ge=1, le=100)


class GrepArgs(_Args):
    pattern: str = Field(
        ...,
        description="Text pattern or regular expression to search for",
        min_length=1,
    )
    path: str | None = Field(
        None, description="Optional path prefix to restrict the search"
    )
    glob: str | None = Field(
        None, description="Optional glob filter on file paths (e.g. *.py)"
    )
    limit: int = Field(
        100, description="Maximum matching lines to return", ge=1, le=1000
    )
    is_regex: bool = Field(False, description="Treat pattern as a regular expression")
    case_sensitive: bool = Field(True, description="Perform case-sensitive matching")


class TreeArgs(_Args):
    path: str = Field("/", description="Folder to visualize. Absolute, defaults to /")
    max_depth: int = Field(
        3, description="Maximum directory depth to traverse", ge=1, le=10
    )


class HistoryArgs(_Args):
    path: str = Field(
        ..., description="File to inspect revision history for", min_length=1
    )
    limit: int = Field(
        20, description="Maximum number of historical entries to return", ge=1, le=100
    )


class DiffArgs(_Args):
    path: str = Field(..., description="File to diff", min_length=1)
    from_generation: int = Field(
        ..., description="Baseline generation number to compare from", ge=1
    )
    to_generation: int | None = Field(
        None,
        description="Target generation number to compare to (defaults to current)",
    )


class RestoreArgs(_Args):
    path: str = Field(..., description="File to restore", min_length=1)
    generation: int = Field(
        ..., description="Historical generation number to restore", ge=1
    )


class UndeleteArgs(_Args):
    path: str = Field(
        ..., description="Path of the deleted file to recover", min_length=1
    )


class RelateArgs(_Args):
    from_path: str = Field(..., description="Source file path", min_length=1)
    relation: str = Field(
        ...,
        description=(
            "Relation type: 'references', 'supersedes', 'derives_from', "
            "'implements', or 'links_to'"
        ),
        pattern=r"^(references|supersedes|derives_from|implements|links_to)$",
    )
    to_path: str = Field(..., description="Target file path", min_length=1)


class BacklinksArgs(_Args):
    path: str = Field(
        ..., description="File path to find incoming references for", min_length=1
    )


class SearchSectionsArgs(_Args):
    query: str = Field(
        ...,
        description="Query terms or keywords to match within document sections",
        min_length=1,
    )
    path: str | None = Field(
        None,
        description=("Optional path prefix or markdown file to scope section search"),
    )
    limit: int = Field(
        10, description="Maximum number of sections to return", ge=1, le=50
    )


class AcquireLeaseArgs(_Args):
    path: str = Field(..., description="File path to acquire a lease on", min_length=1)
    ttl_seconds: int = Field(60, description="Lease duration in seconds", ge=1, le=3600)
    reason: str | None = Field(
        None, description="Reason or task description for acquiring the lease"
    )


class ReleaseLeaseArgs(_Args):
    path: str = Field(
        ..., description="File path to release the lease from", min_length=1
    )


class ForkWorkspaceArgs(_Args):
    branch: str = Field(
        ...,
        description="Name for the new isolated workspace branch",
        min_length=1,
    )
    source_branch: str = Field(
        "main", description="Source branch or workspace to branch from"
    )


class MergeWorkspaceArgs(_Args):
    branch: str = Field(..., description="Workspace branch to merge", min_length=1)
    target_branch: str = Field("main", description="Target branch to merge into")
