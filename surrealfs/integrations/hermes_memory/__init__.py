"""A Hermes memory provider backed by SurrealFS.

See https://hermes-agent.nousresearch.com/docs/developer-guide/memory-provider-plugin.

The tool plugin in ``surrealfs.integrations.hermes`` gives the agent tools it chooses
to call. This closes the loop: Hermes hands every completed turn to :meth:`sync_turn`,
which files it under ``/home/<agent>/memories/<profile>``, and asks :meth:`prefetch`
for context before each API call, which searches the whole filesystem — the agent's
own notes included, another agent's or profile's transcripts not.

Memory providers are found by directory, not by entry point, so this directory has to
be installed on its own — ``hermes plugins install`` takes it straight out of the repo,
naming it from ``plugin.yaml``::

    hermes plugins install surrealdb/surrealfs/surrealfs/integrations/hermes_memory
    uv pip install --python ~/.hermes/hermes-agent/venv/bin/python \
      "surrealfs @ git+https://github.com/surrealdb/surrealfs.git"
    hermes memory setup surrealfs-memory

The package install is separate and manual because ``surrealfs`` is not on PyPI yet:
the ``pip_dependencies`` Hermes would install itself may not carry a URL. Setup is
:meth:`SurrealFsMemory.post_setup`, which prompts for the settings below, writes them
to ``$HERMES_HOME/.env``, activates the provider in ``~/.hermes/config.yaml``::

    memory:
      provider: surrealfs-memory

and then connects, so a bad database URL is caught there rather than swallowed into a
per-turn warning. The directory name is the provider name Hermes matches that setting
against.
Connection details come from the usual ``SURREALDB_*`` variables; set
``SURREALFS_AGENT_USER`` to name the agent whose home turns are filed in,
``SURREALFS_MEMORY_DIR`` to file them somewhere else entirely, and
``SURREALFS_SEMANTIC=1`` to have recall match on meaning as well as wording.

Those variables are read from ``$HERMES_HOME/.env``, so pointing two profiles at
different databases — or two agents at different homes — is already a matter of
configuring each one. See :func:`_agent_user` for who owns a home and
:func:`_profile` for what happens when one agent runs several.
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

# How many of those slots filed turns may take. The rest belong to the notes
# under /preferences/ and /projects/ — see `_rank`.
MEMORY_SLOTS = 2

# Terms the recall query is cut down to. A prompt is not a search box: every word
# left in is another OR branch, and `search_text` reads the content of every row
# that matches any of them.
RECALL_TERMS = 12  # ponytail: flat cap, rarest-first if it ever measures short

# A trivial prompt is filed anyway when the answer is at least this long: "go
# ahead" followed by a design document is a turn worth keeping.
TRIVIAL_ANSWER_CHARS = 400

# Tool calls appended to a turn file, and how much of one call's arguments.
TOOL_LINES = 10
TOOL_LINE_CHARS = 200

_UNSAFE = re.compile(r"[^A-Za-z0-9_-]+")


def register(ctx: Any) -> None:
    """Hand Hermes the provider. Called by its memory-plugin discovery."""
    ctx.register_memory_provider(SurrealFsMemory())


HOME_ROOT = "/home"

# Where turns were filed before homes existed. Never written now, and excluded from
# recall rather than migrated: left in, one turn per file makes them short documents
# that BM25 favours, so they would take the notes slots `_rank` reserves.
LEGACY_MEMORY_DIR = "/memory"


def _machine_user() -> str:
    """The unix account this process runs as -- the fallback `_agent_user` uses."""
    from .._connect import _machine_user as resolve

    return resolve()


def _agent_user() -> str:
    """The agent whose home this provider files under, and acts as.

    Lifted into `integrations._connect` so the tool plugin, the CLI and this
    provider cannot disagree about who they are -- the answer is now an access
    boundary, not just a path convention.
    """
    from .._connect import agent_user

    return agent_user()


def _home() -> str:
    return f"{HOME_ROOT}/{_agent_user()}"


def _default_memory_dir() -> str:
    return f"{_home()}/memories"


def _memory_dir() -> str:
    default = _default_memory_dir()
    return os.environ.get("SURREALFS_MEMORY_DIR", default).rstrip("/") or default


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
        # One recall, warmed by `queue_prefetch` and consumed by `prefetch`.
        self._warm = ""

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
        """File one completed turn, unless the exchange carries no signal.

        Both sides have to be trivial. Gating on the prompt alone dropped "go
        ahead" followed by a 2000-word design document — the prompt is where the
        signal usually is, but it is not where the content is.
        """
        if self._context != "primary":
            return
        if (
            is_trivial_prompt(user_content)
            and len(assistant_content.strip()) < TRIVIAL_ANSWER_CHARS
        ):
            return
        asyncio.run(
            self._write(
                session_id or self._session_id,
                user_content,
                assistant_content,
                _tool_lines(messages),
            )
        )

    def queue_prefetch(self, query: str, *, session_id: str = "") -> None:
        """Warm the next turn's recall, on the serialized background worker.

        Hermes calls this with the turn that just *finished*, so what it warms
        answers the previous question — see :meth:`prefetch` for why that trade
        is worth taking.
        """
        # Cleared first, so a failed recall leaves nothing warm rather than
        # leaving whatever the turn before that put there.
        self._warm = ""
        if not is_trivial_prompt(query):
            self._warm = asyncio.run(self._recall(query))

    def prefetch(self, query: str, *, session_id: str = "") -> str:
        """Recall context for the upcoming turn, or "" if nothing matches.

        Serves what :meth:`queue_prefetch` warmed, and searches inline when
        there is nothing warm. Two things that costs, both of which Hermes
        hides badly:

        `MemoryManager._prefetch_provider` runs this on a daemon thread it
        joins for 8s, and if a previous call is *still* running it skips recall
        for the whole turn without telling anyone. And with
        ``SURREALFS_SEMANTIC=1`` the query has to be embedded first, which puts
        a third-party API round-trip on the critical path of every turn — so
        "a local search answers in milliseconds" only ever held for half the
        configurations.

        The warm result is one turn behind, because Hermes warms with the
        finished turn's message and asks here with the new one. On a
        conversation that stays on topic that is free; on a topic switch it
        recalls the previous subject for one turn. Upstream's own providers
        make the same trade. Staleness needs no turn counter: the cache holds
        one entry, reading clears it, and `on_session_switch` clears it too.
        """
        if is_trivial_prompt(query):
            return ""
        warm, self._warm = self._warm, ""
        return warm or asyncio.run(self._recall(query))

    def on_session_switch(self, new_session_id: str, **kwargs: Any) -> None:
        """Follow `/resume`, `/branch`, `/reset`, `/new` and compression.

        Turn files are named by wall clock rather than a counter, so
        `_session_id` — the fallback for a `sync_turn` that arrives without one
        — is the only per-session state there is to rebind. The warm recall goes
        with it: it was keyed on a conversation this one is no longer having.

        `**kwargs` is not decoration. `MemoryManager` forwards `reason` and
        `parent_session_id` always, `rewound` only when true, and has added
        keywords before.
        """
        self._session_id = new_session_id or ""
        self._warm = ""

    def on_memory_write(
        self,
        action: str,
        target: str,
        content: str,
        metadata: dict[str, Any] | None = None,
    ) -> None:
        """Mirror a built-in memory tool write into SurrealFS.

        Without this, Hermes' own `MEMORY.md` / `USER.md` and SurrealFS diverge
        permanently and recall never sees what that tool wrote.

        What gets mirrored is the *state*, not the events: Hermes hands over
        deltas (`add` / `replace` / `remove`), and applying them to one file per
        target reproduces the file the user could otherwise only read inside
        Hermes. An append-only log of the events was the alternative and is
        worse on both counts — it is not what `MEMORY.md` says, and a batched
        `operations:` call would file several entries inside one millisecond,
        which is exactly the filename collision :meth:`path_for` exists to avoid.

        The `metadata` parameter must keep that name: Hermes inspects this
        signature to decide how to call it, and falls back to a three-argument
        legacy call for anything it does not recognise.
        """
        meta = metadata or {}
        # Same rule as `sync_turn`, read per-write because the memory tool can
        # run inside a subagent while this provider was initialized as primary.
        if str(meta.get("execution_context") or self._context) != "primary":
            return
        if action not in {"add", "replace", "remove"} or not content.strip():
            return
        # ponytail: one connection per write, and Hermes calls this inline on
        # the conversation thread, once per op — so a batched write is N
        # connects on the turn. Buffer them if that ever shows up in a trace.
        asyncio.run(
            self._mirror(target, action, content, str(meta.get("old_text") or ""))
        )

    def on_pre_compress(self, messages: list[dict[str, Any]]) -> str:
        """File the messages compression is about to discard, and say where.

        The one hook a filesystem-shaped memory is strictly better at than a
        cloud one: everybody else can only summarise what is being thrown away,
        and summarising is what the compressor is already doing. So keep the
        text verbatim and hand the compressor a pointer instead — the detail
        survives at full fidelity, and the agent can go read it.

        Returns the pointer, which Hermes folds into the compression summary
        prompt. Must stay a single write: the whole compression pass runs under
        a host timeout, and work still in flight when it expires is discarded.
        """
        if self._context != "primary" or not messages:
            return ""
        session = self._session_id or "unknown"
        when = datetime.now(UTC)
        path = self.path_for(f"{session}-compressed", when)
        asyncio.run(self._write_transcript(path, session, when, messages))
        return (
            f"The messages being compressed out of this conversation are preserved "
            f"verbatim at {path} in SurrealFS. Say so in the summary, so the detail "
            f"can be read back with the `surrealfs_read` tool instead of guessed at."
        )

    def get_config_schema(self) -> list[dict[str, Any]]:
        """Every setting this provider reads from the environment, in one list.

        Env-var only, so `save_config` can stay the ABC's no-op — `hermes memory
        setup` writes any field carrying an `env_var` to `.env` itself.

        The upstream advice is to prompt only for what the user *must* set and
        document the rest. That is aimed at providers with many options; eight
        is not many, and this list is load-bearing elsewhere — `hermes
        surrealfs-memory status` builds its report by walking it, so a field
        demoted to README-only silently vanishes from the one command whose job
        is diagnosing a bad install. One rule: if the code reads it from the
        environment, it is here.

        The two path fields overlap deliberately: `agent_user` is the knob to
        reach for, and `memory_dir` overrides the whole thing for anyone who wants
        turns outside `/home` altogether.
        """
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
                "key": "user",
                "description": "SurrealDB user",
                "default": "root",
                "env_var": "SURREALDB_USER",
            },
            {
                "key": "password",
                "description": "SurrealDB password",
                "secret": True,
                "required": True,
                "env_var": "SURREALDB_PASS",
            },
            {
                "key": "agent_user",
                "description": (
                    "Name of the agent's home in SurrealFS, under /home. "
                    "Set it when two agents would otherwise share one"
                ),
                # The fallback, not the resolved value: `cli.py` renders this as
                # `os.environ.get(env_var) or default`, so a resolved value here
                # would report the env var's contents even when it is unset.
                "default": _machine_user(),
                "env_var": "SURREALFS_AGENT_USER",
            },
            {
                "key": "memory_dir",
                "description": "Folder in SurrealFS to file turns under",
                "default": _default_memory_dir(),
                "env_var": "SURREALFS_MEMORY_DIR",
            },
            {
                "key": "semantic",
                "description": (
                    "Match recall on meaning as well as wording "
                    "(needs OPENAI_API_KEY and the `embed` extra)"
                ),
                "default": "0",
                "choices": ["0", "1"],
                "env_var": "SURREALFS_SEMANTIC",
            },
        ]

    def post_setup(self, hermes_home: str, config: dict[str, Any]) -> None:
        """The whole of ``hermes memory setup surrealfs-memory``.

        Hermes hands this hook the entire setup once it exists, and the generic path
        it replaces prompts for nothing when setup is given a provider name: it
        writes the activation key and stops, leaving the schema walk to the
        interactive picker only. So a user who followed the README's own command
        never got asked for a database URL.

        Ends by connecting, which is the part no amount of config writing can
        confirm — and the reason ``hermes surrealfs-memory status`` exists at all.
        """
        from hermes_cli.config import save_config, save_env_value
        from hermes_cli.secret_prompt import masked_secret_prompt

        print(f"\n  Configuring {PROVIDER_NAME}:\n")
        for field in self.get_config_schema():
            env_var = field["env_var"]
            current = os.environ.get(env_var)
            try:
                if field.get("secret"):
                    answer = masked_secret_prompt(
                        f"  {field['description']}"
                        f"{' (set, blank to keep)' if current else ''}: "
                    ).strip()
                else:
                    shown = current or field.get("default", "")
                    answer = input(f"  {field['description']} [{shown}]: ").strip()
            except EOFError:
                # Setup is not always a terminal — the web UI drives it too. Every
                # setting has a working default, so an unanswerable prompt is not an
                # error: keep what is there and go on to activate and connect.
                print("  (no input available — keeping the current values)")
                break
            if not answer:
                continue
            save_env_value(env_var, answer)
            # `.env` is read at startup, so this process still holds the old value —
            # and the connection test below is in this process.
            os.environ[env_var] = answer

        config.setdefault("memory", {})["provider"] = PROVIDER_NAME
        save_config(config)
        print(f"\n  Activated in config.yaml. Profile: {_profile(hermes_home)}")

        # Deferred like every other `surrealfs` import here, and for the same reason:
        # setup has to survive being run before the package is installed, which is the
        # state it most needs to report on.
        try:
            from surrealfs.integrations.hermes_memory.cli import _status
        except ImportError as exc:
            print(f"\n  Configured, but surrealfs is not importable yet: {exc}")
            print("  Install it — see 'Install' in the plugin's README — then run:")
            print("  hermes surrealfs-memory status\n")
            return

        _status()
        if _semantic():
            print(
                "  Semantic recall matches only what the indexer has reached — "
                "schedule it:\n  hermes cron create 'every 5m' --no-agent "
                "--name surrealfs-embed --script surrealfs-embed.sh\n"
                "  See https://github.com/surrealdb/surrealfs/blob/main/"
                "docs/hermes-indexer.md\n"
            )

    def root(self) -> str:
        """The folder this agent's profile files its turns under."""
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
        self, session: str, user_content: str, assistant_content: str, tools: str = ""
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
        # Which files were read, which commands ran. Frequently the substance of
        # the turn, and invisible in either message's text. Omitted rather than
        # left empty, so a turn with no tool calls reads as it always did.
        if tools:
            body += f"\n## Tools\n\n{tools}\n"
        async with connected() as db:
            fs = SurrealFs(db, user=_agent_user())
            await fs.write_text(self.path_for(session, when), body)

    async def _mirror(
        self, target: str, action: str, content: str, old_text: str
    ) -> None:
        """Apply one built-in memory delta to this profile's copy of the file."""
        from surrealfs import SurrealFs  # deferred — see `_write`
        from surrealfs.errors import NotFound
        from surrealfs.integrations._connect import connected

        path = f"{self.root()}/builtin/{_slug(target)}.md"
        entry = content.strip()
        async with connected() as db:
            fs = SurrealFs(db, user=_agent_user())
            if action == "add":
                current = await fs.read_text(path) if await fs.exists(path) else ""
                await fs.write_text(path, f"{current}{entry}\n")
                return
            try:
                # `replace` names what it is replacing in `old_text`; `remove`
                # names the entry itself in `content`.
                await fs.edit(path, old_text or entry, entry if old_text else "")
            except NotFound:
                # The mirror never saw the entry — a write from before this hook
                # existed, or a profile whose file was curated by hand. Dropping
                # it is right: there is nothing to edit, and re-adding it would
                # resurrect text the user just removed.
                return

    async def _write_transcript(
        self, path: str, session: str, when: datetime, messages: list[dict[str, Any]]
    ) -> None:
        """Write `messages` out as markdown. The only place `messages` is used."""
        from surrealfs import SurrealFs  # deferred — see `_write`
        from surrealfs.integrations._connect import connected

        parts = [
            f"# Compressed out of {session} at "
            f"{when.strftime('%Y-%m-%d %H:%M:%S')} UTC\n"
        ]
        for message in messages:
            role = str(message.get("role") or "unknown")
            parts.append(f"## {role}\n\n{_render(message)}\n")
        async with connected() as db:
            await SurrealFs(db, user=_agent_user()).write_text(path, "\n".join(parts))

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
            fs = SurrealFs(db, user=_agent_user())
            # Whole filesystem, not just the memory folder: the notes the agent
            # wrote itself under /preferences/ and /projects/ are the strongest
            # signal in here. The vector is built from the whole sentence, since
            # reading one is the single thing that arm does better.
            vector = await make_embedder()(query) if _semantic() else None
            # Over-fetch so the filtering below cannot cost a slot — it drops
            # whole classes of hit, not the odd one. `search_text` reads every
            # matching row's content whatever the limit, so a wider one is close
            # to free.
            hits = await fs.search(
                _recall_query(query), vector=vector, limit=RECALL_LIMIT * 4
            )
        hits = self._rank(hits)
        if not hits:
            return ""
        lines = "\n".join(f"{hit.path}\n    {hit.snippet}" for hit in hits)
        return f"Recalled from SurrealFS:\n{lines}"

    def _rank(self, hits: list[Any]) -> list[Any]:
        """The hits worth putting on a turn, curated notes first.

        Recall reads the tree it writes, so without this it feeds on its own
        output: one turn per file means short documents, which BM25 favours, and
        after a few sessions the top five is verbatim transcript rather than the
        `/preferences/` and `/projects/` notes that are the point. Worst case is
        this session's own first turn outranking everything on the query that
        produced it, which :meth:`_is_this_session` drops outright.

        Filed turns keep :data:`MEMORY_SLOTS` of the five rather than being
        excluded, because sometimes a past turn *is* the answer. Notes keep their
        fused rank; turns are appended after them, never interleaved.
        """
        kept = [
            hit
            for hit in hits
            if self._is_mine(hit.path) and not self._is_this_session(hit.path)
        ]
        tree = f"{_memory_dir()}/"
        notes = [hit for hit in kept if not hit.path.startswith(tree)]
        filed = [hit for hit in kept if hit.path.startswith(tree)]
        return (notes + filed[:MEMORY_SLOTS])[:RECALL_LIMIT]

    def _is_mine(self, path: str) -> bool:
        """Whether `path` is this agent's to recall.

        One thing it drops now: another *profile's* filed turns — raw
        transcripts are what must not cross over, and sibling profiles live
        inside this agent's own home, under the same owner, so no file
        permission can tell them apart.

        Other agents' homes used to be dropped here too. They no longer reach
        this method: `/home/<user>` is created 0700, so `search` never returns
        anything from one. Filtering them again would be dead code.

        Outside `/home/` nothing is dropped. `/preferences/` and `/projects/` are
        the shared root the notes skill points at, and they are the handover point
        between an agent and everyone else — the highest-signal memories in the
        tree, and hiding them would defeat the point of recalling them.
        """
        if path.startswith(f"{self.root()}/"):
            return True
        return not (
            path.startswith(f"{_memory_dir()}/")
            or path.startswith(f"{LEGACY_MEMORY_DIR}/")
        )

    def _is_this_session(self, path: str) -> bool:
        """Whether `path` is a turn this very conversation filed.

        Matched on the basename, not the whole path: `path_for` puts the session
        in the filename and the date in the folder, so a conversation that runs
        past midnight — or a resumed one — spans day folders under one id. The
        `-compressed` transcripts match too, and should: same conversation.

        An empty session id matches nothing. `_slug("")` is `"session"`, which
        would otherwise quietly filter out every file literally named that.
        """
        if not self._session_id or not path.startswith(f"{self.root()}/"):
            return False
        return path.rsplit("/", 1)[-1].startswith(f"{_slug(self._session_id)}-")


def _slug(session: str) -> str:
    """Make a session id safe to use as a filename."""
    return _UNSAFE.sub("-", session).strip("-") or "session"


def _recall_query(query: str) -> str:
    """A whole prompt cut down to something worth searching for.

    `search_text` matches on `content @1,OR@ $query`, so every word left in is
    another OR branch — and it reads the content of every row any branch matched
    back into Python to rank it. A human types three words into the search tool;
    recall gets a paragraph, on every turn, on the thread Hermes joins with an 8s
    timeout.

    Stopwords go first because they carry the branches that match everything, and
    `search_text` already refuses to *score* them (see `_scoring_terms`, which
    this borrows so the two lists cannot drift). Then a flat cap, since a long
    prompt is long in content words too.
    """
    from surrealfs.fs import _scoring_terms  # deferred — see `_write`

    return " ".join(_scoring_terms(query)[:RECALL_TERMS])


def _tool_lines(messages: list[dict[str, Any]] | None) -> str:
    """The tool calls of the turn that just finished, one per line.

    `messages` is the whole conversation, not the turn — Hermes hands the same
    list to every provider (`memory_manager.sync_all`) — so this walks back to
    the last user message and reads only what came after it. Without that, every
    turn file re-lists the entire session.

    Same `→ name(args)` shape :func:`_render` writes, so a turn file and a
    compressed transcript read alike. Truncated because a single `write_file`
    argument can be an entire file, and this rides on every turn.
    """
    turn = list(messages or [])
    for index in range(len(turn) - 1, -1, -1):
        if turn[index].get("role") == "user":
            turn = turn[index + 1 :]
            break
    lines = []
    for message in turn:
        for call in message.get("tool_calls") or []:
            function = call.get("function") or {} if isinstance(call, dict) else {}
            line = f"→ {function.get('name') or 'tool'}({function.get('arguments')})"
            lines.append(line[:TOOL_LINE_CHARS])
    return "\n".join(lines[:TOOL_LINES])


def _render(message: dict[str, Any]) -> str:
    """One OpenAI-style message as text, tool calls included.

    The calls are in here because they are frequently the substance of a turn —
    which files were read, which commands ran — and a transcript that drops them
    preserves the conversation about the work but not the work.
    """
    content = message.get("content")
    if isinstance(content, list):  # Multimodal turns carry typed parts.
        content = "\n".join(
            str(part.get("text") or "") for part in content if isinstance(part, dict)
        )
    lines = [str(content or "").strip()]
    for call in message.get("tool_calls") or []:
        function = call.get("function") or {} if isinstance(call, dict) else {}
        lines.append(f"→ {function.get('name') or 'tool'}({function.get('arguments')})")
    return "\n".join(line for line in lines if line) or "(empty)"
