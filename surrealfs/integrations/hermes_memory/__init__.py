"""A Hermes memory provider backed by SurrealFS.

See https://hermes-agent.nousresearch.com/docs/developer-guide/memory-provider-plugin.

The tool plugin in ``surrealfs.integrations.hermes`` gives the agent tools it chooses
to call. This closes the loop: Hermes hands every completed turn to :meth:`sync_turn`,
which files it under ``/memory``, and asks :meth:`prefetch` for context before each
API call, which searches the whole filesystem — the agent's own notes included.

Memory providers are found by directory, not by entry point, so this has to be
installed on its own::

    ln -s "$PWD/surrealfs/integrations/hermes_memory" ~/.hermes/plugins/surrealfs-memory

and then activated in ``~/.hermes/config.yaml``::

    memory:
      provider: surrealfs-memory

The directory name is the provider name Hermes matches that setting against.
Connection details come from the usual ``SURREALDB_*`` variables; set
``SURREALFS_MEMORY_DIR`` to file turns somewhere other than ``/memory``, and
``SURREALFS_SEMANTIC=1`` to have recall match on meaning as well as wording.
"""

from __future__ import annotations

import asyncio
import os
import re
from datetime import UTC, datetime
from typing import Any

from surrealfs import SurrealFs
from surrealfs.embed import make_embedder
from surrealfs.integrations._connect import connected

try:  # Inside Hermes, subclass the real ABC so its isinstance checks hold.
    from agent.memory_provider import MemoryProvider as _Base
    from agent.memory_provider import is_trivial_prompt
except ImportError:  # Outside it — tests, linting, a plain `pip install`.
    _Base = object  # type: ignore[assignment,misc]

    # Mirrors the shape of Hermes' own TRIVIAL_PROMPT_RE: an anchored alternation
    # that may only be followed by punctuation, so "note" does not match "no".
    _TRIVIAL_RE = re.compile(
        r"^(yes|no|ok|okay|sure|thanks|thank you|y|n|yep|nope|yeah|nah|"
        r"hi|hey|hello|yo|sup|continue|go ahead|do it|proceed|got it|"
        r"cool|nice|great|done|next|lgtm|k)[\W_]*$",
        re.IGNORECASE,
    )

    def is_trivial_prompt(text: str | None) -> bool:
        """Stand-in for Hermes' own helper, which is authoritative when present."""
        if not text or not text.strip():
            return True
        stripped = text.strip()
        return stripped.startswith("/") or bool(_TRIVIAL_RE.match(stripped))


__all__ = ["SurrealFsMemory", "register"]

PROVIDER_NAME = "surrealfs-memory"

RECALL_LIMIT = 5

_UNSAFE = re.compile(r"[^A-Za-z0-9_-]+")


def register(ctx: Any) -> None:
    """Hand Hermes the provider. Called by its memory-plugin discovery."""
    ctx.register_memory_provider(SurrealFsMemory())


def _memory_dir() -> str:
    return os.environ.get("SURREALFS_MEMORY_DIR", "/memory").rstrip("/") or "/memory"


def _semantic() -> bool:
    return os.environ.get("SURREALFS_SEMANTIC", "").strip().lower() in {
        "1",
        "true",
        "yes",
    }


class SurrealFsMemory(_Base):
    """Files each turn into SurrealFS, and recalls from it by search.

    Every method here is called on a plain worker thread with no running event
    loop — Hermes' ``MemoryManager`` runs ``sync_turn`` and ``queue_prefetch`` on a
    ``ThreadPoolExecutor`` and ``prefetch`` on a daemon thread it joins with an 8s
    timeout — so blocking with ``asyncio.run`` is both safe and expected.
    """

    def __init__(self) -> None:
        self._session_id = ""
        # Keyed by session: gateways serve several at once, and `sync_turn` is
        # told which one a turn belongs to.
        self._turns: dict[str, int] = {}

    @property
    def name(self) -> str:
        return PROVIDER_NAME

    def is_available(self) -> bool:
        """True whenever the library imported. No network, per the ABC."""
        return True

    def initialize(self, session_id: str, **kwargs: Any) -> None:
        """Note the session. Deliberately does no I/O — the first write connects."""
        self._session_id = session_id or ""

    def get_tool_schemas(self) -> list[dict[str, Any]]:
        """None. The `surrealfs_*` tools already cover explicit reads and writes."""
        return []

    def system_prompt_block(self) -> str:
        return (
            f"Memory: this conversation is filed under {_memory_dir()}/ in SurrealFS, "
            "and relevant notes are recalled automatically. Use the `surrealfs_*` "
            "tools to read or curate any of it."
        )

    def sync_turn(
        self,
        user_content: str,
        assistant_content: str,
        *,
        session_id: str = "",
        messages: list[dict[str, Any]] | None = None,
    ) -> None:
        """File one completed turn, unless the prompt carries no signal."""
        if is_trivial_prompt(user_content):
            return
        session = session_id or self._session_id
        turn = self._turns.get(session, 0) + 1
        self._turns[session] = turn
        asyncio.run(self._write(session, turn, user_content, assistant_content))

    def prefetch(self, query: str, *, session_id: str = "") -> str:
        """Recall context for the upcoming turn, or "" if nothing matches."""
        if is_trivial_prompt(query):
            return ""
        # ponytail: searches on the spot rather than serving a cached result.
        # Hermes already threads this call and gives it 8s, and a local search
        # answers in milliseconds. If that stops holding, do the work in
        # `queue_prefetch` after each turn and return the cached text here.
        return asyncio.run(self._recall(query))

    def get_config_schema(self) -> list[dict[str, Any]]:
        """Env-var only, so `save_config` can stay the ABC's no-op."""
        return [
            {
                "key": "url",
                "description": "SurrealDB WebSocket URL",
                "default": "ws://localhost:8000/rpc",
                "env_var": "SURREALDB_URL",
            },
            {
                "key": "namespace",
                "description": "SurrealDB namespace",
                "default": "surrealfs",
                "env_var": "SURREALDB_NAMESPACE",
            },
            {
                "key": "database",
                "description": "SurrealDB database",
                "default": "demo",
                "env_var": "SURREALDB_DATABASE",
            },
            {
                "key": "password",
                "description": "SurrealDB root password",
                "secret": True,
                "env_var": "SURREALDB_PASS",
            },
            {
                "key": "memory_dir",
                "description": "Folder in SurrealFS to file turns under",
                "default": "/memory",
                "env_var": "SURREALFS_MEMORY_DIR",
            },
        ]

    # -- the async half ------------------------------------------------------

    def path_for(self, session: str, turn: int, when: datetime | None = None) -> str:
        """Where one turn is filed. Public so tests and callers can predict it."""
        when = when or datetime.now(UTC)
        day = when.strftime("%Y-%m-%d")
        return f"{_memory_dir()}/{day}/{_slug(session)}-{turn:03d}.md"

    async def _write(
        self, session: str, turn: int, user_content: str, assistant_content: str
    ) -> None:
        when = datetime.now(UTC)
        body = (
            f"# Turn {turn} — {when.strftime('%Y-%m-%d %H:%M')} UTC\n\n"
            f"session: {session or 'unknown'}\n\n"
            f"## User\n\n{user_content.strip()}\n\n"
            f"## Assistant\n\n{assistant_content.strip()}\n"
        )
        async with connected() as db:
            await SurrealFs(db).write_text(self.path_for(session, turn, when), body)

    async def _recall(self, query: str) -> str:
        # The same call the `surrealfs_search` tool makes, deliberately: recall
        # used to run its own fan-out, one search per keyword rarest-first, which
        # measured marginally better (MRR 0.854 against 0.833 over eight queries)
        # only because `search` ranked badly. Now that ranking is BM25, the
        # difference is inside the noise of an eight-query sample, and one shared
        # retrieval path that both surfaces improve together beats two.
        async with connected() as db:
            fs = SurrealFs(db)
            # Whole filesystem, not just the memory folder: the notes the agent
            # wrote itself under /preferences/ and /projects/ are the strongest
            # signal in here. The vector is built from the whole sentence, since
            # reading one is the single thing that arm does better.
            vector = await make_embedder()(query) if _semantic() else None
            hits = await fs.search(query, vector=vector, limit=RECALL_LIMIT)
        if not hits:
            return ""
        lines = "\n".join(f"{hit.path}\n    {hit.snippet}" for hit in hits)
        return f"Recalled from SurrealFS:\n{lines}"


def _slug(session: str) -> str:
    """Make a session id safe to use as a filename."""
    return _UNSAFE.sub("-", session).strip("-") or "session"
