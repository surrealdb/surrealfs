"""The Hermes memory provider.

These tests are deliberately synchronous: the provider's methods call
``asyncio.run``, which raises inside the running loop that ``asyncio_mode = "auto"``
gives every ``async def`` test. Hermes calls them from plain worker threads, so
blocking there is the intended shape — see the class docstring.
"""

from __future__ import annotations

import argparse
import ast
import asyncio
from pathlib import Path

import pytest

from surrealfs import SurrealFs
from surrealfs.integrations import _connect, hermes_memory
from surrealfs.integrations.hermes_memory import SurrealFsMemory, register
from surrealfs.integrations.hermes_memory import cli as memory_cli


class _FakeMemoryCtx:
    """Stands in for Hermes' `_ProviderCollector`."""

    def __init__(self) -> None:
        self.provider = None

    def register_memory_provider(self, provider) -> None:
        self.provider = provider


@pytest.fixture
def memory(surreal_url, request, monkeypatch) -> SurrealFsMemory:
    """A provider pointed at the test server, in a namespace of its own.

    Per-test namespace, like conftest's `db` fixture: these tests assert on
    absence as well as presence, so a shared one would let writes leak between
    them.
    """
    monkeypatch.setenv("SURREALDB_URL", surreal_url)
    namespace = "m" + str(abs(hash(request.node.nodeid)))[:12]
    monkeypatch.setenv("SURREALDB_NAMESPACE", namespace)
    monkeypatch.setenv("SURREALDB_DATABASE", "test")
    monkeypatch.setattr(_connect, "_schema_applied", False)

    provider = SurrealFsMemory()
    provider.initialize("sess-1", hermes_home="/tmp", platform="cli")
    return provider


def _filed(prefix: str = "/memory") -> dict[str, str]:
    """Every turn filed under *prefix*, as ``{path: body}``.

    Filenames carry a wall-clock stamp rather than a turn number, so tests find
    them by listing rather than by predicting one.
    """

    async def go() -> dict[str, str]:
        async with _connect.connected() as db:
            fs = SurrealFs(db)
            # `glob` does not return content, so read each hit back.
            return {
                e.path: await fs.read_text(e.path)
                for e in await fs.glob(f"{prefix}/**/*.md")
            }

    return asyncio.run(go())


def test_the_tool_plugin_never_mentions_the_provider_base_class():
    """Hermes coerces any plugin dir whose __init__.py names it to kind=exclusive.

    That routes the directory to memory discovery instead of the plugin manager, so
    a stray mention here — even in a comment — would silently stop the 14 tools and
    the bundled skill from loading for anyone who installs by symlink.
    """
    from surrealfs.integrations import hermes

    scanned = Path(hermes.__file__).read_text(encoding="utf-8")[:8192]
    assert "MemoryProvider" not in scanned
    assert "register_memory_provider" not in scanned


def _module_level_imports(path: Path) -> set[str]:
    """Every module imported at import time — anything not inside a function body."""
    tree = ast.parse(path.read_text(encoding="utf-8"))
    deferred = {
        node
        for fn in ast.walk(tree)
        if isinstance(fn, ast.FunctionDef | ast.AsyncFunctionDef)
        for node in ast.walk(fn)
    }
    names: set[str] = set()
    for node in ast.walk(tree):
        if node in deferred:
            continue
        if isinstance(node, ast.Import):
            names.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            names.add(node.module or "")
    return names


def test_the_plugin_imports_without_surrealfs_installed():
    """`hermes memory setup` is the only thing that installs `surrealfs` for Hermes.

    It can only run once the user picks this provider out of a list built by
    `_get_available_providers`, which imports each candidate's `__init__.py` and drops
    the ones that raise — so a module-level `surrealfs` import hides the provider from
    the wizard that would have installed it. `cli.py` is loaded even earlier, during
    argparse setup, inside a bare `except Exception`.
    """
    for module in (hermes_memory, memory_cli):
        imported = _module_level_imports(Path(module.__file__))
        eager = [n for n in imported if n.split(".")[0] == "surrealfs"]
        assert not eager, f"{module.__name__} imports {eager} at module level"


def test_register_cli_dispatches_status():
    """Hermes can't find our handler by name, so `register_cli` has to set `func`.

    It looks for `getattr(cli_mod, f"{provider_name}_command")`, and
    `surrealfs-memory_command` is not a valid identifier.
    """
    parser = argparse.ArgumentParser()
    memory_cli.register_cli(parser)

    args = parser.parse_args(["status"])
    assert args.surrealfs_memory_command == "status"
    assert callable(args.func)


def test_register_hands_over_a_provider():
    ctx = _FakeMemoryCtx()
    register(ctx)

    assert isinstance(ctx.provider, SurrealFsMemory)
    # The name Hermes matches `memory.provider` in config.yaml against.
    assert ctx.provider.name == "surrealfs-memory"
    assert ctx.provider.is_available()
    # Context-only: the `surrealfs_*` tools already cover explicit reads and writes.
    assert ctx.provider.get_tool_schemas() == []
    assert "SurrealFS" in ctx.provider.system_prompt_block()


def test_config_schema_is_env_vars_only():
    """`save_config` stays a no-op only if every field names an env var."""
    for field in SurrealFsMemory().get_config_schema():
        assert field["env_var"], field


def test_sync_turn_files_the_exchange(memory):
    memory.sync_turn("how do I get paid", "You invoice monthly.", session_id="sess-1")

    ((path, body),) = _filed().items()
    assert path.startswith("/memory/default/") and "/sess-1-" in path
    assert "session: sess-1" in body
    assert "how do I get paid" in body
    assert "You invoice monthly." in body


def test_every_turn_gets_its_own_file(memory):
    memory.sync_turn("first question", "first answer", session_id="sess-1")
    memory.sync_turn("second question", "second answer", session_id="sess-1")
    # A concurrent gateway session is filed under its own name.
    memory.sync_turn("other question", "other answer", session_id="sess-2")

    filed = _filed()
    assert len(filed) == 3, filed
    assert sorted(p.rsplit("/", 1)[1].rsplit("-", 1)[0] for p in filed) == [
        "sess-1",
        "sess-1",
        "sess-2",
    ]


def test_resuming_a_session_does_not_overwrite_its_earlier_turns(memory, surreal_url):
    """`hermes --resume` starts a second process on the same session id.

    A turn counter held in memory restarts at one there, and `write_text`
    replaces, so the original turn one used to disappear. Names carry a wall-clock
    stamp instead.
    """
    memory.sync_turn("first question of the day", "answer one", session_id="sess-9")

    resumed = SurrealFsMemory()
    resumed.initialize("sess-9", hermes_home="/tmp", platform="cli")
    resumed.sync_turn("a totally different question", "answer two", session_id="sess-9")

    bodies = "\n".join(_filed().values())
    assert "first question of the day" in bodies
    assert "a totally different question" in bodies


def test_trivial_turns_are_not_filed(memory):
    memory.sync_turn("thanks", "You're welcome.", session_id="sess-1")
    memory.sync_turn("/reset", "Done.", session_id="sess-1")

    assert _filed() == {}


def test_only_primary_agents_write(surreal_url, memory):
    """The ABC: a cron or subagent turn filed as a memory corrupts the user model."""
    for context in ("cron", "subagent", "flush"):
        provider = SurrealFsMemory()
        provider.initialize(
            "sess-1", hermes_home="/tmp", platform="cron", agent_context=context
        )
        provider.sync_turn("run the nightly report", "Done.", session_id="sess-1")

    assert _filed() == {}


def test_profiles_do_not_read_each_others_turns(memory):
    """Two profiles sharing one database still get separate memories."""
    work = SurrealFsMemory()
    work.initialize("sess-1", hermes_home="/root/.hermes/profiles/work", platform="cli")
    work.sync_turn("the deploy key rotates monthly", "Noted.", session_id="sess-1")

    assert work.root() == "/memory/work"
    assert list(_filed("/memory/work"))
    # `memory` is the default profile: it can see that a subtree exists, but the
    # turns in it never ride on one of its own turns.
    assert "deploy key" not in memory.prefetch("when does the deploy key rotate")
    assert "deploy key" in work.prefetch("when does the deploy key rotate")


def test_prefetch_recalls_from_the_whole_filesystem(memory):
    """Not just /memory: the notes the agent wrote itself are the point."""

    async def seed() -> None:
        async with _connect.connected() as db:
            await SurrealFs(db).write_text(
                "/preferences/voice.md", "Prefers terse replies, no preamble."
            )

    asyncio.run(seed())

    recalled = memory.prefetch("what tone does the user prefer")
    assert "/preferences/voice.md" in recalled
    assert "Recalled from SurrealFS" in recalled


def test_recall_finds_the_note_that_answers_the_question(memory):
    """Recall goes through `fs.search`, the same call the search tool makes.

    It used to run its own fan-out — one search per keyword, rarest first — which
    scored marginally better (MRR 0.854 against 0.833 over eight queries) only
    because `search` ranked badly. Now that ranking is BM25 over analyzer tokens,
    the gap is inside the noise of that sample, so this asserts the note is
    recalled rather than pinning an exact position: one retrieval path that both
    surfaces improve together is worth more than a rank.
    """

    async def seed() -> None:
        async with _connect.connected() as db:
            fs = SurrealFs(db)
            await fs.write_text("/preferences/voice.md", "Prefers terse replies.")
            await fs.write_text(
                "/projects/acme/notes.md",
                "The user wants the user dashboard. The user asked what the user does.",
            )
            await fs.write_text("/notes/music.md", "The tone of the guitar was warm.")

    asyncio.run(seed())

    recalled = memory.prefetch("what tone does the user prefer")
    paths = [line for line in recalled.splitlines() if line.startswith("/")]
    assert "/preferences/voice.md" in paths, recalled
    # Scoring on the analyzer's tokens is what gets it here at all: "Prefers"
    # only matches "prefer" after stemming, and counting raw words scored the
    # file 0.000 and sorted it last.
    assert len(paths) <= 5, "RECALL_LIMIT still caps what rides on the turn"


def test_prefetch_is_quiet_when_nothing_matches(memory):
    assert memory.prefetch("xylophone quarantine zeppelin") == ""
    # And never bothers searching for a bare acknowledgement.
    assert memory.prefetch("ok") == ""
