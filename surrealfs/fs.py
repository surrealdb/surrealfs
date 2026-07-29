"""The SurrealFS core API.

:class:`SurrealFs` wraps an already-connected ``AsyncSurreal`` handle. It never
connects, signs in, or selects a namespace -- the caller owns the connection and
therefore the authentication context that the ``file`` table's ``owner =
$auth.id`` permissions key off.
"""

from __future__ import annotations

import asyncio
import difflib
import re
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

from surrealdb import RecordID

from . import paths
from .errors import (
    AlreadyExists,
    DirectoryNotEmpty,
    InvalidPath,
    IsADirectory,
    NotADirectory,
    NotATextFile,
    NotFound,
    QueryError,
)
from .models import FOLDER_CONTENT_TYPE, FileEntry, SearchHit

__all__ = ["SurrealFs", "raise_for_status"]

_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")

_MAX_RETRIES = 5
_RETRY_BACKOFF = 0.02  # seconds; doubles per attempt

# How the schema's `parent_key` field spells "no parent". Must match file.surql.
ROOT_KEY = "root"


def _parent_key(parent_id: RecordID | None) -> str:
    """The indexed key for a parent record id, matching the schema's VALUE clause."""
    return ROOT_KEY if parent_id is None else str(parent_id)


# Every field the model layer needs. `path` and `is_folder` are COMPUTED, so
# selecting them costs a parent-chain walk per row -- worth it, and the only way
# to get a path at all.
_FIELDS = (
    "id, filename, path, content_type, is_folder, hash, created_at, updated_at, "
    "IF content IS NOT NONE THEN string::len(content) "
    "ELSE IF file IS NOT NONE THEN bytes::len(file) "
    "ELSE 0 END AS size"
)
_FIELDS_WITH_CONTENT = f"{_FIELDS}, content, file"


def raise_for_status(raw: Any) -> list[Any]:
    """Validate a ``query_raw`` envelope and return each statement's result.

    We use ``query_raw`` rather than ``query`` because the envelope carries each
    statement's error *kind*, which is what tells us whether a failure is a
    retryable transaction conflict. It is also version-tolerant: on surrealdb-py
    2.x, ``query()`` returned only the first statement's result and silently
    discarded later failures (``LET $x = 1; THROW 'boom'`` returned ``None``).
    """
    results = raw.get("result", raw) if isinstance(raw, dict) else raw
    if not isinstance(results, list):
        return [results]
    for statement in results:
        if isinstance(statement, dict) and statement.get("status") == "ERR":
            message = str(statement.get("result", "unknown query error"))
            # The kind sits at the top level on some versions and under
            # `details` on others, so check both.
            kind = statement.get("kind") or (statement.get("details") or {}).get("kind")
            raise QueryError(
                message,
                retryable=kind == "TransactionConflict"
                or "retry the transaction" in message,
            )
    return [
        statement.get("result") if isinstance(statement, dict) else statement
        for statement in results
    ]


class SurrealFs:
    """A hierarchical filesystem stored in a SurrealDB ``file`` table.

    Args:
        db: A connected ``AsyncSurreal`` handle (signed in, namespace/database
            selected). Not owned by this object -- closing it is the caller's job.
        table: Table name, if you renamed it in the schema.
    """

    def __init__(self, db: Any, *, table: str = "file") -> None:
        if not _IDENT.match(table):
            raise InvalidPath(f"invalid table name: {table!r}")
        self.db = db
        self.table = table

    # ---------------------------------------------------------------- querying

    async def _query(self, sql: str, variables: dict[str, Any] | None = None) -> Any:
        """Run one or more statements and return the *last* statement's result.

        Retries transaction conflicts with a short backoff: the schema's
        ``evt_file_hash`` event updates a row just after its content changes, so
        writing twice in quick succession legitimately races it.
        """
        for attempt in range(_MAX_RETRIES):
            try:
                results = raise_for_status(
                    await self.db.query_raw(sql, variables or {})
                )
            except QueryError as exc:
                if not exc.retryable or attempt == _MAX_RETRIES - 1:
                    raise
                await asyncio.sleep(_RETRY_BACKOFF * (2**attempt))
                continue
            return results[-1] if results else None
        raise AssertionError("unreachable")  # pragma: no cover

    async def _resolve(self, path: str, *, fields: str = _FIELDS) -> FileEntry | None:
        """Look up a path in a single round trip.

        ``path`` is a COMPUTED field, so ``WHERE path = $p`` is a table scan that
        recomputes every row's ancestry. Instead this walks the segments through
        the ``(parent, filename)`` index, chaining the lookups with ``LET`` so
        the whole descent is one request.
        """
        segments = paths.split(path)
        if not segments:
            return None  # the root has no record of its own
        lines = [f"LET $k0 = '{ROOT_KEY}';"]
        variables: dict[str, Any] = {}
        for i, segment in enumerate(segments):
            variables[f"s{i}"] = segment
            if i < len(segments) - 1:
                lines.append(
                    f"LET $p{i + 1} = (SELECT VALUE id FROM ONLY {self.table} "
                    f"WHERE parent_key = $k{i} AND filename = $s{i} LIMIT 1);"
                )
                # A missing intermediate segment casts to the string "NONE",
                # which matches no row -- so the descent correctly dead-ends
                # instead of falling back to the root and matching the wrong file.
                lines.append(f"LET $k{i + 1} = <string>$p{i + 1};")
        last = len(segments) - 1
        lines.append(
            f"RETURN (SELECT {fields} FROM ONLY {self.table} "
            f"WHERE parent_key = $k{last} AND filename = $s{last} LIMIT 1);"
        )
        row = await self._query("\n".join(lines), variables)
        return FileEntry.from_row(row) if row else None

    async def _require(self, path: str, *, fields: str = _FIELDS) -> FileEntry:
        entry = await self._resolve(path, fields=fields)
        if entry is None:
            raise NotFound(f"No such file or directory: {paths.normalize(path)}")
        return entry

    async def _require_file(self, path: str, *, fields: str = _FIELDS) -> FileEntry:
        entry = await self._require(path, fields=fields)
        if entry.is_folder:
            raise IsADirectory(f"Is a directory: {entry.path}")
        return entry

    async def _parent_id(self, path: str) -> RecordID | None:
        """Record id of ``path``'s parent folder, or ``None`` if it is root."""
        parent_path = paths.parent_of(path)
        if parent_path == "/":
            return None
        parent = await self._resolve(parent_path)
        if parent is None:
            raise NotFound(f"No such file or directory: {parent_path}")
        if not parent.is_folder:
            raise NotADirectory(f"Not a directory: {parent.path}")
        return parent.id

    # -------------------------------------------------------------------- read

    async def read_text(self, path: str) -> str:
        """Full text content of a file."""
        entry = await self._require_file(path, fields=_FIELDS_WITH_CONTENT)
        if entry.content is None:
            raise NotATextFile(
                f"Not a text file ({entry.content_type}): {entry.path}. "
                "Use read_bytes() instead."
            )
        return entry.content

    async def read_bytes(self, path: str) -> bytes:
        """Raw bytes of a file, encoding text content as UTF-8 if needed."""
        entry = await self._require_file(path, fields=_FIELDS_WITH_CONTENT)
        if entry.data is not None:
            return entry.data
        return (entry.content or "").encode("utf-8")

    async def tail(self, path: str, n: int = 10) -> str:
        """Last ``n`` lines of a text file."""
        if n <= 0:
            raise ValueError("n must be positive")
        return "\n".join((await self.read_text(path)).splitlines()[-n:])

    async def exists(self, path: str) -> bool:
        if paths.normalize(path) == "/":
            return True
        return await self._resolve(path) is not None

    async def stat(self, path: str) -> FileEntry:
        """Metadata for a path, without fetching its content."""
        return await self._require(path)

    async def ls(self, path: str = "/", *, recursive: bool = False) -> list[FileEntry]:
        """List the contents of a folder.

        Uses the ``(parent, filename)`` index. Recursion is a breadth-first walk
        over the same indexed query rather than a scan of the computed path.
        """
        normalized = paths.normalize(path)
        if normalized == "/":
            root_id: RecordID | None = None
        else:
            folder = await self._require(normalized)
            if not folder.is_folder:
                raise NotADirectory(f"Not a directory: {folder.path}")
            root_id = folder.id

        out: list[FileEntry] = []
        frontier: list[str] = [_parent_key(root_id)]
        while frontier:
            rows = await self._query(
                f"SELECT {_FIELDS} FROM {self.table} "
                "WHERE parent_key IN $keys ORDER BY filename",
                {"keys": frontier},
            )
            entries = [FileEntry.from_row(row) for row in (rows or [])]
            out.extend(entries)
            if not recursive:
                break
            frontier = [_parent_key(e.id) for e in entries if e.is_folder]
        out.sort(key=lambda e: e.path)
        return out

    async def glob(self, pattern: str) -> list[FileEntry]:
        """Find files whose path matches a shell-style glob.

        Narrows to the pattern's literal directory prefix server-side, then
        applies the full pattern in Python -- the computed ``path`` field cannot
        be indexed, so the prefix is the only available filter.
        """
        prefix = paths.literal_prefix(pattern)
        regex = paths.glob_to_regex(pattern)
        rows = await self._query(
            f"SELECT {_FIELDS} FROM {self.table} "
            "WHERE string::starts_with(path, $prefix) ORDER BY path",
            {"prefix": prefix},
        )
        matches = [FileEntry.from_row(row) for row in (rows or [])]
        return [e for e in matches if regex.match(e.path)]

    # ------------------------------------------------------------------- write

    async def mkdir(self, path: str, *, parents: bool = False) -> FileEntry:
        """Create a folder. With ``parents``, create missing ancestors too."""
        segments = paths.split(path)
        if not segments:
            raise InvalidPath("cannot create the root directory")

        parent_id: RecordID | None = None
        created: FileEntry | None = None
        for i, segment in enumerate(segments):
            partial = "/" + "/".join(segments[: i + 1])
            existing = await self._resolve(partial)
            is_last = i == len(segments) - 1
            if existing is not None:
                if not existing.is_folder:
                    raise AlreadyExists(f"Not a directory: {partial}")
                if is_last:
                    raise AlreadyExists(f"File exists: {partial}")
                parent_id = existing.id
                continue
            if not is_last and not parents:
                raise NotFound(
                    f"No such file or directory: {partial}. "
                    "Pass parents=True to create it."
                )
            created = await self._create(segment, parent_id, FOLDER_CONTENT_TYPE)
            parent_id = created.id

        assert created is not None  # the last segment always exists or is created
        return created

    async def _create(
        self,
        filename: str,
        parent_id: RecordID | None,
        content_type: str,
        *,
        content: str | None = None,
        data: bytes | None = None,
    ) -> FileEntry:
        payload: dict[str, Any] = {
            "filename": filename,
            "parent": parent_id,
            "content_type": content_type,
        }
        if content is not None:
            payload["content"] = content
        if data is not None:
            payload["file"] = data
        rows = await self._query(
            f"CREATE {self.table} CONTENT $payload RETURN {_FIELDS}",
            {"payload": payload},
        )
        row = rows[0] if isinstance(rows, list) else rows
        return FileEntry.from_row(row)

    async def _ensure_parent(self, path: str, *, create: bool) -> RecordID | None:
        parent_path = paths.parent_of(path)
        if parent_path == "/":
            return None
        if create:
            existing = await self._resolve(parent_path)
            if existing is None:
                return (await self.mkdir(parent_path, parents=True)).id
            if not existing.is_folder:
                raise NotADirectory(f"Not a directory: {parent_path}")
            return existing.id
        return await self._parent_id(path)

    async def write_text(
        self,
        path: str,
        content: str,
        *,
        content_type: str | None = None,
        create_parents: bool = True,
    ) -> FileEntry:
        """Create or replace a text file. Returns the stored entry."""
        normalized = paths.normalize(path)
        filename = paths.basename(normalized)
        if not filename:
            raise InvalidPath("cannot write to the root directory")
        resolved_type = content_type or _sniff_content_type(filename, content)

        existing = await self._resolve(normalized)
        if existing is not None:
            if existing.is_folder:
                raise IsADirectory(f"Is a directory: {normalized}")
            rows = await self._query(
                f"UPDATE $id SET content = $content, file = NONE, "
                f"content_type = $content_type RETURN {_FIELDS}",
                {
                    "id": existing.id,
                    "content": content,
                    "content_type": resolved_type,
                },
            )
            row = rows[0] if isinstance(rows, list) else rows
            return FileEntry.from_row(row)

        parent_id = await self._ensure_parent(normalized, create=create_parents)
        return await self._create(filename, parent_id, resolved_type, content=content)

    async def write_bytes(
        self,
        path: str,
        data: bytes,
        *,
        content_type: str = "application/octet-stream",
        create_parents: bool = True,
    ) -> FileEntry:
        """Create or replace a binary file."""
        normalized = paths.normalize(path)
        filename = paths.basename(normalized)
        if not filename:
            raise InvalidPath("cannot write to the root directory")

        existing = await self._resolve(normalized)
        if existing is not None:
            if existing.is_folder:
                raise IsADirectory(f"Is a directory: {normalized}")
            rows = await self._query(
                f"UPDATE $id SET file = $data, content = NONE, "
                f"content_type = $content_type RETURN {_FIELDS}",
                {"id": existing.id, "data": data, "content_type": content_type},
            )
            row = rows[0] if isinstance(rows, list) else rows
            return FileEntry.from_row(row)

        parent_id = await self._ensure_parent(normalized, create=create_parents)
        return await self._create(filename, parent_id, content_type, data=data)

    async def touch(self, path: str, *, create_parents: bool = True) -> FileEntry:
        """Create an empty file if it does not exist; otherwise leave it alone.

        The new file gets ``content = ""`` rather than NONE -- the schema's
        ``is_folder`` is computed as "no content, no bytes, no symlink", so a
        NONE-content row would come back as a directory.
        """
        existing = await self._resolve(paths.normalize(path))
        if existing is not None:
            return existing
        return await self.write_text(
            path, "", content_type="text/plain", create_parents=create_parents
        )

    async def edit(
        self, path: str, old: str, new: str, *, replace_all: bool = False
    ) -> str:
        """Replace text in a file and return a unified diff of the change."""
        entry = await self._require_file(path, fields=_FIELDS_WITH_CONTENT)
        if entry.content is None:
            raise NotATextFile(f"Not a text file ({entry.content_type}): {entry.path}")
        if old == "":
            raise ValueError("`old` must not be empty")
        current = entry.content
        if old not in current:
            raise NotFound(f"Text not found in {entry.path}: {old!r}")
        updated = (
            current.replace(old, new) if replace_all else current.replace(old, new, 1)
        )
        await self._query(
            "UPDATE $id SET content = $content",
            {"id": entry.id, "content": updated},
        )
        return _unified_diff(current, updated, entry.path)

    # ------------------------------------------------------------ move / delete

    async def mv(self, src: str, dst: str) -> FileEntry:
        """Move or rename a file or folder.

        A single ``UPDATE``: ``path`` is computed from the parent chain, so every
        descendant's path follows automatically.
        """
        entry = await self._require(src)
        dst_normalized = paths.normalize(dst)
        filename = paths.basename(dst_normalized)
        if not filename:
            raise InvalidPath("cannot move onto the root directory")
        if dst_normalized == entry.path:
            return entry
        if dst_normalized.startswith(entry.path + "/"):
            raise InvalidPath(
                f"cannot move {entry.path} into its own subtree ({dst_normalized})"
            )
        if await self._resolve(dst_normalized) is not None:
            raise AlreadyExists(f"File exists: {dst_normalized}")

        parent_id = await self._parent_id(dst_normalized)
        rows = await self._query(
            f"UPDATE $id SET filename = $filename, parent = $parent RETURN {_FIELDS}",
            {"id": entry.id, "filename": filename, "parent": parent_id},
        )
        row = rows[0] if isinstance(rows, list) else rows
        return FileEntry.from_row(row)

    async def cp(self, src: str, dst: str, *, recursive: bool = False) -> FileEntry:
        """Copy a file, or a whole folder with ``recursive=True``."""
        entry = await self._require(src, fields=_FIELDS_WITH_CONTENT)
        dst_normalized = paths.normalize(dst)
        filename = paths.basename(dst_normalized)
        if not filename:
            raise InvalidPath("cannot copy onto the root directory")
        if await self._resolve(dst_normalized) is not None:
            raise AlreadyExists(f"File exists: {dst_normalized}")

        if not entry.is_folder:
            parent_id = await self._ensure_parent(dst_normalized, create=True)
            return await self._create(
                filename,
                parent_id,
                entry.content_type,
                content=entry.content,
                data=entry.data,
            )

        if not recursive:
            raise IsADirectory(f"Is a directory: {entry.path} (pass recursive=True)")
        if dst_normalized.startswith(entry.path + "/"):
            raise InvalidPath(
                f"cannot copy {entry.path} into its own subtree ({dst_normalized})"
            )

        root = await self.mkdir(dst_normalized, parents=True)
        for child in await self.ls(entry.path, recursive=True):
            relative = child.path[len(entry.path) :]
            target = dst_normalized + relative
            if child.is_folder:
                await self.mkdir(target, parents=True)
            else:
                source = await self._require(child.path, fields=_FIELDS_WITH_CONTENT)
                await self._create(
                    paths.basename(target),
                    await self._parent_id(target),
                    source.content_type,
                    content=source.content,
                    data=source.data,
                )
        return root

    async def rm(self, path: str, *, recursive: bool = False) -> int:
        """Delete a file, or a folder and its contents with ``recursive=True``.

        This is a hard delete. Soft-deleting via ``deleted_at`` is not viable
        here: ``child_unique`` is UNIQUE on ``(parent, filename)``, so a tombstone
        would permanently block recreating a file under the same name.

        Returns the number of records removed.
        """
        entry = await self._require(path)
        if not entry.is_folder:
            await self._query("DELETE $id", {"id": entry.id})
            return 1

        children = await self.ls(entry.path, recursive=True)
        if children and not recursive:
            raise DirectoryNotEmpty(f"Directory not empty: {entry.path}")
        # Deepest first, so no row is ever orphaned mid-delete.
        ids = [c.id for c in sorted(children, key=lambda c: c.path, reverse=True)]
        ids.append(entry.id)
        await self._query("DELETE $ids", {"ids": ids})
        return len(ids)

    # ------------------------------------------------------------------ search

    async def search_text(self, query: str, *, limit: int = 20) -> list[SearchHit]:
        """Full-text search over file contents.

        Matching uses the BM25 full-text index. Ranking and snippets are computed
        in Python: on SurrealDB 3.2.x ``search::score`` returns 0.0 and
        ``search::highlight`` returns the content unchanged, so relying on them
        would produce an arbitrary order.
        """
        if not query.strip():
            return []
        rows = await self._query(
            f"SELECT {_FIELDS_WITH_CONTENT} FROM {self.table} WHERE content @1@ $query",
            {"query": query},
        )
        terms = [t for t in re.findall(r"\w+", query.lower()) if t]
        hits: list[SearchHit] = []
        for row in rows or []:
            entry = FileEntry.from_row(row)
            body = (entry.content or "").lower()
            score = float(sum(body.count(term) for term in terms))
            hits.append(
                SearchHit(
                    entry=entry,
                    score=score,
                    snippet=_snippet(entry.content or "", terms),
                )
            )
        hits.sort(key=lambda h: (-h.score, h.entry.path))
        return hits[:limit]

    async def search_semantic(
        self, vector: Sequence[float], *, k: int = 10, ef: int = 40
    ) -> list[SearchHit]:
        """Vector similarity search over the HNSW index.

        Takes a pre-computed embedding -- the library does not call an embedding
        model. Use :meth:`reindex_embeddings` to populate the vectors.
        """
        # k and ef must be *literals* in the query text. Binding them as
        # parameters looks like it works -- it parses and returns correct rows --
        # but the planner silently drops to a brute-force scan instead of the
        # HNSW index (verified with EXPLAIN). The int() cast makes interpolation
        # injection-proof.
        k_literal, ef_literal = int(k), int(ef)
        if k_literal <= 0 or ef_literal <= 0:
            raise ValueError("k and ef must be positive")
        rows = await self._query(
            f"SELECT {_FIELDS}, vector::distance::knn() AS distance "
            f"FROM {self.table} "
            f"WHERE embedding <|{k_literal},{ef_literal}|> $vector "
            "ORDER BY distance ASC",
            {"vector": list(vector)},
        )
        return [
            SearchHit(entry=FileEntry.from_row(row), score=float(row["distance"]))
            for row in (rows or [])
        ]

    async def reindex_embeddings(
        self,
        embed: Callable[[str], Awaitable[Sequence[float]]],
        *,
        version: str,
        batch: int = 32,
    ) -> int:
        """Embed text files whose vectors are missing or stale.

        A row is stale when it has never been embedded, when its content has
        changed since it was, or when a different ``version`` produced it.
        Returns the number of files re-embedded.

        Staleness is keyed on ``embedded_hash`` rather than ``embedded_at``:
        ``updated_at`` is bumped by *every* write, including the indexer's own,
        so a timestamp comparison would mark every row stale forever.
        """
        total = 0
        seen: set[Any] = set()
        while True:
            rows = await self._query(
                f"SELECT id, content FROM {self.table} "
                "WHERE content IS NOT NONE AND content != '' AND ("
                "  embedded_hash IS NONE"
                "  OR embedded_hash != crypto::md5(content)"
                "  OR indexer_version != $version"
                ") LIMIT $batch",
                {"version": version, "batch": batch},
            )
            if not rows:
                return total
            # Guard against a batch that never clears: without this, a staleness
            # predicate that stays true would spin forever.
            fresh = [row for row in rows if str(row["id"]) not in seen]
            if not fresh:
                return total
            for row in fresh:
                seen.add(str(row["id"]))
                vector = await embed(row["content"])
                await self._query(
                    "UPDATE $id SET embedding = $vector, embedded_at = time::now(), "
                    "embedded_hash = crypto::md5(content), "
                    "indexer_version = $version",
                    {"id": row["id"], "vector": list(vector), "version": version},
                )
                total += 1


_EXTENSION_TYPES = {
    "md": "text/markdown",
    "markdown": "text/markdown",
    "txt": "text/plain",
    "html": "text/html",
    "htm": "text/html",
    "json": "application/json",
    "csv": "text/csv",
    "yaml": "application/yaml",
    "yml": "application/yaml",
    "py": "text/x-python",
    "rs": "text/x-rust",
    "js": "text/javascript",
    "ts": "text/typescript",
    "toml": "application/toml",
    "surql": "text/x-surrealql",
}


def _sniff_content_type(filename: str, content: str) -> str:
    """Best-effort media type from the extension, falling back to the content."""
    _, _, extension = filename.rpartition(".")
    if extension and extension.lower() in _EXTENSION_TYPES:
        return _EXTENSION_TYPES[extension.lower()]
    if content[:64].strip().lower().startswith(("<html", "<!doctype html")):
        return "text/html"
    return "text/markdown"


def _unified_diff(before: str, after: str, path: str) -> str:
    # lineterm="" with splitlines() (no keepends) keeps every hunk line on its
    # own row even when the file has no trailing newline.
    diff = difflib.unified_diff(
        before.splitlines(),
        after.splitlines(),
        fromfile=f"a{path}",
        tofile=f"b{path}",
        lineterm="",
    )
    return "\n".join(diff) or f"(no textual change in {path})"


def _snippet(content: str, terms: Sequence[str], *, width: int = 160) -> str:
    """A window of ``content`` around the first matching term."""
    if not content:
        return ""
    lowered = content.lower()
    positions = [pos for pos in (lowered.find(t) for t in terms) if pos >= 0]
    if not positions:
        return content[:width].strip()
    start = max(0, min(positions) - width // 3)
    end = min(len(content), start + width)
    return (
        ("..." if start else "")
        + content[start:end].strip()
        + ("..." if end < len(content) else "")
    )
