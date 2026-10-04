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
import re
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import replace
from typing import Any, Literal

from surrealdb import RecordID

from . import paths
from .chunking import chunk_text
from .errors import (
    AlreadyExists,
    ConflictError,
    DirectoryNotEmpty,
    InvalidPath,
    IsADirectory,
    NotADirectory,
    NotATextFile,
    NotFound,
    PermissionDenied,
    QueryError,
)
from .frontmatter import extract_markdown_links, parse_frontmatter, resolve_link
from .models import (
    FOLDER_CONTENT_TYPE,
    FileEntry,
    FileVersionEntry,
    GraphRelation,
    GrepMatch,
    SearchHit,
    SectionHit,
)

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

_MAX_RETRIES = 5
_RETRY_BACKOFF = 0.02  # seconds; doubles per attempt

# How the schema's `parent_key` field spells "no parent". Must match file.surql.
ROOT_KEY = "root"

# Whether a multi-word full-text query needs just one term or every term.
MatchMode = Literal["any", "all"]

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
    """The indexed key for a parent record id, matching the schema's VALUE clause.

    Built by hand rather than with ``str(parent_id)``, because the two do not
    agree. The schema stores ``<string>$this.parent`` server-side, which never
    escapes; surrealdb-py 3.0.0b8 changed ``RecordID.__str__`` to escape an id
    that would otherwise parse as something else, so an id beginning with a
    digit came back as ``file:\u27e83213bq...\u27e9`` and matched no stored key.
    `ls` then listed nothing for that one folder and stayed correct for its
    siblings -- silent, and depending on how the id generator happened to roll.
    """
    if parent_id is None:
        return ROOT_KEY
    return f"{parent_id.table_name}:{parent_id.id}"


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


def _matches_meta(
    entry_meta: dict[str, Any] | None, filter_meta: dict[str, Any]
) -> bool:
    if not entry_meta:
        return False
    for k, v in filter_meta.items():
        if k not in entry_meta:
            return False
        actual = entry_meta[k]
        if isinstance(actual, (list, tuple, set)):
            if v not in actual and actual != v:
                return False
        elif actual != v:
            return False
    return True


# Every field the model layer needs. `path`, `is_folder` and `gate` are
# COMPUTED, so selecting them costs a parent-chain walk per row -- worth it, and
# the only way to get a path or an ancestry check at all.
_FIELDS = (
    "id, filename, path, content_type, is_folder, owner, mode, gate, "
    "hash, generation, created_at, updated_at, meta, "
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
    if isinstance(raw, dict) and "error" in raw:
        error = raw["error"]
        msg = error.get("message") if isinstance(error, dict) else str(error)
        raise QueryError(f"Database error: {msg}")
    results = raw.get("result", raw) if isinstance(raw, dict) else raw
    if not isinstance(results, list):
        return [results]
    for statement in results:
        if isinstance(statement, dict) and statement.get("status") == "ERR":
            message = str(statement.get("result", "unknown query error"))
            if "sfs:conflict" in message or "sfs:denied_or_conflict" in message:
                raise ConflictError(message)
            if "sfs:not_found" in message:
                raise NotFound(message)
            if "sfs:not_empty" in message:
                raise DirectoryNotEmpty(message)
            if "sfs:exists" in message:
                raise AlreadyExists(message)
            if "sfs:locked" in message:
                raise ConflictError(message)
            if "sfs:denied" in message or "sfs:denied_or_missing" in message:
                raise PermissionDenied(message)
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


def _written(rows: Any, path: str, *, if_generation: int | None = None) -> Any:
    """The single row a write returns, or `PermissionDenied` / `ConflictError`.

    A write the table's PERMISSIONS clause rejects is not reported as an error:
    SurrealDB returns an empty result. Only record auth can produce that -- a
    system credential bypasses the clause -- and `SurrealFs`'s own checks should
    have refused first, so reaching here means the two disagreed about this row.
    Either way it is a denial, and it must not surface as an ``IndexError`` from
    the row unpacking two lines down.
    """
    row = (rows[0] if rows else None) if isinstance(rows, list) else rows
    if not row:
        if if_generation is not None:
            raise ConflictError(f"Conflict: generation mismatch on {path}")
        raise PermissionDenied(f"Permission denied: {path}")
    return row


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

    def _reachable(self, entry: FileEntry) -> bool:
        """Whether every directory above ``entry`` is traversable by this user.

        `gate` is the ancestry check: who may traverse down to this row. Non-NONE
        and not ours means some parent is closed to us, and nothing below it is
        reachable whatever its own bits say. Mirrors `fn::sfs_reachable`, and
        costs no queries because `gate` collapses the whole chain into one
        column. Says nothing about the row's own bits.
        """
        return self.is_root or entry.gate is None or entry.gate == self.user

    def _allowed(self, entry: FileEntry, need: int) -> bool:
        if self.is_root:
            return True
        if not self._reachable(entry):
            return False
        return self._bits(entry) & need == need

    def _owns(self, entry: FileEntry) -> bool:
        """Whether this user may change the row itself, rather than its content.

        Ownership, not mode bits: `chmod` is the one operation no bit grants,
        because anyone who could chmod a folder could open it and read inside.
        """
        return self.is_root or entry.owner == self.user

    def _check(self, entry: FileEntry, need: int, verb: str) -> None:
        if not self._allowed(entry, need):
            raise PermissionDenied(f"Permission denied: cannot {verb} {entry.path}")

    def _shown(self, entry: FileEntry) -> FileEntry:
        """The row as this user may see it: no content hash without the read bit.

        `stat` and `ls` name a file whatever its own bits say, which is what
        unix does -- but `hash` is the md5 of the content, and for anything
        short or low-entropy (a yes/no answer, a token, a name) the digest *is*
        the content, checkable offline against a guess. It is the one field in
        `_FIELDS` that is not metadata. Real `stat` exposes a size, never a
        digest.
        """
        if entry.hash and not self._allowed(entry, READ):
            return replace(entry, hash="")
        return entry

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
        return self._shown(FileEntry.from_row(row)) if row else None

    async def _resolve_reachable(self, path: str) -> FileEntry | None:
        """Resolve a path, or None -- refusing a row behind a closed ancestor.

        The "does this already exist" branch every write takes. `_resolve`
        answers from the index alone, so on its own it hands back a full row
        from inside somebody else's private home: an existence and metadata
        oracle for exactly the path `stat` refuses. Raising keeps that outcome
        indistinguishable from the not-found one, which `_ensure_parent` denies
        a few lines later anyway.
        """
        entry = await self._resolve(path)
        if entry is not None and not self._reachable(entry):
            raise PermissionDenied(f"Permission denied: {paths.normalize(path)}")
        return entry

    async def _require(self, path: str, *, fields: str = _FIELDS) -> FileEntry:
        """Resolve a path, or raise. Enforces ancestry, not the row's own bits.

        `gate` covers every directory above the row in one column, so this costs
        no extra queries. What the caller may then *do* with the row is its own
        check -- `stat` needs nothing further, `cat` needs read.
        """
        entry = await self._resolve(path, fields=fields)
        if entry is None:
            raise NotFound(f"No such file or directory: {paths.normalize(path)}")
        if not self._reachable(entry):
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

    async def head(self, path: str, n: int = 10) -> str:
        """First ``n`` lines of a text file."""
        if n <= 0:
            raise ValueError("n must be positive")
        return "\n".join((await self.read_text(path)).splitlines()[:n])

    async def tail(self, path: str, n: int = 10) -> str:
        """Last ``n`` lines of a text file."""
        if n <= 0:
            raise ValueError("n must be positive")
        return "\n".join((await self.read_text(path)).splitlines()[-n:])

    async def read_range(
        self, path: str, start: int = 1, end: int = -1, *, numbers: bool = False
    ) -> str:
        """Read lines ``start`` through ``end`` (1-indexed, inclusive) of a text file.

        If ``end`` is -1 or exceeds the total line count, reads up to the end
        of the file. If ``numbers`` is True, formats lines with line numbers.
        """
        if start < 1:
            raise ValueError("start line must be >= 1")
        if end != -1 and end < start:
            raise ValueError("end line must be >= start line")
        text = await self.read_text(path)
        if not text:
            return ""
        lines = text.splitlines()
        total = len(lines)
        if start > total:
            return ""
        end_idx = total if (end == -1 or end > total) else end
        selected = lines[start - 1 : end_idx]
        if not numbers:
            return "\n".join(selected)
        width = len(str(end_idx))
        return "\n".join(
            f"{i + start:>{width}} | {line}" for i, line in enumerate(selected)
        )

    async def exists(self, path: str) -> bool:
        """Whether ``path`` is there *and* reachable by this user.

        False for anything behind somebody else's private folder, which is the
        answer `os.path.exists` gives on EACCES and the only safe one here: a
        True would confirm the name of a file inside another user's home, which
        `stat` on the same path correctly refuses to.
        """
        if paths.normalize(path) == "/":
            return True
        entry = await self._resolve(path)
        return entry is not None and self._reachable(entry)

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
            entries = [self._shown(FileEntry.from_row(row)) for row in (rows or [])]
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

    async def glob(
        self,
        pattern: str,
        *,
        meta: dict[str, Any] | None = None,
    ) -> list[FileEntry]:
        """Find files whose path matches a shell-style glob.

        Narrows to the pattern's literal directory prefix server-side, then
        applies the full pattern in Python -- the computed ``path`` field cannot
        be indexed, so the prefix is the only available filter.

        Optional ``meta`` filter matches against structured frontmatter fields.
        """
        prefix = paths.literal_prefix(pattern)
        regex = paths.glob_to_regex(pattern)
        where, extra = self._readable("string::starts_with(path, $prefix)")
        rows = await self._query(
            f"SELECT {_FIELDS} FROM {self.table} WHERE {where} ORDER BY path",
            {"prefix": prefix, **extra},
        )
        matches = [FileEntry.from_row(row) for row in (rows or [])]
        res = [e for e in matches if regex.match(e.path)]
        if meta:
            res = [e for e in res if _matches_meta(e.meta, meta)]
        return res

    async def tree(self, path: str = "/", *, max_depth: int = 3) -> str:
        """Render an ASCII tree of the directory hierarchy up to ``max_depth``."""
        if max_depth < 1:
            raise ValueError("max_depth must be at least 1")
        normalized = paths.normalize(path)
        root_entry = await self._resolve_reachable(normalized)
        if root_entry is None:
            raise NotFound(f"Not found: {normalized}")
        if not root_entry.is_folder:
            raise NotADirectory(f"Not a directory: {normalized}")
        self._check(root_entry, EXEC, "enter")

        lines = [normalized]

        async def _walk(cur_path: str, prefix: str, depth: int) -> None:
            if depth >= max_depth:
                return
            try:
                entries = await self.ls(cur_path)
            except Exception:
                return
            entries = sorted(
                entries, key=lambda e: (not e.is_folder, e.filename.lower())
            )
            count = len(entries)
            for idx, entry in enumerate(entries):
                is_last = idx == count - 1
                connector = "└── " if is_last else "├── "
                suffix = "/" if entry.is_folder else ""
                lines.append(f"{prefix}{connector}{entry.filename}{suffix}")
                if entry.is_folder:
                    sub_prefix = prefix + ("    " if is_last else "│   ")
                    await _walk(entry.path, sub_prefix, depth + 1)

        await _walk(normalized, "", 0)
        return "\n".join(lines)

    # ------------------------------------------------------------------- write

    async def mkdir(
        self,
        path: str,
        *,
        parents: bool = False,
        exist_ok: bool = False,
        mode: int | None = None,
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
                    if exist_ok:
                        return existing
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
        meta: dict[str, Any] | None = None,
    ) -> FileEntry:
        """Insert one row, stamped with this user and a default mode.

        ``path`` is passed in rather than derived because the schema computes it
        from the parent chain only *after* the row exists, and `default_mode`
        has to know whether this is a home directory before then.
        """
        is_folder = content is None and data is None
        if mode is None or home_owner(path, is_folder=is_folder) is not None:
            # A home's mode is the path's to decide, never the caller's. `cp`
            # passes the source folder's mode straight through (`mkdir(dst,
            # mode=entry.mode | 0o700)`), so copying anything onto an
            # unclaimed `/home/dave` would land the home at 0777 -- readable
            # and writable by everyone, the one thing the `/home` seed and
            # `default_owner` exist to prevent.
            mode = default_mode(path, is_folder=is_folder)
        payload: dict[str, Any] = {
            "filename": filename,
            "parent": parent_id,
            "content_type": content_type,
            "owner": default_owner(path, is_folder=is_folder, creator=self.user),
            "mode": mode,
        }
        if content is not None:
            payload["content"] = content
        if data is not None:
            payload["file"] = data
        if meta is not None:
            payload["meta"] = meta
        rows = await self._query(
            f"CREATE {self.table} CONTENT $payload RETURN {_FIELDS}",
            {"payload": payload},
        )
        return FileEntry.from_row(_written(rows, path))

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
        if_generation: int | None = None,
    ) -> FileEntry:
        """Create or replace a text file. Returns the stored entry."""
        normalized = paths.normalize(path)
        filename = paths.basename(normalized)
        if not filename:
            raise InvalidPath("cannot write to the root directory")
        resolved_type = content_type or _sniff_content_type(filename, content)
        meta, _ = parse_frontmatter(content)

        existing = await self._resolve_reachable(normalized)
        if existing is not None:
            if existing.is_folder:
                raise IsADirectory(f"Is a directory: {normalized}")
            self._check(existing, WRITE, "write to")
            if if_generation is not None and existing.generation != if_generation:
                raise ConflictError(
                    f"Generation mismatch on {normalized}: "
                    f"expected {if_generation}, got {existing.generation}"
                )
            rows = await self._query(
                f"UPDATE $id SET content = $content, file = NONE, "
                f"content_type = $content_type, meta = $meta "
                f"WHERE $if_gen IS NONE OR generation = $if_gen "
                f"RETURN {_FIELDS}",
                {
                    "id": existing.id,
                    "content": content,
                    "content_type": resolved_type,
                    "meta": meta,
                    "if_gen": if_generation,
                },
            )
            row = _written(rows, normalized, if_generation=if_generation)
            if isinstance(row, dict) and not row.get("path"):
                row = {**row, "path": normalized}
            entry = FileEntry.from_row(row)
        else:
            if if_generation is not None:
                raise ConflictError(
                    f"Generation mismatch on {normalized}: file does not exist"
                )
            parent_id = await self._ensure_parent(normalized, create=create_parents)
            entry = await self._create(
                filename,
                parent_id,
                resolved_type,
                path=normalized,
                content=content,
                meta=meta,
            )

        # Maintain links_to relations for markdown/text
        links = extract_markdown_links(content)
        if links:
            for link in links:
                target_path = resolve_link(normalized, link)
                try:
                    await self.relate(normalized, "links_to", target_path)
                except Exception:
                    pass

        return entry

    async def write_bytes(
        self,
        path: str,
        data: bytes,
        *,
        content_type: str = "application/octet-stream",
        create_parents: bool = True,
        if_generation: int | None = None,
    ) -> FileEntry:
        """Create or replace a binary file."""
        normalized = paths.normalize(path)
        filename = paths.basename(normalized)
        if not filename:
            raise InvalidPath("cannot write to the root directory")

        existing = await self._resolve_reachable(normalized)
        if existing is not None:
            if existing.is_folder:
                raise IsADirectory(f"Is a directory: {normalized}")
            self._check(existing, WRITE, "write to")
            if if_generation is not None and existing.generation != if_generation:
                raise ConflictError(
                    f"Generation mismatch on {normalized}: "
                    f"expected {if_generation}, got {existing.generation}"
                )
            rows = await self._query(
                f"UPDATE $id SET file = $data, content = NONE, "
                f"content_type = $content_type "
                f"WHERE $if_gen IS NONE OR generation = $if_gen "
                f"RETURN {_FIELDS}",
                {
                    "id": existing.id,
                    "data": data,
                    "content_type": content_type,
                    "if_gen": if_generation,
                },
            )
            row = _written(rows, normalized, if_generation=if_generation)
            if isinstance(row, dict) and not row.get("path"):
                row = {**row, "path": normalized}
            return FileEntry.from_row(row)

        if if_generation is not None:
            raise ConflictError(
                f"Generation mismatch on {normalized}: file does not exist"
            )
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
        existing = await self._resolve_reachable(paths.normalize(path))
        if existing is not None:
            return existing
        return await self.write_text(
            path, "", content_type="text/plain", create_parents=create_parents
        )

    async def append_text(
        self, path: str, suffix: str, *, if_generation: int | None = None
    ) -> FileEntry:
        """Atomically append text to a file."""
        entry = await self._require_file(
            path, fields=_FIELDS_WITH_CONTENT, need=WRITE, verb="append to"
        )
        if entry.content is None:
            raise NotATextFile(f"Not a text file ({entry.content_type}): {entry.path}")
        if if_generation is not None and entry.generation != if_generation:
            raise ConflictError(
                f"Generation mismatch on {entry.path}: "
                f"expected {if_generation}, got {entry.generation}"
            )
        rows = await self._query(
            f"UPDATE $id SET content = (content ?? '') + $suffix "
            f"WHERE $if_gen IS NONE OR generation = $if_gen "
            f"RETURN {_FIELDS}",
            {"id": entry.id, "suffix": suffix, "if_gen": if_generation},
        )
        row = _written(rows, entry.path, if_generation=if_generation)
        if isinstance(row, dict) and not row.get("path"):
            row = {**row, "path": entry.path}
        return FileEntry.from_row(row)

    async def edit(
        self,
        path: str,
        old: str,
        new: str,
        *,
        replace_all: bool = False,
        if_generation: int | None = None,
    ) -> str:
        """Replace text in a file and return a unified diff of the change."""
        entry = await self._require_file(
            path, fields=_FIELDS_WITH_CONTENT, need=READ | WRITE, verb="edit"
        )
        if entry.content is None:
            raise NotATextFile(f"Not a text file ({entry.content_type}): {entry.path}")
        if old == "":
            raise ValueError("`old` must not be empty")
        if if_generation is not None and entry.generation != if_generation:
            raise ConflictError(
                f"Generation mismatch on {entry.path}: "
                f"expected {if_generation}, got {entry.generation}"
            )
        current = entry.content
        if old not in current:
            raise NotFound(f"Text not found in {entry.path}: {old!r}")
        updated = (
            current.replace(old, new) if replace_all else current.replace(old, new, 1)
        )
        meta, _ = parse_frontmatter(updated)
        rows = await self._query(
            "UPDATE $id SET content = $content, meta = $meta "
            "WHERE $if_gen IS NONE OR generation = $if_gen RETURN id",
            {
                "id": entry.id,
                "content": updated,
                "meta": meta,
                "if_gen": if_generation,
            },
        )
        # Or the diff below describes a change that did not happen; see
        # `_written`. `RETURN id` rather than the full field list: nothing here
        # needs the row back, only proof that one was written.
        _written(rows, entry.path, if_generation=if_generation)

        links = extract_markdown_links(updated)
        if links:
            for link in links:
                target_path = resolve_link(entry.path, link)
                try:
                    await self.relate(entry.path, "links_to", target_path)
                except Exception:
                    pass

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
        if home_owner(dst_normalized, is_folder=entry.is_folder) is not None:
            # A home is created, never moved into place. `mv` is a rename: the
            # row keeps the owner and mode it had wherever it came from, and
            # neither can be put right afterwards -- there is no chown, and
            # `mode` is the owner's alone -- so this is the one way to land a
            # `/home/<user>` owned by somebody else, or open to everyone.
            raise PermissionDenied(
                f"Permission denied: {dst_normalized} is a home directory. "
                "Create it with mkdir and move its contents instead."
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
        return FileEntry.from_row(_written(rows, dst_normalized))

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

        # Every folder is created at its source mode with the *owner* bits
        # forced open, then chmod'd down once the tree is in place. It cannot
        # simply be created at `entry.mode`: the children go in through the
        # ordinary `mkdir`/`_create` checks, which need w+x, so a 0555 or 0500
        # folder would fail on its first child and leave a partial tree at
        # the destination -- and it cannot be created at `default_mode`,
        # which comes out 0777 however private the original was. Widening only
        # the owner digit exposes nothing in between: `fn::sfs_gate` reads the
        # *other* x bit, so what the source closed to everyone else stays
        # closed the whole way through.
        folders = [(dst_normalized, entry.mode)]
        await self.mkdir(dst_normalized, parents=True, mode=entry.mode | 0o700)
        for child in children:
            relative = child.path[len(entry.path) :]
            target = dst_normalized + relative
            if child.is_folder:
                folders.append((target, child.mode))
                await self.mkdir(target, parents=True, mode=child.mode | 0o700)
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
        # Order does not matter: the caller owns every row it just created, and
        # `gate` is the owner's own name for a folder closed to others, so each
        # path stays reachable to them whatever the mode lands on.
        for path, mode in folders:
            if mode & 0o700 != 0o700:
                await self.chmod(path, mode)
        return await self.stat(dst_normalized)

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
            # RETURN BEFORE for the same reason `edit` and `chmod` read back:
            # a delete the table's PERMISSIONS clause rejects is not an error,
            # it is an empty result, and reporting it as a removal is worse
            # than failing.
            _written(
                await self._query("DELETE $id RETURN BEFORE", {"id": entry.id}),
                entry.path,
            )
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
        removed = await self._query("DELETE $ids RETURN BEFORE", {"ids": ids})
        # The subtree was vetted above, so a short count means the schema
        # refused a row this layer allowed -- in practice `!fn::sfs_has_children`
        # catching a child created under a folder after `ls` read it. The leaves
        # are gone either way; what must not happen is reporting the whole tree
        # removed when part of it is still there.
        if len(removed or []) != len(ids):
            raise PermissionDenied(
                f"Permission denied: removed {len(removed or [])} of {len(ids)} "
                f"entries under {entry.path}"
            )
        return len(ids)

    async def chmod(self, path: str, mode: int, *, recursive: bool = False) -> int:
        """Change the mode bits of a file or folder. Returns the count changed.

        Only the owner and root may. Unix allows no more than that, and allowing
        more here would make every other check pointless: anyone who can chmod
        a folder can open it and read what is inside.

        With ``recursive``, applies the same mode to everything underneath the
        caller owns, and *skips* what it does not -- exactly as ``chmod -R``
        does. Refusing the whole tree instead was worse than useless: a shared
        folder legitimately holds other people's files, so an agent asked to
        lock down a folder it owns got a flat denial naming somebody else's file
        and changed nothing at all.

        Skipping is safe to leave quiet because of `gate`: the folder's own mode
        is what makes its contents unreachable, whatever bits they carry. What
        the caller asked to close is closed. The count returned is what actually
        changed, so a caller that cares can compare.

        The listing it walks is itself permission-filtered, so this cannot reach
        into a subtree the caller could not otherwise see.
        """
        if not 0 <= mode <= 0o777:
            raise ValueError(f"mode must be between 0o000 and 0o777, got {mode:o}")

        entry = await self._require(path)
        # The path the caller named *is* the request, so not owning that one is
        # an error rather than a skip.
        if not self._owns(entry):
            raise PermissionDenied(
                f"Permission denied: {entry.path} is owned by {entry.owner}"
            )
        targets = [entry]
        if recursive and entry.is_folder:
            children = await self.ls(entry.path, recursive=True)
            targets += [c for c in children if self._owns(c)]

        changed = await self._query(
            "UPDATE $ids SET mode = $mode RETURN mode",
            {"ids": [t.id for t in targets], "mode": mode},
        )
        # `_written`'s problem, for a statement that touches many rows -- and
        # one turn worse: `mode`'s field permission fails by *reverting* the
        # value rather than by dropping the row, so the count alone would not
        # notice. Neither failure raises, so both have to be read back.
        if len(changed or []) != len(targets) or any(
            int(row["mode"]) != mode for row in changed
        ):
            raise PermissionDenied(f"Permission denied: cannot chmod {entry.path}")
        return len(targets)

    # ------------------------------------------------------------------ search

    async def grep(
        self,
        pattern: str,
        *,
        path_prefix: str | None = None,
        glob: str | None = None,
        limit: int = 100,
        is_regex: bool = False,
        case_sensitive: bool = True,
    ) -> list[GrepMatch]:
        """Search text files for lines matching ``pattern``."""
        if not pattern:
            return []
        if limit <= 0:
            return []

        if is_regex:
            raw_regex = pattern
        else:
            raw_regex = re.escape(pattern)

        if not case_sensitive:
            regex_str = f"(?i){raw_regex}"
            compiled = re.compile(raw_regex, re.IGNORECASE)
        else:
            regex_str = raw_regex
            compiled = re.compile(raw_regex)

        normalized_prefix = paths.normalize(path_prefix) if path_prefix else None
        if normalized_prefix == "/":
            normalized_prefix = None

        rows = await self._query(
            "RETURN fn::sfs_grep($pattern, $prefix, $me)",
            {
                "pattern": regex_str,
                "prefix": normalized_prefix,
                "me": self.user,
            },
        )

        glob_matcher = None
        if glob:
            glob_pat = glob if glob.startswith("/") else f"/**/{glob}"
            glob_matcher = paths.glob_to_regex(glob_pat)

        matches: list[GrepMatch] = []
        for row in rows or []:
            entry_path = row.get("path") or ""
            if glob_matcher and not glob_matcher.match(entry_path):
                continue
            content = row.get("content") or ""
            for idx, line in enumerate(content.splitlines()):
                if compiled.search(line):
                    matches.append(
                        GrepMatch(path=entry_path, line_number=idx + 1, line=line)
                    )
                    if len(matches) >= limit:
                        return matches
        return matches

    async def search_text(
        self, query: str, *, limit: int = 20, match: MatchMode = "any"
    ) -> list[SearchHit]:
        """Full-text search over file contents.

        Matching and BM25 ranking both happen in the database, in
        ``fn::sfs_search_text`` -- see ``schema/file.surql``. This method exists
        to turn its rows into :class:`SearchHit` objects and to cut the snippet,
        which is the one part the server cannot do (``search::highlight``
        returns the content unchanged and ``search::offsets`` returns NONE on
        3.2.x).

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

        A ``score`` of 0.0 is normal rather than a failure: SurrealDB uses the
        unsmoothed BM25 IDF, clamped at zero, so a term held by more than about
        half the corpus scores nothing for anyone and ``path`` decides the
        order. See the full-text note in ``schema/file.surql``.
        """
        if not query.strip():
            return []
        rows = await self._query(
            "RETURN fn::sfs_search_text($q, $k, $match, $me)",
            {"q": query, "k": int(limit), "match": match, "me": self.user},
        )
        # The unstemmed words, so the snippet window lands on the match in the
        # original text rather than on a stem that may not appear in it.
        words = _scoring_terms(query)
        return [
            SearchHit(
                entry=(entry := FileEntry.from_row(row)),
                score=float(row.get("score") or 0.0),
                snippet=_snippet(entry.content or "", words),
            )
            for row in rows or []
        ]

    async def search_semantic(
        self, vector: Sequence[float], *, k: int = 10
    ) -> list[SearchHit]:
        """Vector similarity search over the HNSW index.

        Takes a pre-computed embedding -- the library does not call an embedding
        model. Use :meth:`reindex_embeddings` to populate the vectors. The query
        itself is ``fn::sfs_search_semantic`` in ``schema/file.surql``, which
        holds the ``<|80,160|>`` candidate pool the HNSW index needs as a
        literal.
        """
        if int(k) <= 0:
            raise ValueError("k must be positive")
        rows = await self._query(
            "RETURN fn::sfs_search_semantic($qvec, $k, $me)",
            {"qvec": list(vector), "k": int(k), "me": self.user},
        )
        return [
            SearchHit(
                entry=(entry := FileEntry.from_row(row)),
                score=float(row["distance"]),
                # A match by meaning shares no terms with the query, so there is
                # nothing to centre a snippet on: show the opening.
                snippet=_snippet(entry.content or "", ()),
            )
            for row in rows or []
        ]

    async def search(
        self,
        query: str,
        *,
        vector: Sequence[float] | None = None,
        limit: int = 20,
        match: MatchMode = "any",
    ) -> list[SearchHit]:
        """Full-text search, with vector results fused in when given a vector.

        Takes a pre-computed embedding for the same reason
        :meth:`search_semantic` does -- the library never calls an embedding
        model. Without one this is the full-text arm alone.

        ``match`` is passed to the full-text arm only; the vector arm reads the
        whole query either way.

        ``score`` is the fused rank score (``rrf_score``), not either arm's own,
        so it is comparable across the whole result set. The fusion is
        ``fn::sfs_hybrid_search`` in ``schema/file.surql``.

        A query with nothing to match in it is not necessarily an empty search:
        with a vector it is the vector arm alone, fused like any other result,
        so `score` is still `rrf_score`. Only with neither is there nothing to
        run -- and that is the only case this returns early for, since the text
        arm already answers a term-less query with no rows of its own.
        """
        if not query.strip() and vector is None:
            return []
        rows = await self._query(
            "RETURN fn::sfs_hybrid_search($q, $qvec, $k, $match, $me)",
            {
                "q": query,
                "qvec": list(vector) if vector is not None else None,
                "k": int(limit),
                "match": match,
                "me": self.user,
            },
        )
        words = _scoring_terms(query)
        hits: list[SearchHit] = []
        for row in rows or []:
            entry = FileEntry.from_row(row)
            # A row the text arm found carries `score`, and its snippet can be
            # centred on the query terms. One only the vector arm found cannot
            # be -- it shares no words with the query -- so it shows its opening.
            found_by_text = row.get("score") is not None
            hits.append(
                SearchHit(
                    entry=entry,
                    score=float(row.get("rrf_score") or 0.0),
                    snippet=_snippet(
                        entry.content or "", words if found_by_text else ()
                    ),
                )
            )
        return hits

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

        Root only. Indexing has to read every text file in the tree to embed
        it, and the query cannot carry `_readable` and still be an index of the
        whole tree -- so this is the one bulk read that is not filtered, and it
        must not be reachable as anyone else. Searching what root indexed is
        filtered as usual, in :meth:`search_semantic`.
        """
        if not self.is_root:
            raise PermissionDenied(
                f"Permission denied: only {ROOT} may reindex embeddings"
            )
        total = 0
        seen: set[Any] = set()
        while True:
            rows = await self._query(
                f"SELECT id, filename, content_type, content FROM {self.table} "
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
                sections = chunk_text(
                    row["content"],
                    filename=row.get("filename") or "",
                    content_type=row.get("content_type") or "",
                )
                await self._query(
                    "DELETE file_section WHERE file_id = $file_id "
                    "AND section_idx >= $count;",
                    {"file_id": row["id"], "count": len(sections)},
                )
                for s in sections:
                    s_vec = (
                        vector
                        if len(sections) == 1 and s.content == row["content"]
                        else await embed(s.content)
                    )
                    await self._query(
                        "UPSERT file_section CONTENT {"
                        "  file_id: $file_id,"
                        "  section_idx: $idx,"
                        "  heading: $heading,"
                        "  line_start: $line_start,"
                        "  line_end: $line_end,"
                        "  content: $content,"
                        "  embedding: $embedding,"
                        "  source_hash: $source_hash"
                        "};",
                        {
                            "file_id": row["id"],
                            "idx": s.section_idx,
                            "heading": s.heading,
                            "line_start": s.line_start,
                            "line_end": s.line_end,
                            "content": s.content,
                            "embedding": list(s_vec),
                            "source_hash": s.source_hash,
                        },
                    )
                total += 1

    async def search_sections(
        self,
        vector: Sequence[float],
        *,
        limit: int = 20,
    ) -> list[SectionHit]:
        """Semantic search over fine-grained file sections."""
        if int(limit) <= 0:
            raise ValueError("limit must be positive")
        rows = await self._query(
            "RETURN fn::sfs_search_sections($qvec, $k, $me);",
            {"qvec": list(vector), "k": int(limit), "me": self.user},
        )
        return [SectionHit.from_row(r) for r in (rows or [])]

    async def history(self, path: str, *, limit: int = 20) -> list[FileVersionEntry]:
        """List historical revisions of a file at `path`, newest first."""
        clean = paths.normalize(path)
        rows = await self._query(
            "RETURN fn::sfs_history($path, $lim);",
            {"path": clean, "lim": int(limit)},
        )
        return [FileVersionEntry.from_row(r) for r in (rows or [])]

    async def diff(
        self, path: str, from_generation: int, to_generation: int | None = None
    ) -> str:
        """Generate unified diff between two revisions or a revision and current."""
        clean = paths.normalize(path)
        from_row = await self._query(
            "RETURN fn::sfs_version_content($path, $gen);",
            {"path": clean, "gen": int(from_generation)},
        )
        if not from_row:
            raise NotFound(f"Version {from_generation} of {clean} not found")
        from_content = from_row.get("content") or ""

        if to_generation is not None:
            to_row = await self._query(
                "RETURN fn::sfs_version_content($path, $gen);",
                {"path": clean, "gen": int(to_generation)},
            )
            if not to_row:
                raise NotFound(f"Version {to_generation} of {clean} not found")
            to_content = to_row.get("content") or ""
            to_label = f"gen {to_generation}"
        else:
            to_content = await self.read_text(clean)
            to_label = "current"

        from_lines = from_content.splitlines(keepends=True)
        to_lines = to_content.splitlines(keepends=True)
        diff_lines = list(
            difflib.unified_diff(
                from_lines,
                to_lines,
                fromfile=f"{clean} (gen {from_generation})",
                tofile=f"{clean} ({to_label})",
            )
        )
        return "".join(diff_lines)

    async def restore(self, path: str, generation: int) -> FileEntry:
        """Restore a historical version of a file as a new generation."""
        clean = paths.normalize(path)
        row = await self._query(
            "RETURN fn::sfs_version_content($path, $gen);",
            {"path": clean, "gen": int(generation)},
        )
        if not row:
            raise NotFound(f"Version {generation} of {clean} not found")
        content = row.get("content")
        file_bytes = row.get("file_bytes")
        if content is not None:
            return await self.write_text(clean, content)
        elif file_bytes is not None:
            return await self.write_bytes(clean, file_bytes)
        else:
            return await self.write_text(clean, "")

    async def undelete(self, path: str) -> FileEntry:
        """Re-create the last deleted version of a file."""
        clean = paths.normalize(path)
        if await self.exists(clean):
            raise AlreadyExists(f"File already exists: {clean}")
        rows = await self._query(
            "SELECT * FROM file_version WHERE path = $path "
            "ORDER BY created_at DESC, generation DESC LIMIT 1;",
            {"path": clean},
        )
        if not rows:
            raise NotFound(f"No version history found to undelete: {clean}")
        last = rows[0]
        content = last.get("content")
        file_bytes = last.get("file_bytes")
        if content is not None:
            return await self.write_text(clean, content)
        elif file_bytes is not None:
            return await self.write_bytes(clean, file_bytes)
        else:
            return await self.write_text(clean, "")

    async def relate(self, from_path: str, relation: str, to_path: str) -> bool:
        """Create a directional semantic relation edge between two files."""
        valid_relations = {
            "references",
            "supersedes",
            "derives_from",
            "implements",
            "links_to",
        }
        if relation not in valid_relations:
            expected = sorted(valid_relations)
            raise ValueError(
                f"Invalid relation '{relation}'. Expected one of {expected}"
            )
        from_clean = paths.normalize(from_path)
        to_clean = paths.normalize(to_path)
        await self._query(
            "RETURN fn::sfs_relate($from, $rel, $to);",
            {"from": from_clean, "rel": relation, "to": to_clean},
        )
        return True

    async def backlinks(self, path: str) -> list[GraphRelation]:
        """List all inbound relations pointing to `path`."""
        clean = paths.normalize(path)
        rows = await self._query(
            "RETURN fn::sfs_backlinks($path);",
            {"path": clean},
        )
        return [
            GraphRelation(
                source_path=r.get("source_path", ""),
                target_path=clean,
                relation=r.get("relation", ""),
                source_name=r.get("source_name", ""),
                target_name=paths.basename(clean),
            )
            for r in (rows or [])
        ]

    async def get_neighbors(
        self, path: str, *, relation: str | None = None
    ) -> list[GraphRelation]:
        """List all outbound relations from `path`."""
        clean = paths.normalize(path)
        rows = await self._query(
            "RETURN fn::sfs_neighbors($path, $rel);",
            {"path": clean, "rel": relation},
        )
        return [
            GraphRelation(
                source_path=clean,
                target_path=r.get("target_path", ""),
                relation=r.get("relation", ""),
                source_name=paths.basename(clean),
                target_name=r.get("target_name", ""),
            )
            for r in (rows or [])
        ]

    async def find_broken_links(self, path_prefix: str = "/") -> list[dict[str, str]]:
        """Find links in markdown files under `path_prefix` whose targets
        do not exist.
        """
        clean = paths.normalize(path_prefix)
        files = await self.glob(f"{clean}/**/*.md" if clean != "/" else "/**/*.md")
        broken: list[dict[str, str]] = []
        for f in files:
            content = await self.read_text(f.path)
            for link in extract_markdown_links(content):
                target = resolve_link(f.path, link)
                if not await self.exists(target):
                    broken.append(
                        {"source": f.path, "target": target, "raw_link": link}
                    )
        return broken

    async def acquire_lock(
        self, path: str, *, ttl_seconds: int = 60, reason: str = "advisory lease"
    ) -> dict[str, Any]:
        """Acquire or renew an advisory lease on a file."""
        clean = paths.normalize(path)
        row = await self._query(
            "RETURN fn::sfs_acquire_lock($path, $holder, $ttl, $reason);",
            {
                "path": clean,
                "holder": self.user,
                "ttl": int(ttl_seconds),
                "reason": reason,
            },
        )
        return row if isinstance(row, dict) else {}

    async def release_lock(self, path: str) -> bool:
        """Release an advisory lease on a file."""
        clean = paths.normalize(path)
        await self._query(
            "RETURN fn::sfs_release_lock($path, $holder);",
            {"path": clean, "holder": self.user},
        )
        return True


_EXTENSION_TYPES = {
    "md": "text/markdown",
    "markdown": "text/markdown",
    "txt": "text/plain",
    "html": "text/html",
    "htm": "text/html",
    "json": "application/json",
    "csv": "text/csv",
    "tsv": "text/tab-separated-values",
    "yaml": "application/yaml",
    "yml": "application/yaml",
    "py": "text/x-python",
    "rs": "text/x-rust",
    "js": "text/javascript",
    "ts": "text/typescript",
    "jsx": "text/javascript",
    "tsx": "text/typescript",
    "toml": "application/toml",
    "surql": "text/x-surrealql",
    "sql": "text/x-sql",
    "sh": "text/x-shellscript",
    "bash": "text/x-shellscript",
    "zsh": "text/x-shellscript",
    "xml": "application/xml",
    "css": "text/css",
    "diff": "text/x-diff",
    "patch": "text/x-diff",
}


def _sniff_content_type(filename: str, content: str) -> str:
    """Best-effort media type from the extension, falling back to the content."""
    _, _, extension = filename.rpartition(".")
    if extension and extension.lower() in _EXTENSION_TYPES:
        return _EXTENSION_TYPES[extension.lower()]
    stripped = content[:256].strip()
    if stripped.lower().startswith(("<html", "<!doctype html")):
        return "text/html"
    if stripped.startswith(("{", "[")):
        try:
            import json

            json.loads(content)
            return "application/json"
        except Exception:
            pass
    if stripped.startswith("<?xml"):
        return "application/xml"
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


def _scoring_terms(query: str) -> list[str]:
    """The query's content-bearing words, in the order written.

    Stopwords are dropped -- they still *match*, they just do not get a say in
    where the snippet window is centred, which is what this feeds now that
    ranking happens in the database. Centring on "the" anchors the window at
    character zero and shows the caller nothing.

    The name is historical: this used to pick the terms Python's BM25 scored,
    where leaving stopwords in was measured to be the difference between BM25
    helping and ranking exactly as badly as raw term counts. It is also what
    `integrations/hermes_memory` trims a recall query down to.

    A query made only of stopwords keeps them all, so ``"the who"`` still has
    something to centre on.
    """
    terms = [t for t in re.findall(r"\w+", query.lower()) if t]
    return [t for t in terms if t not in _STOPWORDS] or terms


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
