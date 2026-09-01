"""The SurrealFS core API.

:class:`SurrealFs` wraps an already-connected ``AsyncSurreal`` handle. It never
connects, signs in, or selects a namespace -- the caller owns the connection.

Every instance acts as exactly one user, named at construction. That user is
not discovered from the database: every shipped surface signs in as a
root/namespace/database *system* credential, which bypasses table permissions
entirely, so the identity has to come from the process that built the object.
:data:`ROOT` bypasses every check, as it does on a real machine.

This class is the enforcement boundary -- no tool exposes raw SurrealQL, so
every path an agent has into the tree runs the checks below. See
``docs/permissions.md``.
"""

from __future__ import annotations

import asyncio
import difflib
import math
import re
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import replace
from typing import Any, Literal

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
    PermissionDenied,
    QueryError,
)
from .models import FOLDER_CONTENT_TYPE, FileEntry, SearchHit

__all__ = [
    "ROOT",
    "SurrealFs",
    "default_mode",
    "default_owner",
    "home_owner",
    "raise_for_status",
]

# The user that bypasses every permission check, as uid 0 does on a real
# machine. The indexer runs as this -- it has to read every file to embed it --
# and so does the browser, which is an admin view over the whole tree.
ROOT = "root"

READ, WRITE, EXEC = 4, 2, 1

_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
# A username is one path segment, and it must never collide with the
# `'!closed'` sentinel `fn::sfs_gate` uses for "closed by two owners".
# Matches `_slug` in `integrations/_connect.py`.
_USERNAME = re.compile(r"^[A-Za-z0-9_-]+$")

# How much wider than `k` a non-root vector search casts, so that the
# permission filter -- which HNSW applies only after its own cut -- still has k
# readable rows left to return.
_KNN_OVERFETCH = 4

_MAX_RETRIES = 5
_RETRY_BACKOFF = 0.02  # seconds; doubles per attempt

# How the schema's `parent_key` field spells "no parent". Must match file.surql.
ROOT_KEY = "root"

# Whether a multi-word full-text query needs just one term or every term.
MatchMode = Literal["any", "all"]

# BM25 knobs, matching `BM25(1.2,0.75)` on the index in schema/file.surql: ranking
# happens in Python for now, and the two should not disagree about the parameters.
_BM25_K1 = 1.2  # how fast term frequency saturates
_BM25_B = 0.75  # how hard long documents are penalised

# The analyzer the FULLTEXT index is built with. Must match schema/file.surql: it
# is asked to tokenise queries and content for scoring, and scoring on anything
# else would disagree with what the index matched.
ANALYZER = "fts_simple"

# Dropped when scoring a query, never when matching it. Not linguistics -- just the
# words common enough that counting them buries the term that carries the question.
_STOPWORDS = frozenset(
    "a an and are as at be been being but by can could did do does doing for from "
    "had has have having he her here hers him his how i if in into is it its me "
    "might must my no nor not of off on once only or other our ours out over own "
    "same she should so some such than that the their theirs then there these they "
    "this those to too under until up us very was we were what when where which "
    "while who whom why will with would you your yours "
    "about after again all also any because before below get got just know like "
    "made make need now please said say tell think use used using want".split()
)


def _parent_key(parent_id: RecordID | None) -> str:
    """The indexed key for a parent record id, matching the schema's VALUE clause."""
    return ROOT_KEY if parent_id is None else str(parent_id)


def home_owner(path: str, *, is_folder: bool) -> str | None:
    """The user whose home ``path`` is, or ``None`` if it is not a home.

    A home is any folder directly under ``/home`` -- ``/home/alice`` is alice's,
    and nothing deeper is a home of its own.
    """
    if is_folder and paths.parent_of(path) == paths.HOME_ROOT:
        return paths.basename(path)
    return None


def default_mode(path: str, *, is_folder: bool) -> int:
    """The mode a newly created file or folder gets.

    Half of the default policy. A home is private; everything else is shared
    read-write, which is what the tree already was before permissions existed.
    `chmod` is how you tighten something, not how you open it up.
    """
    if home_owner(path, is_folder=is_folder) is not None:
        return 0o700
    return 0o777 if is_folder else 0o666


def default_owner(path: str, *, is_folder: bool, creator: str) -> str:
    """Who a newly created file or folder belongs to.

    The other half, and the reason it is not simply the creator: a home belongs
    to the user it is named after, whoever runs the `mkdir`. Without this, root
    seeding `/home/alice` -- the indexer, the browser, a migration -- would lock
    alice out of her own home, and there is no `chown` to undo it with.
    """
    return home_owner(path, is_folder=is_folder) or creator


# Every field the model layer needs. `path`, `is_folder` and `gate` are
# COMPUTED, so selecting them costs a parent-chain walk per row -- worth it, and
# the only way to get a path or an ancestry check at all.
_FIELDS = (
    "id, filename, path, content_type, is_folder, owner, mode, gate, "
    "hash, created_at, updated_at, "
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
        user: Who this filesystem acts as. Required, with no default: the
            database cannot tell us (every surface signs in as a system
            credential), and guessing wrong is either a lockout or a leak.
            :data:`ROOT` bypasses every check.
        table: Table name, if you renamed it in the schema.
    """

    def __init__(self, db: Any, *, user: str, table: str = "file") -> None:
        if not _IDENT.match(table):
            raise InvalidPath(f"invalid table name: {table!r}")
        if not _USERNAME.match(user or ""):
            raise InvalidPath(f"user must be [A-Za-z0-9_-], or ROOT: {user!r}")
        self.db = db
        self.table = table
        self.user = user
        self.is_root = user == ROOT

    # ------------------------------------------------------------ permissions

    def _bits(self, entry: FileEntry) -> int:
        """The octal digit that applies to this user. Mirrors `fn::sfs_bits`."""
        if entry.owner == self.user:
            return (entry.mode >> 6) & 7
        return entry.mode & 7

    def _allowed(self, entry: FileEntry, need: int) -> bool:
        if self.is_root:
            return True
        # `gate` is the ancestry check: who may traverse down to this row.
        # Non-NONE and not ours means some parent is closed to us, and nothing
        # below it is reachable whatever its own bits say.
        if entry.gate is not None and entry.gate != self.user:
            return False
        return self._bits(entry) & need == need

    def _check(self, entry: FileEntry, need: int, verb: str) -> None:
        if not self._allowed(entry, need):
            raise PermissionDenied(f"Permission denied: cannot {verb} {entry.path}")

    def _readable(self, where: str) -> tuple[str, dict[str, Any]]:
        """Add the read predicate to a bulk query's WHERE clause.

        `ls`, `glob` and both search arms return many rows without resolving any
        of them, so they cannot be filtered row by row in Python -- and `search`
        returns file *content*, which makes a missed filter a disclosure rather
        than a cosmetic bug. The predicate is `fn::sfs_can_read` in
        `schema/file.surql`, so the rule is written once.

        Root gets the query untouched, which keeps its plans exactly as they
        were before permissions existed.
        """
        if self.is_root:
            return where, {}
        return (
            f"({where}) AND fn::sfs_can_read(gate, owner, mode, $me)",
            {"me": self.user},
        )

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
        """Resolve a path, or raise. Enforces ancestry, not the row's own bits.

        `gate` covers every directory above the row in one column, so this costs
        no extra queries. What the caller may then *do* with the row is its own
        check -- `stat` needs nothing further, `cat` needs read.
        """
        entry = await self._resolve(path, fields=fields)
        if entry is None:
            raise NotFound(f"No such file or directory: {paths.normalize(path)}")
        if not self.is_root and entry.gate is not None and entry.gate != self.user:
            raise PermissionDenied(f"Permission denied: {entry.path}")
        return entry

    async def _require_file(
        self, path: str, *, fields: str = _FIELDS, need: int = READ, verb: str = "read"
    ) -> FileEntry:
        entry = await self._require(path, fields=fields)
        if entry.is_folder:
            raise IsADirectory(f"Is a directory: {entry.path}")
        self._check(entry, need, verb)
        return entry

    async def _parent_id(self, path: str, *, write: bool = False) -> RecordID | None:
        """Record id of ``path``'s parent folder, or ``None`` if it is root.

        With ``write``, also require the bits unix requires to create, remove or
        rename an entry *in* that folder: write and execute on the folder, and
        nothing at all on the entry itself.
        """
        parent_path = paths.parent_of(path)
        if parent_path == "/":
            # The root is not a row (see `_resolve`). It behaves as 0777 owned
            # by root, so anyone may create a top-level entry.
            self._guard_home(path)
            return None
        parent = await self._require(parent_path)
        if not parent.is_folder:
            raise NotADirectory(f"Not a directory: {parent.path}")
        if write:
            self._check(parent, WRITE | EXEC, "write to")
            self._guard_home(path)
        return parent.id

    async def _require_writable_dir(self, folder: str, new_path: str) -> None:
        """Require the bits needed to create ``new_path`` inside ``folder``.

        A folder that does not exist yet is not an error here: the callers that
        allow it go on to `mkdir` the chain, which runs this check on each
        ancestor it actually creates.
        """
        if folder != "/":
            existing = await self._resolve(folder)
            if existing is not None:
                self._check(existing, WRITE | EXEC, "write to")
        self._guard_home(new_path)

    def _guard_home(self, path: str) -> None:
        """Refuse to create, remove or move somebody else's home directory.

        `/home` is 0777 like the rest of the shared tree, so without this anyone
        could squat `/home/alice` before alice first connects and would own it
        -- and owning it means reading everything she later puts in it.

        `/home` itself belongs to root, seeded by `schema/file.surql`. It is
        guarded here too: the root directory is not a row, so nothing above
        `/home` can deny a delete, and removing it lets the same squat happen
        one level up.
        """
        if self.is_root:
            return
        normalized = paths.normalize(path)
        if normalized == paths.HOME_ROOT:
            raise PermissionDenied(f"Permission denied: {normalized} belongs to root")
        if paths.parent_of(normalized) == paths.HOME_ROOT:
            if paths.basename(normalized) != self.user:
                raise PermissionDenied(
                    f"Permission denied: {normalized} is not your home directory"
                )

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
            self._check(folder, READ | EXEC, "list")
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
            # Unfiltered on purpose: unix needs `r` on the *folder* to list it,
            # and then names every child whatever its own bits say -- `ls /home`
            # shows every user's home. Only the descent is filtered, so a
            # recursive walk stops at a folder it may not enter.
            out.extend(entries)
            if not recursive:
                break
            frontier = [
                _parent_key(e.id)
                for e in entries
                if e.is_folder and self._allowed(e, READ | EXEC)
            ]
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
        where, extra = self._readable("string::starts_with(path, $prefix)")
        rows = await self._query(
            f"SELECT {_FIELDS} FROM {self.table} WHERE {where} ORDER BY path",
            {"prefix": prefix, **extra},
        )
        matches = [FileEntry.from_row(row) for row in (rows or [])]
        return [e for e in matches if regex.match(e.path)]

    # ------------------------------------------------------------------- write

    async def mkdir(
        self, path: str, *, parents: bool = False, mode: int | None = None
    ) -> FileEntry:
        """Create a folder. With ``parents``, create missing ancestors too.

        ``mode`` applies to the folder ``path`` names and not to any ancestor
        created on the way to it, the same split as ``mkdir -m``. It defaults to
        `default_mode`, which is what makes a home private.
        """
        if mode is not None and not 0 <= mode <= 0o777:
            raise ValueError(f"mode must be between 0o000 and 0o777, got {mode:o}")
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
                # Descending through it, so we need to be able to traverse it;
                # the write bit is checked on whichever folder we create in.
                self._check(existing, EXEC, "enter")
                parent_id = existing.id
                continue
            if not is_last and not parents:
                raise NotFound(
                    f"No such file or directory: {partial}. "
                    "Pass parents=True to create it."
                )
            await self._require_writable_dir(paths.parent_of(partial), partial)
            created = await self._create(
                segment,
                parent_id,
                FOLDER_CONTENT_TYPE,
                path=partial,
                mode=mode if is_last else None,
            )
            parent_id = created.id

        assert created is not None  # the last segment always exists or is created
        return created

    async def _create(
        self,
        filename: str,
        parent_id: RecordID | None,
        content_type: str,
        *,
        path: str,
        content: str | None = None,
        data: bytes | None = None,
        mode: int | None = None,
    ) -> FileEntry:
        """Insert one row, stamped with this user and a default mode.

        ``path`` is passed in rather than derived because the schema computes it
        from the parent chain only *after* the row exists, and `default_mode`
        has to know whether this is a home directory before then.
        """
        is_folder = content is None and data is None
        payload: dict[str, Any] = {
            "filename": filename,
            "parent": parent_id,
            "content_type": content_type,
            "owner": default_owner(path, is_folder=is_folder, creator=self.user),
            "mode": default_mode(path, is_folder=is_folder) if mode is None else mode,
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
            self._guard_home(path)
            return None
        if create:
            existing = await self._resolve(parent_path)
            if existing is None:
                # mkdir runs the same checks on the way down.
                return (await self.mkdir(parent_path, parents=True)).id
            if not existing.is_folder:
                raise NotADirectory(f"Not a directory: {parent_path}")
            self._check(existing, WRITE | EXEC, "write to")
            self._guard_home(path)
            return existing.id
        return await self._parent_id(path, write=True)

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
            self._check(existing, WRITE, "write to")
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
        return await self._create(
            filename, parent_id, resolved_type, path=normalized, content=content
        )

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
            self._check(existing, WRITE, "write to")
            rows = await self._query(
                f"UPDATE $id SET file = $data, content = NONE, "
                f"content_type = $content_type RETURN {_FIELDS}",
                {"id": existing.id, "data": data, "content_type": content_type},
            )
            row = rows[0] if isinstance(rows, list) else rows
            return FileEntry.from_row(row)

        parent_id = await self._ensure_parent(normalized, create=create_parents)
        return await self._create(
            filename, parent_id, content_type, path=normalized, data=data
        )

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
        entry = await self._require_file(
            path, fields=_FIELDS_WITH_CONTENT, need=READ | WRITE, verb="edit"
        )
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
        # Removing the entry from its old folder is a write to that folder --
        # unix keys rename on the two directories, not on the file. Both checks
        # run before the existence probe below, so a destination this user
        # cannot write to cannot be used to discover what is already in it.
        await self._require_writable_dir(paths.parent_of(entry.path), entry.path)
        parent_id = await self._parent_id(dst_normalized, write=True)
        if await self._resolve(dst_normalized) is not None:
            raise AlreadyExists(f"File exists: {dst_normalized}")
        rows = await self._query(
            f"UPDATE $id SET filename = $filename, parent = $parent RETURN {_FIELDS}",
            {"id": entry.id, "filename": filename, "parent": parent_id},
        )
        row = rows[0] if isinstance(rows, list) else rows
        return FileEntry.from_row(row)

    async def cp(self, src: str, dst: str, *, recursive: bool = False) -> FileEntry:
        """Copy a file, or a whole folder with ``recursive=True``.

        ``recursive`` is all-or-nothing, like `rm`: if any part of the source
        subtree is one this user could not read by hand, nothing is copied. A
        `cp` that returned a tree with a subtree quietly missing would be worse
        than a failure, because the caller believes it has a copy.
        """
        entry = await self._require(src, fields=_FIELDS_WITH_CONTENT)
        self._check(entry, READ, "read")
        dst_normalized = paths.normalize(dst)
        filename = paths.basename(dst_normalized)
        if not filename:
            raise InvalidPath("cannot copy onto the root directory")
        await self._require_writable_dir(
            paths.parent_of(dst_normalized), dst_normalized
        )
        if await self._resolve(dst_normalized) is not None:
            raise AlreadyExists(f"File exists: {dst_normalized}")

        if not entry.is_folder:
            parent_id = await self._ensure_parent(dst_normalized, create=True)
            return await self._create(
                filename,
                parent_id,
                entry.content_type,
                path=dst_normalized,
                content=entry.content,
                data=entry.data,
                mode=entry.mode,
            )

        if not recursive:
            raise IsADirectory(f"Is a directory: {entry.path} (pass recursive=True)")
        if dst_normalized.startswith(entry.path + "/"):
            raise InvalidPath(
                f"cannot copy {entry.path} into its own subtree ({dst_normalized})"
            )

        children = await self.ls(entry.path, recursive=True)
        # Vet the whole source subtree before writing anything at the
        # destination. `ls` names a folder's children whatever their own bits
        # say and only refuses to *descend*, so a folder without r+x here is
        # exactly one whose contents are missing from `children`: copying it
        # would put an empty folder where a subtree belongs and report success.
        # Files need the read bit for the plainer reason -- a copy hands their
        # content to whoever can read the destination.
        for child in (entry, *children):
            if child.is_folder:
                self._check(child, READ | EXEC, "copy")
            else:
                self._check(child, READ, "read")

        # `mode=` on both mkdirs, or a copy quietly opens up what it copied:
        # a folder recreated at `default_mode` comes out 0777 however private
        # the original was.
        root = await self.mkdir(dst_normalized, parents=True, mode=entry.mode)
        for child in children:
            relative = child.path[len(entry.path) :]
            target = dst_normalized + relative
            if child.is_folder:
                await self.mkdir(target, parents=True, mode=child.mode)
            else:
                source = await self._require_file(
                    child.path, fields=_FIELDS_WITH_CONTENT
                )
                await self._create(
                    paths.basename(target),
                    await self._parent_id(target, write=True),
                    source.content_type,
                    path=target,
                    content=source.content,
                    data=source.data,
                    mode=source.mode,
                )
        return root

    async def rm(self, path: str, *, recursive: bool = False) -> int:
        """Delete a file, or a folder and its contents with ``recursive=True``.

        This is a hard delete. Soft-deleting via ``deleted_at`` is not viable
        here: ``child_unique`` is UNIQUE on ``(parent, filename)``, so a tombstone
        would permanently block recreating a file under the same name.

        Returns the number of records removed.

        ``recursive`` is all-or-nothing: if any folder in the subtree is one
        this user could not empty by hand, nothing is deleted. Real ``rm -r`` is
        best-effort and reports per-entry failures, which is not expressible in
        a single return count -- and a partial delete here is worse than none,
        because what survives is a row whose parent is gone.
        """
        entry = await self._require(path)
        # Unix keys deletion on the containing folder, not on the entry: a
        # read-only file in a folder you can write is yours to remove.
        await self._require_writable_dir(paths.parent_of(entry.path), entry.path)
        if not entry.is_folder:
            await self._query("DELETE $id", {"id": entry.id})
            return 1

        children = await self.ls(entry.path, recursive=True)
        if children:
            if not recursive:
                raise DirectoryNotEmpty(f"Directory not empty: {entry.path}")
            # Every folder being emptied needs the bits to enumerate *and*
            # unlink. `ls` names a folder's children whatever their own bits
            # say but only descends into ones it may enter, so a folder failing
            # this check is exactly one whose contents are missing from
            # `children` -- deleting it would leave them behind with a dangling
            # parent, unreachable to everyone and no longer in any listing.
            for folder in (entry, *(c for c in children if c.is_folder)):
                self._check(folder, READ | WRITE | EXEC, "delete from")
        # Deepest first, so no row is ever orphaned mid-delete.
        ids = [c.id for c in sorted(children, key=lambda c: c.path, reverse=True)]
        ids.append(entry.id)
        await self._query("DELETE $ids", {"ids": ids})
        return len(ids)

    async def chmod(self, path: str, mode: int, *, recursive: bool = False) -> int:
        """Change the mode bits of a file or folder. Returns the count changed.

        Only the owner and root may. Unix allows no more than that, and allowing
        more here would make every other check pointless: anyone who can chmod
        a folder can open it and read what is inside.

        With ``recursive``, applies the same mode to everything underneath, like
        ``chmod -R``. The listing it walks is itself permission-filtered, so this
        cannot reach into a subtree the caller could not otherwise see.
        """
        if not 0 <= mode <= 0o777:
            raise ValueError(f"mode must be between 0o000 and 0o777, got {mode:o}")

        entry = await self._require(path)
        targets = [entry]
        if recursive and entry.is_folder:
            targets += await self.ls(entry.path, recursive=True)

        for target in targets:
            if not self.is_root and target.owner != self.user:
                raise PermissionDenied(
                    f"Permission denied: {target.path} is owned by {target.owner}"
                )

        await self._query(
            "UPDATE $ids SET mode = $mode",
            {"ids": [t.id for t in targets], "mode": mode},
        )
        return len(targets)

    # ------------------------------------------------------------------ search

    async def search_text(
        self, query: str, *, limit: int = 20, match: MatchMode = "any"
    ) -> list[SearchHit]:
        """Full-text search over file contents.

        Matching uses the full-text index; ranking is Okapi BM25 computed in
        Python over the matched rows, because on SurrealDB 3.2.x
        ``search::score`` returns 0.0 and ``search::highlight`` returns the
        content unchanged. Stopwords match but do not score -- see
        :func:`_scoring_terms`.

        ``match`` picks how a multi-word query is treated:

        ``"any"`` (the default)
            One term is enough. This surface hands back a ranked list of
            snippets, so a caller can dismiss a weak hit at a glance -- but it
            cannot dismiss a result it never saw, and an agent told to "search
            before you create" writes a duplicate when a real note comes back
            empty. BM25 is what makes the wide net usable: a file that merely
            repeats a common word does not outrank the short exact match.
        ``"all"``
            Every term must appear -- ``"what tone does the user prefer"`` finds
            nothing while ``"prefer"`` finds the note. For callers that want a
            precise filter and read an empty result as a real answer.
        """
        if not query.strip():
            return []
        # ponytail: `limit` is applied after Python ranking, so this fetches the
        # content of every matching row. Fine for a filesystem of notes; if one
        # grows to where a common term matches thousands, ranking has to move
        # into the query (server-side BM25) before a SQL LIMIT can be trusted.
        operator = "@1,OR@" if match == "any" else "@1@"
        words = _scoring_terms(query)
        # Score on the analyzer's own tokens, on both sides. Counting raw words
        # instead silently scored zero for every file that matched only through
        # stemming -- the index turns "Prefers" into "prefer", so a query for
        # "prefer" matched the file and then ranked it below files that did not
        # answer the question at all. `LET` plus a `$terms` column keeps this to
        # one round trip, since `_query` hands back the last statement.
        where, extra = self._readable(f"content {operator} $query")
        rows = await self._query(
            "LET $terms = search::analyze($analyzer, $text);\n"
            f"SELECT {_FIELDS_WITH_CONTENT}, "
            "search::analyze($analyzer, content) AS tokens, $terms AS terms "
            f"FROM {self.table} WHERE {where}",
            {
                "query": query,
                "text": " ".join(words),
                "analyzer": ANALYZER,
                **extra,
            },
        )
        rows = rows or []
        if not rows:
            return []
        tokens = {
            row.get("path", ""): [
                token
                for token in row.get("tokens") or []
                if any(char.isalnum() for char in token)
            ]
            for row in rows
        }
        scores = _bm25(tokens, rows[0].get("terms") or [])
        hits = [
            SearchHit(
                entry=(entry := FileEntry.from_row(row)),
                score=scores.get(entry.path, 0.0),
                # The unstemmed words, so the window lands on the match in the
                # original text rather than on a stem that may not appear in it.
                snippet=_snippet(entry.content or "", words),
            )
            for row in rows
        ]
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
        # HNSW cuts to k *before* the permission predicate filters, so asking for
        # k neighbours would return fewer than k readable ones. Over-fetch, then
        # trim back in Python.
        wanted = k_literal
        if not self.is_root:
            k_literal *= _KNN_OVERFETCH
            ef_literal = max(ef_literal, k_literal)
        where, extra = self._readable(f"embedding <|{k_literal},{ef_literal}|> $vector")
        rows = await self._query(
            f"SELECT {_FIELDS_WITH_CONTENT}, vector::distance::knn() AS distance "
            f"FROM {self.table} "
            f"WHERE {where} "
            "ORDER BY distance ASC",
            {"vector": list(vector), **extra},
        )
        hits: list[SearchHit] = []
        for row in (rows or [])[:wanted]:
            entry = FileEntry.from_row(row)
            hits.append(
                SearchHit(
                    entry=entry,
                    score=float(row["distance"]),
                    # A match by meaning shares no terms with the query, so
                    # there is nothing to centre a snippet on: show the opening.
                    snippet=_snippet(entry.content or "", ()),
                )
            )
        return hits

    async def search(
        self,
        query: str,
        *,
        vector: Sequence[float] | None = None,
        limit: int = 20,
        match: MatchMode = "any",
    ) -> list[SearchHit]:
        """Full-text search, with vector results fused in when given a vector.

        Takes a pre-computed embedding for the same reason :meth:`search_semantic`
        does -- the library never calls an embedding model. Without one this is
        :meth:`search_text` under another name.

        ``match`` is passed to :meth:`search_text` and governs the full-text arm
        only; the vector arm reads the whole query either way.

        ``score`` on the returned hits is the fused rank score, not either arm's
        own score, so it is comparable across the whole result set.
        """
        text_hits = await self.search_text(query, limit=limit, match=match)
        by_path = {hit.path: hit for hit in text_hits}
        ranked = [[hit.path for hit in text_hits]]

        if vector is not None:
            vector_hits = await self.search_semantic(vector, k=limit)
            ranked.append([hit.path for hit in vector_hits])
            for hit in vector_hits:
                # A path both arms found keeps the text hit: its snippet is
                # centred on the query terms, which the vector arm cannot do.
                by_path.setdefault(hit.path, hit)

        fused = list(_rrf(*ranked).items())[:limit]
        return [replace(by_path[path], score=score) for path, score in fused]

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


def _rrf(*ranked: Sequence[str], k: int = 60) -> dict[str, float]:
    """Reciprocal rank fusion: merge ranked path lists into one ranking.

    The two search arms produce incomparable numbers -- full-text scores are term
    counts (higher is better), semantic scores are cosine distances (lower is
    better). Fusing on rank instead of score sidesteps normalising them.
    """
    scores: dict[str, float] = {}
    for arm in ranked:
        for rank, path in enumerate(arm):
            scores[path] = scores.get(path, 0.0) + 1 / (k + rank)
    return dict(sorted(scores.items(), key=lambda kv: -kv[1]))


def _scoring_terms(query: str) -> list[str]:
    """The query's content-bearing words, in the order written.

    Stopwords are dropped for *scoring* only -- they still match, they just do
    not get a say in the ranking or in where the snippet is centred. Leaving them
    in was measured to be the difference between BM25 helping and BM25 ranking
    exactly as badly as raw term counts: with "the" scored, a long file repeating
    it beats a short exact match. A query made only of stopwords keeps them all,
    so ``"the who"`` still ranks something.
    """
    terms = [t for t in re.findall(r"\w+", query.lower()) if t]
    return [t for t in terms if t not in _STOPWORDS] or terms


def _bm25(bodies: dict[str, Sequence[str]], terms: Sequence[str]) -> dict[str, float]:
    """Score analyzed documents against analyzed query terms. Paths to scores.

    Okapi BM25: term frequency saturates (so repetition stops paying), long
    documents are penalised, and a term is worth more the fewer documents hold
    it. `df` is counted over the matched rows rather than the whole table --
    measured no worse than a corpus-wide count, and it costs no extra queries.

    Both arguments must be analyzer output, or a document that matched only by
    stem scores zero.
    """
    if not bodies or not terms:
        return {}
    lengths = {path: len(words) or 1 for path, words in bodies.items()}
    average = sum(lengths.values()) / len(lengths)
    total = len(bodies)

    inverse: dict[str, float] = {}
    for term in set(terms):
        documents = sum(1 for body in bodies.values() if term in body)
        inverse[term] = math.log(1 + (total - documents + 0.5) / (documents + 0.5))

    scores: dict[str, float] = {}
    for path, body in bodies.items():
        score = 0.0
        for term in inverse:
            frequency = body.count(term)
            if not frequency:
                continue
            saturation = frequency + _BM25_K1 * (
                1 - _BM25_B + _BM25_B * lengths[path] / average
            )
            score += inverse[term] * frequency * (_BM25_K1 + 1) / saturation
        scores[path] = score
    return scores


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
