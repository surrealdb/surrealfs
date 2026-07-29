"""Tool implementations: turn structured results into text for a model.

Every handler takes a :class:`ToolContext` and a validated arguments model and
returns a string. Both integration surfaces call these, so a model sees exactly
the same output whichever one is wired up.
"""

from __future__ import annotations

import base64
import binascii
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass

from ..errors import SurrealFsError
from ..fs import SurrealFs
from ..models import FileEntry
from .args import (
    CatArgs,
    CpArgs,
    EditArgs,
    GlobArgs,
    LsArgs,
    MkdirArgs,
    MvArgs,
    ReadBytesArgs,
    RmArgs,
    SearchSemanticArgs,
    SearchTextArgs,
    TailArgs,
    TouchArgs,
    WriteBytesArgs,
    WriteFileArgs,
)

__all__ = ["ToolContext"]

Embedder = Callable[[str], Awaitable[Sequence[float]]]


@dataclass
class ToolContext:
    """What the tools need at call time.

    Args:
        fs: The filesystem to operate on.
        embed: Turns a query string into a vector. Required only by
            ``search_semantic``; when omitted that tool is not offered.
    """

    fs: SurrealFs
    embed: Embedder | None = None


def _format_entry(entry: FileEntry) -> str:
    if entry.is_folder:
        return f"{'-':>8}  {entry.path}/"
    return f"{entry.size:>8}  {entry.path}"


def _format_listing(entries: Sequence[FileEntry]) -> str:
    if not entries:
        return "(empty)"
    return "\n".join(_format_entry(e) for e in entries)


async def ls(ctx: ToolContext, args: LsArgs) -> str:
    return _format_listing(await ctx.fs.ls(args.path, recursive=args.recursive))


async def glob(ctx: ToolContext, args: GlobArgs) -> str:
    entries = await ctx.fs.glob(args.pattern)
    if not entries:
        return f"No files match {args.pattern}"
    return _format_listing(entries)


async def cat(ctx: ToolContext, args: CatArgs) -> str:
    content = await ctx.fs.read_text(args.path)
    return content if content else "(empty file)"


async def read_bytes(ctx: ToolContext, args: ReadBytesArgs) -> str:
    data = await ctx.fs.read_bytes(args.path)
    return base64.b64encode(data).decode("ascii")


async def tail(ctx: ToolContext, args: TailArgs) -> str:
    return await ctx.fs.tail(args.path, args.n) or "(empty file)"


async def write_file(ctx: ToolContext, args: WriteFileArgs) -> str:
    entry = await ctx.fs.write_text(
        args.path, args.content, content_type=args.content_type
    )
    return f"Wrote {entry.size} bytes to {entry.path}"


async def write_bytes(ctx: ToolContext, args: WriteBytesArgs) -> str:
    try:
        data = base64.b64decode(args.data_base64, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise SurrealFsError(f"data_base64 is not valid base64: {exc}") from exc
    entry = await ctx.fs.write_bytes(args.path, data, content_type=args.content_type)
    return f"Wrote {entry.size} bytes to {entry.path}"


async def edit(ctx: ToolContext, args: EditArgs) -> str:
    diff = await ctx.fs.edit(
        args.path, args.old, args.new, replace_all=args.replace_all
    )
    return f"Edited {args.path}\n\n{diff}"


async def touch(ctx: ToolContext, args: TouchArgs) -> str:
    entry = await ctx.fs.touch(args.path)
    return f"Ready: {entry.path}"


async def mkdir(ctx: ToolContext, args: MkdirArgs) -> str:
    entry = await ctx.fs.mkdir(args.path, parents=args.parents)
    return f"Created folder {entry.path}"


async def cp(ctx: ToolContext, args: CpArgs) -> str:
    entry = await ctx.fs.cp(args.src, args.dst, recursive=args.recursive)
    return f"Copied {args.src} to {entry.path}"


async def mv(ctx: ToolContext, args: MvArgs) -> str:
    entry = await ctx.fs.mv(args.src, args.dst)
    return f"Moved {args.src} to {entry.path}"


async def rm(ctx: ToolContext, args: RmArgs) -> str:
    count = await ctx.fs.rm(args.path, recursive=args.recursive)
    noun = "item" if count == 1 else "items"
    return f"Deleted {count} {noun} at {args.path}"


async def search_text(ctx: ToolContext, args: SearchTextArgs) -> str:
    hits = await ctx.fs.search_text(args.query, limit=args.limit)
    if not hits:
        return f"No files contain {args.query!r}"
    return "\n".join(f"{h.path}\n    {h.snippet}" for h in hits)


async def search_semantic(ctx: ToolContext, args: SearchSemanticArgs) -> str:
    if ctx.embed is None:
        raise SurrealFsError("Semantic search is not configured on this filesystem.")
    vector = await ctx.embed(args.query)
    hits = await ctx.fs.search_semantic(vector, k=args.limit)
    if not hits:
        return f"Nothing related to {args.query!r} has been indexed"
    return "\n".join(f"{h.path} (distance {h.score:.4f})" for h in hits)
