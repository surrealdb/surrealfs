# Hermes memory provider — decisions

Choices made in `surrealfs/integrations/hermes_memory/` that a future change could
plausibly undo without realising what it was for. Rationale that fits in a docstring
lives in the docstring; this file is for the parts that span files, and for the
alternatives that were considered and rejected.

Open work is in [improvements](hermes-memory-review-improvements.md). Both files were
written against
[Building a Memory Provider Plugin](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/memory-provider-plugin.md)
and the sources it describes — `agent/memory_provider.py`, `agent/memory_manager.py`,
`plugins/memory/__init__.py`, `hermes_cli/memory_setup.py`, `hermes_cli/plugins.py` —
which is where the line references below point. Where the published doc and those
sources disagree, the sources win: the doc omits `on_session_switch` entirely and shows
`on_memory_write` without its `metadata` parameter.

## Turns are named by wall clock, not by a turn number

`/memory/<profile>/<day>/<session>-HHMMSSmmm.md`.

A counter has to be stored somewhere, and in-process is the one place it cannot be:
`hermes --resume` starts a second process on the same session id, a counter in
`__init__` restarts at one, and `SurrealFs.write_text` replaces — so turn one of the
resumed conversation silently overwrote turn one of the original. Reproduced before
the fix; `test_resuming_a_session_does_not_overwrite_its_earlier_turns` holds it
down.

Rejected: seeding the counter from a directory listing on first write. Correct, but
it trades a free string format for a query and a cache, and buys only a prettier
filename.

Milliseconds are in the stamp because seconds alone are not *obviously* enough. Two
turns in one second is implausible at LLM pace, but "implausible" is how the bug
above happened.

Consequence for `on_session_switch`: there is no per-session state left to invalidate
except `_session_id` and the warm recall below, so that hook is two reassignments rather
than a buffer flush.

The `# Turn N —` heading went with it, replaced by the timestamp. That is a
doc-visible format change and the cheapest thing here to revert — the filename is
the part that has to stay collision-free.

## Profiles scope the turn subtree, not the database

`initialize()`'s `hermes_home` kwarg picks a segment: `<root>/profiles/<name>` →
`<name>`, anything else (including `~/.hermes` and an absent kwarg) → `default`.
Turns are written under `/memory/<profile>/`, and `_is_mine` drops other profiles'
folders from recall.

Three things this deliberately does not do:

**It does not scope the database.** That would isolate harder, and it would also cut
recall off from the notes the agent writes with the `surrealfs_*` tools, which read
`SURREALDB_DATABASE` unscoped and have no way to learn the profile. Those notes are
the highest-signal memories in the tree — losing them defeats the design. Users who
want database-level separation already have it: `.env` lives in `$HERMES_HOME`, so
`SURREALDB_DATABASE` is per-profile today, and `hermes memory setup` writes it there.

**It does not hide tool-written notes across profiles.** They are the user's own
curated files in a database the user chose. Raw transcripts are the part that must
not cross over. If this ever needs to change, `_is_mine` is the single place.

**It does not leave the default profile unprefixed.** That was less churn, but it
isolates in one direction only: `/memory/<day>/` for the default and
`/memory/<name>/<day>/` for the rest means the default profile still recalls every
named profile's transcripts. Symmetry is worth the extra path segment.

`_recall` over-fetches `RECALL_LIMIT * 2` so the filter cannot cost a result slot.
Nearly free — `search_text` reads every matching row's content regardless of the
limit (see `fs.py:586`).

`_is_mine` is also the natural home for
[improvements #2](hermes-memory-review-improvements.md), down-weighting the memory
tree against hand-written notes.

## Only primary agents write

`sync_turn` returns early unless `agent_context` is `"primary"`. Subagent, cron and
flush turns are not the user talking, and the ABC is explicit that filing them
corrupts what the agent believes about the user.

The kwarg is absent on older Hermes, which only ran primary agents, so a missing
value must default to `"primary"` — reading it as non-primary would silently stop
all writes.

`prefetch` is deliberately *not* gated. The ABC's instruction is about writes, and
recalling context for a cron run is useful rather than harmful.

## Recall is one turn behind, on purpose

`queue_prefetch` runs the search and stashes the text; `prefetch` serves it and searches
inline only when there is nothing warm.

The part the ABC docs do not mention, and the reason this is a trade rather than the
free win the review that asked for it assumed: Hermes calls `queue_prefetch_all` with
the turn that
just **finished** (`run_agent.py:4134`) and `prefetch_all` with the **new** message
(`turn_context.py:1165`). So the warm result answers the previous question. On a
conversation that stays on topic that is invisible; on an abrupt switch the first turn
on the new subject carries context for the old one.

Taken anyway, because inline retrieval costs more than the old `ponytail:` note
assumed: `SURREALFS_SEMANTIC=1` puts an embedding API round-trip on the critical path of
every turn, and `MemoryManager._prefetch_provider` (`memory_manager.py:568-576`,
verified) skips recall for a whole turn — silently, at `logger.debug` — whenever the
previous call is still running. One slow turn used to cost two.

Rejected: keying the cache on the query and only serving it on an exact match. Never
wrong, and close to useless — the two call sites above almost never pass the same
string, so every turn would fall through to the inline path and neither problem would
go away. Upstream's own providers take the same trade this does
(`plugins/memory/honcho/__init__.py:884`); ByteRover is the one that went the other way,
and made `queue_prefetch` an explicit no-op.

Staleness needs no turn counter — the counter Honcho keeps for this. The cache holds one
entry, reading clears it, and `on_session_switch` clears it again, so a warm result can
never be more than one turn old.

## The built-in memory mirror is state, not events

`on_memory_write` applies each delta to `/memory/<profile>/builtin/<target>.md`, so that
file says what Hermes' `MEMORY.md` / `USER.md` say. `add` appends, `replace` and `remove`
go through `fs.edit`, and an edit whose text was never mirrored is dropped rather than
re-added — that write predates the hook, and resurrecting text the user just removed is
worse than missing it.

Rejected: an append-only log, one file per write. It is not what `MEMORY.md` says, which
is what recall should surface, and a batched `operations:` call mirrors several ops
inside one millisecond — the exact filename collision the wall-clock stamp above exists
to avoid, arrived at from the other direction.

Hermes calls this **inline on the conversation thread**, once per op, unlike `sync_turn`
and `queue_prefetch` which get a background worker. One connection per op is the ceiling;
buffering is the upgrade if it ever shows up.

## `on_pre_compress` writes the messages out instead of summarising them

Every other provider can only hand the compressor a summary, which is what the
compressor is already doing. A filesystem-shaped memory can keep the text, so it does —
verbatim, tool calls included (which is also the one place `messages` is used at all,
see [improvements #4](hermes-memory-review-improvements.md)) — and returns a pointer to
the file for the summary prompt.

Filed through `path_for` with a `-compressed` session suffix, so it inherits the profile
subtree, the day folder and the collision-free stamp without a signature change. It must
stay a single write: the whole compression pass runs on a pooled daemon thread under a
host timeout, and work still in flight when it expires is discarded
(`conversation_compression.py:29-45`).

Consequence, worth knowing before it looks like a bug: these are long files inside the
tree recall reads, which feeds
[improvements #2](hermes-memory-review-improvements.md). BM25 favours short documents,
so they rank low rather than dominating — that is the only thing holding it, and it is
not a design.

## Every setting the provider reads is in the config schema

The upstream doc advises the opposite — prompt only for what the user must set,
document the rest — and that advice is aimed at providers with many options. There are
seven here, and the schema is load-bearing beyond setup: `cli.py:58-62` builds
`hermes surrealfs-memory status` by walking it, so a field demoted to README-only
vanishes from the one command whose job is diagnosing a broken install.

So: one rule, no tiers. `test_config_schema_covers_every_setting_the_provider_reads`
pins the env-var set, so adding a variable without a field fails there.

## The `surrealfs` imports are deferred into the methods that use them

`__init__.py` and `cli.py` import nothing from `surrealfs` at module level. That reads
like a tidiness lapse and is the reverse — it is what makes the provider installable at
all.

Bootstrap runs entirely through `pip_dependencies: [surrealfs]` in `plugin.yaml`:
`hermes memory setup` installs it into Hermes' own virtualenv
(`memory_setup.py:_install_dependencies`), and `hermes update` reinstalls it with
`force=True` after a venv sync strips or downgrades it (#53272, #70636). But the user
only reaches that install by picking this provider out of a list that
`_get_available_providers()` builds by *importing* each candidate and dropping the ones
that raise. A module-level `surrealfs` import therefore hides the provider from the
wizard on exactly the machines where the wizard was the point — and the README's old
manual `uv pip install --python ~/.hermes/hermes-agent/venv/bin/python` step existed
only to paper over that. `cli.py` is stricter still: Hermes imports it during argparse
setup, before the provider loads, inside a bare `except Exception`, so an import error
there deletes `hermes surrealfs-memory status` — the subcommand whose whole job is to
diagnose a broken install.

Moving the imports back to the top would pass every test that touches behaviour, which
is why `test_the_plugin_imports_without_surrealfs_installed` parses both files instead.

`is_available()` is the runtime half of the same decision. It reports whether those
deferred imports will resolve — `find_spec` on `surrealfs` **and** `surrealdb`, both,
because the `force=True` path above exists precisely for venvs left holding one of the
two. A constant `True` is worse than useless: `hermes memory` prints it verbatim as
`Status: available ✓` while every turn dies in a `logger.warning` nobody reads, since
`memory_manager.py` catches everything from `sync_turn` and `prefetch`. The reachability
probe the ABC forbids `is_available()` from making lives in `hermes surrealfs-memory
status` instead.

## `hooks:` in `plugin.yaml` is documentation, not wiring

Nothing in Hermes reads it — memory discovery takes only `description` out of that
file. The plugin docs ask for it and it is the only place the implemented set is
written down, so it stays; it just will not break anything if it drifts, and adding
a hook there does not register one.

`kind: exclusive` in the same file *is* wiring, and load-bearing
(`hermes_cli/plugins.py:1418`).

## The provider name is coupled to the install directory name, and stays that way

`PROVIDER_NAME = "surrealfs-memory"` must match whatever the user names the symlink,
because Hermes matches `memory.provider` against the *directory* name while
`MemoryManager` reports the *property*. A mismatch produces a confusing failure, and
both READMEs saying so is the right mitigation.

Recorded because the obvious reaction is to make the name dynamic — read it off
`__file__`'s parent, say. Don't: the name would then depend on how the plugin was
installed, and every error message, config file and test that names the provider would
have to agree with a value nobody can predict.

## Hooks left at the ABC's defaults, deliberately

Unimplemented is not the same as missing. Each of these is a decision:

- **`handle_tool_call`** — no tools. `get_tool_schemas()` returns `[]`; the 14
  `surrealfs_*` tools from the sibling plugin already cover explicit reads and writes,
  a second set would collide in Hermes' flat namespace, and `add_provider` reserves
  the core tool names anyway.
- **`save_config`** — env-var only. `memory_setup.py:399` writes every field carrying
  an `env_var` to `.env` itself, so a no-op loses nothing.
- **`shutdown`** — no long-lived connection to close. See below.
- **`backup_paths`** — nothing on disk. `hermes backup` walks `HERMES_HOME`; the
  memories are in SurrealDB, and the README says to use `surreal export`.
- **`on_turn_start`**, **`on_session_end`**, **`on_delegation`** — no per-turn state to
  tick, no end-of-session extraction to do (every turn is filed as it completes), and a
  subagent's work is not the user talking, which is the rule that already gates
  `sync_turn`.

## One connection per call, not a pooled one

`_connect.connected()` opens and closes per call, and that is not an oversight to
optimise away. `prefetch` runs on a fresh daemon thread every turn, `sync_turn` and
`queue_prefetch` on the memory worker, `on_memory_write` inline on the conversation
thread — and a SurrealDB WebSocket belongs to the event loop that opened it. A pooled
connection would be handed between loops that do not own it.

This is also why `shutdown()` has nothing to do.

## Two verified facts that read like guesses

Both were checked against Hermes' source, and both look wrong enough to "fix":

- **Memory providers are found by directory, never by entry point.** `PluginContext`
  has no `register_memory_provider`; `plugins/memory/__init__.py:_ProviderCollector` is
  what calls `register()`. The `hermes_agent.plugins` entry point in `pyproject.toml`
  covers only the sibling *tool* plugin.
- **The `MemoryProvider`-string guard** in `tests/test_memory_provider.py` is real, not
  paranoia. `hermes_cli/plugins.py:1749-1767` text-scans a plugin's `__init__.py` and
  coerces `kind="exclusive"` on a hit — so the string appearing in a *comment* in the
  tool plugin would silently unload its fourteen tools.
