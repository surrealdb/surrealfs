"""A Hermes memory provider backed by SurrealFS.

See https://hermes-agent.nousresearch.com/docs/developer-guide/memory-provider-plugin.

The tool plugin in ``surrealfs.integrations.hermes`` gives the agent tools it chooses
to call. This closes the loop: Hermes hands every completed turn to :meth:`sync_turn`,
which files it under ``/memory/<profile>``, and asks :meth:`prefetch` for context
before each API call, which searches the whole filesystem — the agent's own notes
included, another profile's transcripts not.

Memory providers are found by directory, not by entry point, so this has to be
installed on its own::

    ln -s "$PWD/surrealfs/integrations/hermes_memory" ~/.hermes/plugins/surrealfs-memory
    hermes memory setup surrealfs-memory

The second step installs ``surrealfs`` into Hermes' own virtualenv — the
``pip_dependencies`` in ``plugin.yaml`` — and activates the provider in
``~/.hermes/config.yaml``::

    memory:
      provider: surrealfs-memory

The directory name is the provider name Hermes matches that setting against.
Connection details come from the usual ``SURREALDB_*`` variables; set
``SURREALFS_MEMORY_DIR`` to file turns somewhere other than ``/memory``, and
``SURREALFS_SEMANTIC=1`` to have recall match on meaning as well as wording.

Those variables are read from ``$HERMES_HOME/.env``, so pointing two profiles at
different databases is already a matter of configuring each one — see
:func:`_profile` for what happens when they share.
"""

from __future__ import annotations

import asyncio
import importlib.util
import os
import re
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

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


DEFAULT_PROFILE = "default"


def _profile(hermes_home: str) -> str:
    """The profile ``hermes_home`` belongs to.

    Hermes puts a named profile's home at ``<root>/profiles/<name>`` and the
    default profile's at the root itself (``~/.hermes``), so the parent directory
    is what distinguishes them — not the name, which could be anything. An absent
    ``hermes_home`` (older Hermes, a direct caller) counts as the default.

    Every profile gets a subtree, the default one included. Leaving the default
    unprefixed would have been less churn but only isolates in one direction: the
    default profile would still recall every named profile's transcripts.

    This scopes *turns*, not the database. The ``SURREALDB_*`` variables come from
    ``$HERMES_HOME/.env``, which is already per-profile, so a user who wants two
    profiles in separate databases has that today. What they cannot configure away
    is one profile recalling another's raw transcripts out of a database they
    deliberately share, which is what the subtree and :meth:`_is_mine` prevent.
    """
    path = Path(hermes_home)
    if not hermes_home or path.parent.name != "profiles":
        return DEFAULT_PROFILE
    return _slug(path.name)


class SurrealFsMemory(_Base):
    """Files each turn into SurrealFS, and recalls from it by search.

    Every method here is called on a plain worker thread with no running event
    loop — Hermes' ``MemoryManager`` runs ``sync_turn`` and ``queue_prefetch`` on a
    ``ThreadPoolExecutor`` and ``prefetch`` on a daemon thread it joins with an 8s
    timeout — so blocking with ``asyncio.run`` is both safe and expected.
    """

    def __init__(self) -> None:
        self._session_id = ""
        self._profile = DEFAULT_PROFILE
        self._context = "primary"

    @property
    def name(self) -> str:
        return PROVIDER_NAME

    def is_available(self) -> bool:
        """Whether the imports the async half defers will succeed. No network.

        `surrealdb` as well as `surrealfs`, even though the first is a hard
        dependency of the second: `hermes update` force-reinstalls a provider's
        declared dependencies precisely because a venv sync can strip or downgrade
        one bridge package and leave the rest (#53272, #70636).
        """
        return all(importlib.util.find_spec(mod) for mod in ("surrealfs", "surrealdb"))

    def initialize(self, session_id: str, **kwargs: Any) -> None:
        """Note the session, profile, and context. No I/O — the first write connects."""
        self._session_id = session_id or ""
        self._profile = _profile(str(kwargs.get("hermes_home") or ""))
        # Absent on older Hermes, which only ran primary agents. Anything else —
        # a subagent, a cron run, a flush — is not the user talking, and the ABC
        # says not to write it: a cron system prompt filed as a memory and
        # recalled later corrupts what the agent believes about the user.
        self._context = str(kwargs.get("agent_context") or "primary")

    def get_tool_schemas(self) -> list[dict[str, Any]]:
        """None. The `surrealfs_*` tools already cover explicit reads and writes."""
        return []

    def system_prompt_block(self) -> str:
        return (
            f"Memory: this conversation is filed under {self.root()}/ in SurrealFS, "
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
        if self._context != "primary" or is_trivial_prompt(user_content):
            return
        asyncio.run(
            self._write(session_id or self._session_id, user_content, assistant_content)
        )

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

    def root(self) -> str:
        """The folder this profile's turns are filed under."""
        return f"{_memory_dir()}/{self._profile}"

    def path_for(self, session: str, when: datetime) -> str:
        """Where one turn is filed. Public so tests and callers can predict it.

        Named by wall clock rather than a turn counter. A counter has to live
        somewhere, and in-process is the one place it cannot: `hermes --resume`
        starts a second process on the same session id, restarts counting at one,
        and `write_text` replaces — so turn one of the resumed conversation
        silently overwrote turn one of the original. Milliseconds are in there
        because seconds alone are not obviously enough, and a filename is cheaper
        than reasoning about how fast two turns can arrive.
        """
        day = when.strftime("%Y-%m-%d")
        stamp = when.strftime("%H%M%S%f")[:-3]
        return f"{self.root()}/{day}/{_slug(session)}-{stamp}.md"

    # -- the async half ------------------------------------------------------

    async def _write(
        self, session: str, user_content: str, assistant_content: str
    ) -> None:
        # Imported here, not at module top: Hermes' discovery drops any provider
        # whose `__init__.py` fails to import, so a module-level `surrealfs` import
        # hides this one from `hermes memory setup` on the machines that most need
        # it — the ones where setup would have installed the package.
        from surrealfs import SurrealFs
        from surrealfs.integrations._connect import connected

        when = datetime.now(UTC)
        body = (
            f"# {when.strftime('%Y-%m-%d %H:%M:%S')} UTC\n\n"
            f"session: {session or 'unknown'}\n\n"
            f"## User\n\n{user_content.strip()}\n\n"
            f"## Assistant\n\n{assistant_content.strip()}\n"
        )
        async with connected() as db:
            await SurrealFs(db).write_text(self.path_for(session, when), body)

    async def _recall(self, query: str) -> str:
        from surrealfs import SurrealFs  # deferred — see `_write`
        from surrealfs.embed import make_embedder
        from surrealfs.integrations._connect import connected

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
            # Over-fetch so the filter below cannot cost a slot. `search_text`
            # reads every matching row's content whatever the limit, so a wider
            # one is close to free.
            hits = await fs.search(query, vector=vector, limit=RECALL_LIMIT * 2)
        hits = [hit for hit in hits if self._is_mine(hit.path)][:RECALL_LIMIT]
        if not hits:
            return ""
        lines = "\n".join(f"{hit.path}\n    {hit.snippet}" for hit in hits)
        return f"Recalled from SurrealFS:\n{lines}"

    def _is_mine(self, path: str) -> bool:
        """Keep everything except another profile's filed turns.

        Notes written with the `surrealfs_*` tools stay shared: they are the
        user's own curated files, in a database the user chose, and hiding them
        per profile would defeat the point of recalling them. Raw transcripts are
        the part that must not cross over.
        """
        return not path.startswith(f"{_memory_dir()}/") or path.startswith(
            f"{self.root()}/"
        )


def _slug(session: str) -> str:
    """Make a session id safe to use as a filename."""
    return _UNSAFE.sub("-", session).strip("-") or "session"
