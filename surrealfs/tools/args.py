"""Argument models for the SurrealFS tools.

These are the single source of truth for every tool's JSON Schema: the
pydantic-ai toolset and the raw JSON tool definitions both derive from
``model_json_schema()``, so the two surfaces cannot drift apart.
"""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict, Field

__all__ = [
    "CatArgs",
    "CpArgs",
    "EditArgs",
    "GlobArgs",
    "LsArgs",
    "MkdirArgs",
    "MvArgs",
    "ReadBytesArgs",
    "RmArgs",
    "SearchSemanticArgs",
    "SearchTextArgs",
    "TailArgs",
    "TouchArgs",
    "WriteBytesArgs",
    "WriteFileArgs",
]


class _Args(BaseModel):
    model_config = ConfigDict(extra="forbid")


class LsArgs(_Args):
    path: str = Field("/", description="Folder to list. Absolute, defaults to /")
    recursive: bool = Field(False, description="Descend into subfolders")


class GlobArgs(_Args):
    pattern: str = Field(
        ..., description="Glob pattern, e.g. /notes/**/*.md", min_length=1
    )


class CatArgs(_Args):
    path: str = Field(..., description="Text file to read", min_length=1)


class ReadBytesArgs(_Args):
    path: str = Field(..., description="Binary file to read", min_length=1)


class TailArgs(_Args):
    path: str = Field(..., description="Text file to read", min_length=1)
    n: int = Field(10, description="Number of trailing lines", ge=1, le=10_000)


class WriteFileArgs(_Args):
    path: str = Field(..., description="Destination path", min_length=1)
    content: str = Field(..., description="Full text content to write")
    content_type: str | None = Field(
        None, description="Media type; inferred from the extension when omitted"
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


class TouchArgs(_Args):
    path: str = Field(..., description="File to create if missing", min_length=1)


class MkdirArgs(_Args):
    path: str = Field(..., description="Folder to create", min_length=1)
    parents: bool = Field(False, description="Create missing parent folders")


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


class SearchTextArgs(_Args):
    query: str = Field(..., description="Words to search for", min_length=1)
    limit: int = Field(20, description="Maximum results", ge=1, le=100)


class SearchSemanticArgs(_Args):
    query: str = Field(
        ..., description="What you are looking for, in plain language", min_length=1
    )
    limit: int = Field(10, description="Maximum results", ge=1, le=100)
