"""Path normalisation and glob translation.

SurrealFS has no working directory: every path is absolute. A leading slash is
optional on input and always present on output.
"""

from __future__ import annotations

import re

from .errors import InvalidPath

__all__ = [
    "normalize",
    "split",
    "join",
    "parent_of",
    "basename",
    "glob_to_regex",
    "literal_prefix",
]


def normalize(path: str) -> str:
    """Return the canonical absolute form of ``path``.

    Adds a leading slash, collapses repeated slashes, drops ``.`` segments, and
    resolves ``..`` without ever escaping the root (``/../x`` becomes ``/x``,
    matching how a chroot behaves rather than raising).
    """
    if not isinstance(path, str):
        raise InvalidPath(f"path must be a string, got {type(path).__name__}")
    out: list[str] = []
    for segment in path.split("/"):
        if not segment or segment == ".":
            continue
        if segment == "..":
            if out:
                out.pop()
            continue
        out.append(segment)
    return "/" + "/".join(out)


def split(path: str) -> list[str]:
    """Normalised path as a list of segments. ``/`` yields ``[]``."""
    normalized = normalize(path)
    return [] if normalized == "/" else normalized[1:].split("/")


def join(*parts: str) -> str:
    return normalize("/".join(parts))


def parent_of(path: str) -> str:
    """Parent directory of ``path``. The parent of ``/`` is ``/``."""
    segments = split(path)
    return "/" + "/".join(segments[:-1]) if segments else "/"


def basename(path: str) -> str:
    """Final segment of ``path``. Empty for the root."""
    segments = split(path)
    return segments[-1] if segments else ""


def literal_prefix(pattern: str) -> str:
    """Longest wildcard-free directory prefix of a glob.

    Used to narrow a glob to a subtree before scanning, since the computed
    ``path`` field cannot be indexed. ``/notes/**/*.md`` -> ``/notes/``.
    """
    normalized = normalize(pattern)
    segments = normalized[1:].split("/") if normalized != "/" else []
    keep: list[str] = []
    for segment in segments:
        if any(ch in segment for ch in "*?["):
            break
        keep.append(segment)
    # Drop the last literal segment only if it is the whole pattern (a plain
    # path); otherwise every kept segment is a real ancestor directory.
    if len(keep) == len(segments):
        keep = keep[:-1]
    return "/" + "".join(f"{segment}/" for segment in keep)


def glob_to_regex(pattern: str) -> re.Pattern[str]:
    """Compile a shell-style glob against absolute paths.

    Supports ``*`` (within a segment), ``**`` (across segments), ``?``, and
    character classes ``[abc]`` / ``[!abc]``. This matches the semantics the
    previous Rust implementation got from ``globset``; ``fnmatch`` is not usable
    here because its ``*`` also crosses ``/``.
    """
    normalized = normalize(pattern)
    out: list[str] = ["^"]
    i = 0
    n = len(normalized)
    while i < n:
        ch = normalized[i]
        if ch == "*":
            if normalized.startswith("**", i):
                i += 2
                # "/**/" should also match zero directories, so consume the
                # trailing slash and make the whole group optional.
                if normalized.startswith("/", i):
                    i += 1
                    out.append("(?:[^/]+/)*")
                else:
                    out.append(".*")
            else:
                i += 1
                out.append("[^/]*")
        elif ch == "?":
            i += 1
            out.append("[^/]")
        elif ch == "[":
            end = i + 1
            if end < n and normalized[end] in "!^":
                end += 1
            if end < n and normalized[end] == "]":
                end += 1
            while end < n and normalized[end] != "]":
                end += 1
            if end >= n:  # unterminated class: treat as a literal bracket
                out.append(re.escape("["))
                i += 1
            else:
                body = normalized[i + 1 : end]
                if body.startswith(("!", "^")):
                    body = "^" + body[1:]
                out.append(f"[{body}]")
                i = end + 1
        else:
            out.append(re.escape(ch))
            i += 1
    out.append("$")
    return re.compile("".join(out))
