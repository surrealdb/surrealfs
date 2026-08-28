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
import importlib.util
import inspect
import os
import sys
from pathlib import Path

import pytest

from surrealfs import SurrealFs
from surrealfs.fs import ROOT as ROOT_USER
from surrealfs.integrations import _connect, hermes_memory
from surrealfs.integrations.hermes_memory import SurrealFsMemory, register
from surrealfs.integrations.hermes_memory import cli as memory_cli

AGENT = "hermes-test"
HOME = f"/home/{AGENT}"
MEMORIES = f"{HOME}/memories"


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
    # Pinned so the expected paths do not depend on whose machine runs the suite.
    # `test_the_home_belongs_to_the_agent_not_the_machine` covers the unset case.
    monkeypatch.setenv("SURREALFS_AGENT_USER", AGENT)
    monkeypatch.setattr(_connect, "_schema_applied", False)

    provider = SurrealFsMemory()
    provider.initialize("sess-1", hermes_home="/tmp", platform="cli")
    return provider


def _filed(prefix: str = MEMORIES) -> dict[str, str]:
    """Every turn filed under *prefix*, as ``{path: body}``.

    Filenames carry a wall-clock stamp rather than a turn number, so tests find
    them by listing rather than by predicting one.
    """

    async def go() -> dict[str, str]:
        async with _connect.connected() as db:
            fs = SurrealFs(db, user=ROOT_USER)
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


def test_config_schema_covers_every_setting_the_provider_reads():
    """One rule: read it from the environment, declare it here.

    `hermes surrealfs-memory status` walks this list to report what resolved, so
    a setting demoted to README-only disappears from the command that exists to
    diagnose a bad install. Pinned as a set so adding a variable without a field
    fails here rather than silently.
    """
    schema = SurrealFsMemory().get_config_schema()
    assert {field["env_var"] for field in schema} == {
        "SURREALDB_URL",
        "SURREALDB_NAMESPACE",
        "SURREALDB_DATABASE",
        "SURREALDB_USER",
        "SURREALDB_PASS",
        "SURREALFS_AGENT_USER",
        "SURREALFS_MEMORY_DIR",
        "SURREALFS_SEMANTIC",
    }
    # The one field with neither a default nor anything to fall back on.
    (password,) = [f for f in schema if f["env_var"] == "SURREALDB_PASS"]
    assert password["required"] and password["secret"]


def test_the_recall_query_is_the_prompt_cut_down_to_terms():
    """A prompt is not a search box — every word left in is another OR branch."""
    assert hermes_memory._recall_query("what tone does the user prefer, please") == (
        "tone user prefer"
    )
    # A long prompt is long in content words too, so stopwords alone are not a cap.
    long_prompt = " ".join(f"word{n}" for n in range(40))
    assert len(hermes_memory._recall_query(long_prompt).split()) == (
        hermes_memory.RECALL_TERMS
    )
    # Borrowed from `_scoring_terms`, so a query of nothing but stopwords keeps
    # them and still searches for something.
    assert hermes_memory._recall_query("the who") == "the who"


def test_sync_turn_files_the_exchange(memory):
    memory.sync_turn("how do I get paid", "You invoice monthly.", session_id="sess-1")

    ((path, body),) = _filed().items()
    assert path.startswith(f"{MEMORIES}/default/") and "/sess-1-" in path
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


def test_a_trivial_prompt_with_a_long_answer_is_still_filed(memory):
    """The prompt is where the signal usually is, not where the content is."""
    answer = "The retry policy lives in client.py. " * 20
    memory.sync_turn("go ahead", answer, session_id="sess-1")

    ((_, body),) = _filed().items()
    assert "retry policy lives in client.py" in body


def test_the_tool_calls_of_the_turn_are_filed_with_it(memory):
    """Which files were read is invisible in either message's text."""
    memory.sync_turn(
        "which file holds the retry policy",
        "It is in client.py.",
        session_id="sess-1",
        messages=[
            # `messages` is the whole conversation: everything up to the last
            # user message belongs to turns already filed.
            {
                "role": "assistant",
                "tool_calls": [{"function": {"name": "an_earlier_turns_tool"}}],
            },
            *MESSAGES,
        ],
    )

    ((_, body),) = _filed().items()
    assert "## Tools" in body
    assert "surrealfs_search" in body
    assert "an_earlier_turns_tool" not in body


def test_a_turn_with_no_tool_calls_reads_as_it_always_did(memory):
    memory.sync_turn("how do I get paid", "You invoice monthly.", session_id="sess-1")

    ((_, body),) = _filed().items()
    assert "## Tools" not in body


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
    # Filed under an earlier session, because recall drops the *current* one's
    # own turns whatever the profile — see
    # `test_recall_never_serves_this_conversation_back_to_itself`.
    work.sync_turn("the deploy key rotates monthly", "Noted.", session_id="sess-old")

    assert work.root() == f"{MEMORIES}/work"
    assert list(_filed(f"{MEMORIES}/work"))
    # `memory` is the default profile: it can see that a subtree exists, but the
    # turns in it never ride on one of its own turns.
    assert "deploy key" not in memory.prefetch("when does the deploy key rotate")
    assert "deploy key" in work.prefetch("when does the deploy key rotate")


def test_the_home_belongs_to_the_agent_not_the_machine(monkeypatch):
    """`SURREALFS_AGENT_USER` is what keeps two agents off one home.

    The machine username is only the default, and it is the wrong answer in both
    directions: two agents under one account collide on it, and an agent sharing
    a laptop account with its human lands in the human's home.
    """
    monkeypatch.delenv("SURREALFS_AGENT_USER", raising=False)
    monkeypatch.delenv("SURREALFS_MEMORY_DIR", raising=False)
    assert hermes_memory._memory_dir() == (
        f"/home/{hermes_memory._machine_user()}/memories"
    )

    monkeypatch.setenv("SURREALFS_AGENT_USER", "hermes-martin")
    assert hermes_memory._memory_dir() == "/home/hermes-martin/memories"
    # Slugged like a session id, so a name with a slash cannot invent a folder.
    monkeypatch.setenv("SURREALFS_AGENT_USER", "hermes/../martin")
    assert hermes_memory._memory_dir() == "/home/hermes-martin/memories"

    # And the whole thing is still overridable, home or no home.
    monkeypatch.setenv("SURREALFS_MEMORY_DIR", "/shared/turns/")
    assert hermes_memory._memory_dir() == "/shared/turns"


def test_another_agents_home_is_never_recalled(memory):
    """The point of the home: a shared database is not a shared memory.

    Every file here says the same thing, so ranking cannot be what separates
    them — only `_is_mine` can.
    """

    async def seed() -> None:
        async with _connect.connected() as db:
            fs = SurrealFs(db, user=ROOT_USER)
            for path in (
                "/home/hermes-alice/memories/default/2026-01-01/sess-a-120000000.md",
                "/home/hermes-alice/todo.md",  # another agent's scratch, not just turns
                "/home/martin/notes.md",  # the human's own files
                # Filed before homes existed, and left where it is rather than
                # migrated — so it has to be excluded, or it takes a notes slot.
                "/memory/default/2026-01-01/sess-old-120000000.md",
                "/preferences/voice.md",  # the shared root, the handover point
            ):
                await fs.write_text(path, "The flywheel spins.")

    asyncio.run(seed())

    recalled = memory.prefetch("flywheel")
    assert [line for line in recalled.splitlines() if line.startswith("/")] == [
        "/preferences/voice.md"
    ]


def test_the_agents_own_home_is_recalled_but_not_its_other_profiles(memory):
    """The ordering `_is_mine` depends on: a sibling profile lives in this home too."""

    async def seed() -> None:
        async with _connect.connected() as db:
            fs = SurrealFs(db, user=ROOT_USER)
            await fs.write_text(f"{HOME}/todo.md", "The flywheel spins.")
            await fs.write_text(
                f"{MEMORIES}/work/2026-01-01/sess-w-120000000.md",
                "The flywheel spins.",
            )

    asyncio.run(seed())

    recalled = memory.prefetch("flywheel")
    assert [line for line in recalled.splitlines() if line.startswith("/")] == [
        f"{HOME}/todo.md"
    ]


def test_prefetch_recalls_from_the_whole_filesystem(memory):
    """Not just the memories folder: the notes the agent wrote are the point."""

    async def seed() -> None:
        async with _connect.connected() as db:
            await SurrealFs(db, user=ROOT_USER).write_text(
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
            fs = SurrealFs(db, user=ROOT_USER)
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


def test_recall_never_serves_this_conversation_back_to_itself(memory):
    """Worst case of reading the tree it writes: the turn that asked the question.

    One turn per file means short documents, which BM25 favours, so the first
    turn of a session outranks the notes on the query that produced it.
    """
    memory.sync_turn("the flywheel spins", "Noted.", session_id="sess-1")

    async def seed() -> None:
        async with _connect.connected() as db:
            await SurrealFs(db, user=ROOT_USER).write_text(
                "/projects/acme.md", "The flywheel spins."
            )

    asyncio.run(seed())

    recalled = memory.prefetch("flywheel")
    assert "/projects/acme.md" in recalled
    assert f"{MEMORIES}/" not in recalled


def test_filed_turns_cannot_crowd_the_notes_out_of_recall(memory):
    """Curated notes are the point; transcripts accumulate and would bury them."""

    async def seed() -> None:
        async with _connect.connected() as db:
            fs = SurrealFs(db, user=ROOT_USER)
            for n in range(6):
                await fs.write_text(
                    f"{MEMORIES}/default/2026-01-0{n + 1}/older-sess-1200000{n}.md",
                    f"## User\n\nthe flywheel spins, take {n}\n",
                )
            for name in ("voice", "tone", "style"):
                await fs.write_text(
                    f"/preferences/{name}.md", f"The flywheel spins ({name})."
                )

    asyncio.run(seed())

    recalled = memory.prefetch("flywheel")
    paths = [line for line in recalled.splitlines() if line.startswith("/")]
    filed = [p for p in paths if p.startswith(f"{MEMORIES}/")]
    assert len(paths) == 5, paths
    assert len(filed) <= hermes_memory.MEMORY_SLOTS, paths
    # And they are appended after the notes rather than interleaved with them.
    assert paths[: len(paths) - len(filed)] == [p for p in paths if p not in filed]


def test_prefetch_is_quiet_when_nothing_matches(memory):
    assert memory.prefetch("xylophone quarantine zeppelin") == ""
    # And never bothers searching for a bare acknowledgement.
    assert memory.prefetch("ok") == ""


# -- session switching -------------------------------------------------------


def test_a_session_switch_rebinds_the_fallback_session_id(memory):
    """`/resume`, `/branch`, `/reset` and compression all land here.

    `sync_turn` may arrive with no session id at all, and falls back to the one
    `initialize` recorded — which goes stale the moment Hermes rotates it.
    """
    memory.on_session_switch("sess-branched", parent_session_id="sess-1", reset=False)
    memory.sync_turn("a question after branching", "An answer.")

    ((path, _),) = _filed().items()
    assert "/sess-branched-" in path


def test_a_session_switch_tolerates_keywords_it_has_not_seen(memory):
    """Hermes forwards `reason`, and only injects `rewound` when it is true."""
    memory.on_session_switch("sess-2", reason="rewind", rewound=True, whatever=1)

    assert memory._session_id == "sess-2"


# -- the warm recall ---------------------------------------------------------


def _seed(path: str, body: str) -> None:
    async def go() -> None:
        async with _connect.connected() as db:
            await SurrealFs(db, user=ROOT_USER).write_text(path, body)

    asyncio.run(go())


def test_a_queued_recall_is_served_on_the_next_turn(memory):
    """The point of `queue_prefetch`: the search is over before the turn starts.

    Deleting the note between the two calls is what proves it — a `prefetch`
    that searched inline could not possibly still find it.
    """
    _seed("/preferences/voice.md", "Prefers terse replies, no preamble.")
    memory.queue_prefetch("what tone does the user prefer")

    async def drop() -> None:
        async with _connect.connected() as db:
            await SurrealFs(db, user=ROOT_USER).rm("/preferences/voice.md")

    asyncio.run(drop())

    assert "/preferences/voice.md" in memory.prefetch("and what about formatting")


def test_the_warm_recall_is_served_once(memory):
    """It is one turn behind already; serving it twice would make it two."""
    _seed("/preferences/voice.md", "Prefers terse replies, no preamble.")
    memory.queue_prefetch("what tone does the user prefer")

    assert memory.prefetch("first turn after the queue")
    # Nothing warm left, so this one searches — and this query matches nothing.
    assert memory.prefetch("xylophone quarantine zeppelin") == ""


def test_a_session_switch_drops_the_warm_recall(memory):
    """It was warmed for a conversation this process is no longer having."""
    _seed("/preferences/voice.md", "Prefers terse replies, no preamble.")
    memory.queue_prefetch("what tone does the user prefer")
    memory.on_session_switch("sess-fresh", reset=True)

    assert memory.prefetch("xylophone quarantine zeppelin") == ""


def test_queueing_a_trivial_prompt_warms_nothing(memory):
    _seed("/preferences/voice.md", "Prefers terse replies, no preamble.")
    memory.queue_prefetch("what tone does the user prefer")
    memory.queue_prefetch("thanks")

    assert memory.prefetch("xylophone quarantine zeppelin") == ""


# -- mirroring the built-in memory tool --------------------------------------


def test_built_in_memory_writes_are_mirrored(memory):
    """State, not events: the mirror is what `MEMORY.md` currently says."""
    memory.on_memory_write("add", "memory", "Deploys go out on Thursdays.")
    memory.on_memory_write("add", "memory", "The staging database resets nightly.")
    memory.on_memory_write(
        "replace",
        "memory",
        "Deploys go out on Fridays.",
        metadata={"old_text": "Deploys go out on Thursdays."},
    )
    memory.on_memory_write("remove", "memory", "The staging database resets nightly.")
    memory.on_memory_write("add", "user", "Prefers metric units.")

    filed = _filed()
    assert filed[f"{MEMORIES}/default/builtin/memory.md"].strip() == (
        "Deploys go out on Fridays."
    )
    assert filed[f"{MEMORIES}/default/builtin/user.md"].strip() == (
        "Prefers metric units."
    )


def test_a_mirror_edit_for_text_that_was_never_seen_is_dropped(memory):
    """Writes from before this hook existed leave nothing to edit."""
    memory.on_memory_write("remove", "memory", "something nobody ever filed")

    assert _filed() == {}


def test_memory_writes_from_a_subagent_are_not_mirrored(memory):
    """Same rule as `sync_turn`, read per-write — the tool can run in a subagent.

    The provider itself was initialized as primary; only the metadata says
    otherwise.
    """
    memory.on_memory_write(
        "add",
        "memory",
        "A cron job noticed this.",
        metadata={"execution_context": "cron"},
    )

    assert _filed() == {}


def test_mirrored_memory_is_recalled(memory):
    """The whole reason to mirror: recall could not see this before."""
    memory.on_memory_write("add", "memory", "The deploy key rotates every month.")

    assert "builtin/memory.md" in memory.prefetch("when does the deploy key rotate")


# -- compression -------------------------------------------------------------


MESSAGES = [
    {"role": "user", "content": "which file holds the retry policy"},
    {
        "role": "assistant",
        "content": "Let me look.",
        "tool_calls": [
            {"function": {"name": "surrealfs_search", "arguments": '{"q": "retry"}'}}
        ],
    },
    {"role": "assistant", "content": [{"type": "text", "text": "It is in client.py."}]},
]


def test_pre_compress_files_the_messages_and_points_at_them(memory):
    """Everyone else can only summarise what is discarded. This keeps it."""
    pointer = memory.on_pre_compress(MESSAGES)

    ((path, body),) = _filed().items()
    assert path in pointer and "surrealfs_read" in pointer
    assert "which file holds the retry policy" in body
    # Tool calls are the substance of a turn as often as the prose is.
    assert "surrealfs_search" in body
    # Multimodal turns carry typed parts rather than a string.
    assert "It is in client.py." in body


def test_pre_compress_is_quiet_when_there_is_nothing_to_keep(memory):
    assert memory.on_pre_compress([]) == ""
    assert _filed() == {}


def test_only_primary_agents_keep_compressed_messages(surreal_url, memory):
    provider = SurrealFsMemory()
    provider.initialize(
        "sess-1", hermes_home="/tmp", platform="cron", agent_context="cron"
    )

    assert provider.on_pre_compress(MESSAGES) == ""
    assert _filed() == {}


# -- the real ABC ------------------------------------------------------------


def _hermes_abc():
    """Hermes' `MemoryProvider`, loaded from the checkout rather than imported.

    `_Base` is a plain `object` everywhere but inside Hermes, so nothing else in
    this suite checks a single signature against the contract — a rename or a new
    required parameter upstream passes green. Loading it by path keeps that check
    honest without putting Hermes on `sys.path`; the module imports stdlib only,
    so there is nothing else to satisfy.
    """
    home = Path(os.environ.get("HERMES_HOME") or Path.home() / ".hermes")
    path = home / "hermes-agent" / "agent" / "memory_provider.py"
    if not path.exists():
        pytest.skip(f"Hermes is not installed at {path}")
    if (cached := sys.modules.get("_hermes_memory_provider")) is not None:
        return cached.MemoryProvider
    spec = importlib.util.spec_from_file_location("_hermes_memory_provider", path)
    module = importlib.util.module_from_spec(spec)
    # Registered *before* executing: the module's `@dataclass(frozen=True)`
    # classes have string annotations, and dataclasses resolves those through
    # `sys.modules[cls.__module__].__dict__` -- which is `None.__dict__`, and an
    # AttributeError, for a module loaded by path and never registered.
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        del sys.modules[spec.name]
        raise
    return module.MemoryProvider


def test_every_abstract_method_is_implemented():
    abc = _hermes_abc()

    assert set(abc.__abstractmethods__) <= set(dir(SurrealFsMemory))


def test_every_implemented_hook_takes_the_calls_the_abc_permits():
    """Not signature equality — that would fail on things that are fine.

    Annotations legitimately differ (upstream is on `Optional[List[...]]`, this
    file on `list[...] | None`), and `on_session_switch` deliberately absorbs
    upstream's named keywords into `**kwargs` so the next one it adds is not a
    crash. What has to hold is narrower and is the actual contract: every call
    Hermes is allowed to make, this signature accepts.
    """
    abc = _hermes_abc()
    checked = []
    for attribute in dir(abc):
        ours = getattr(SurrealFsMemory, attribute, None)
        theirs = getattr(abc, attribute, None)
        # `ours is None` is a hook left at the ABC's default: `_Base` is a plain
        # `object` here, so an unimplemented hook is absent rather than inherited.
        if attribute.startswith("_") or ours is None or not callable(theirs):
            continue
        checked.append(attribute)
        _assert_accepts(ours, theirs, attribute)
    # Pinned, because a rename upstream shortens this list rather than failing an
    # assertion above — the loop would simply stop checking the renamed hook.
    assert set(checked) == {
        "get_config_schema",
        "get_tool_schemas",
        "initialize",
        "is_available",
        "on_memory_write",
        "on_pre_compress",
        "on_session_switch",
        "prefetch",
        "queue_prefetch",
        "sync_turn",
        "system_prompt_block",
    }


def _assert_accepts(ours, theirs, name: str) -> None:
    """Both the fullest and the barest call the ABC permits must bind."""
    signature = inspect.signature(ours)
    for omit_optional in (False, True):
        args, kwargs = [], {}
        for p in inspect.signature(theirs).parameters.values():
            if p.kind in (p.VAR_POSITIONAL, p.VAR_KEYWORD):
                continue
            if omit_optional and p.default is not p.empty:
                continue
            value = None if p.default is p.empty else p.default
            if p.kind is p.KEYWORD_ONLY:
                kwargs[p.name] = value
            else:
                args.append(value)
        try:
            signature.bind(*args, **kwargs)
        except TypeError as exc:
            raise AssertionError(
                f"{name}{signature} rejects {args}, {kwargs}: {exc}"
            ) from None
