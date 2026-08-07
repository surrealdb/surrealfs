"""The Hermes memory provider.

These tests are deliberately synchronous: the provider's methods call
``asyncio.run``, which raises inside the running loop that ``asyncio_mode = "auto"``
gives every ``async def`` test. Hermes calls them from plain worker threads, so
blocking there is the intended shape — see the class docstring.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from surrealfs import NotFound, SurrealFs
from surrealfs.integrations import _connect
from surrealfs.integrations.hermes_memory import SurrealFsMemory, register


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


def _read(path: str) -> str:
    """Read a file back the way any other client would."""

    async def go() -> str:
        async with _connect.connected() as db:
            return await SurrealFs(db).read_text(path)

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

    path = memory.path_for("sess-1", 1)
    body = _read(path)
    assert "/memory/" in path and path.endswith("-001.md")
    assert "# Turn 1 —" in body
    assert "session: sess-1" in body
    assert "how do I get paid" in body
    assert "You invoice monthly." in body


def test_turns_are_numbered_per_session(memory):
    memory.sync_turn("first question", "first answer", session_id="sess-1")
    memory.sync_turn("second question", "second answer", session_id="sess-1")
    # A concurrent gateway session counts independently.
    memory.sync_turn("other question", "other answer", session_id="sess-2")

    assert "first question" in _read(memory.path_for("sess-1", 1))
    assert "second question" in _read(memory.path_for("sess-1", 2))
    assert "other question" in _read(memory.path_for("sess-2", 1))


def test_trivial_turns_are_not_filed(memory):
    memory.sync_turn("thanks", "You're welcome.", session_id="sess-1")
    memory.sync_turn("/reset", "Done.", session_id="sess-1")

    # Nothing was written, so the turn counter never left zero.
    with pytest.raises(NotFound):
        _read(memory.path_for("sess-1", 1))


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
