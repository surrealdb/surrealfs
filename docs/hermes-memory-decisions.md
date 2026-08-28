# Hermes memory provider — decisions

Choices made in `surrealfs/integrations/hermes_memory/` that a future change could
plausibly undo without realising what it was for. Rationale that fits in a docstring
lives in the docstring; this file is for the parts that span files, and for the
alternatives that were considered and rejected.

Written against
[Building a Memory Provider Plugin](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/memory-provider-plugin.md)
and the sources it describes — `agent/memory_provider.py`, `agent/memory_manager.py`,
`plugins/memory/__init__.py`, `hermes_cli/memory_setup.py`, `hermes_cli/plugins.py` —
which is where the line references below point. Where the published doc and those
sources disagree, the sources win: the doc omits `on_session_switch` entirely and shows
`on_memory_write` without its `metadata` parameter.

## Turns are named by wall clock, not by a turn number

`/home/<agent>/memories/<profile>/<day>/<session>-HHMMSSmmm.md`.

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

## The memory root is a home, and the home belongs to the agent

`/home/<agent>/memories/`. The alternative was a single global `/memory/`, which is
what this was, and it holds only while one person owns the database. Shared, every
user's raw transcripts land in one tree and recall hands one person's conversations
to somebody else's agent.

**The home is the agent's, not the human's.** The notes skill
(`integrations/hermes/skills/notes/SKILL.md`) already gave the agent a home at
`/home/<your name>/` and showed `/home/hermes/todo.md` — so two Hermes instances owned
by two different people were already colliding there, before memories were involved.
Naming the home after the agent fixes the pre-existing collision and gives the
memories somewhere to live in one move: they go in the folder the agent already owns.
`/home/martin` is then the human's, distinct from the `hermes-martin` that serves them.

**The default is the bare machine username**, `getpass.getuser()`. On a VM or sandbox
the agent has an account of its own, so the SurrealFS path matches the machine — which
is the case worth optimising, because it needs no configuration at all. Rejected: an
always-prefixed `hermes-<user>`, which never collides with a human but produces
`hermes-hermes` on exactly that deployment. Where the default is wrong — an agent
sharing a laptop account with its human, two agents under one account —
`SURREALFS_AGENT_USER` names it, and `hermes memory setup` writes it into the
profile's own `.env`.

`SURREALFS_MEMORY_DIR` still overrides the whole path for anyone who wants turns
outside `/home` altogether. `_is_mine` is written to stay correct when it points
somewhere with no home in it.

**Recall dropped whole homes, not just their `memories/`.** The narrower rule was
tempting — it preserves the "notes are shared, transcripts are not" line below
verbatim — but a home is working space: an agent's scratch, to-dos, and half-finished
drafts are not addressed to anyone else, and on a shared database "anyone else"
now means other people. Everything outside `/home/` stays shared, and that is where
the notes skill already points curated notes, so the handover path is unchanged.

*Superseded:* homes are now enforced by the filesystem — `/home/<user>` is created
`0700`, so another agent's home never reaches `_is_mine` in the first place, and
`search` cannot return one. The clause was deleted rather than left as dead code.
What stays is the sibling-*profile* rule below: those share an owner, so no file
permission can tell them apart. See `docs/permissions.md`.

Legacy `/memory/` is excluded explicitly rather than migrated. Left in, it would not
merely go quiet: it sits outside the new memories dir, so `_rank` would read those
transcripts as *notes* and hand them the high-priority slots reserved against exactly
that failure.

## Profiles scope the turn subtree, not the database

`initialize()`'s `hermes_home` kwarg picks a segment: `<root>/profiles/<name>` →
`<name>`, anything else (including `~/.hermes` and an absent kwarg) → `default`.
Turns are written under `<memories>/<profile>/`, and `_is_mine` drops other profiles'
folders from recall.

Both segments earn their place: the home separates agents, which have different
owners — and now literally different file owners; the profile separates one agent's
own configurations, which share an account and a home, and therefore share a file
owner too. That is precisely why the profile rule cannot be handed to the
filesystem the way the home rule was.

Three things this deliberately does not do:

**It does not scope the database.** That would isolate harder, and it would also cut
recall off from the notes the agent writes with the `surrealfs_*` tools, which read
`SURREALDB_DATABASE` unscoped and have no way to learn the profile. Those notes are
the highest-signal memories in the tree — losing them defeats the design. Users who
want database-level separation already have it: `.env` lives in `$HERMES_HOME`, so
`SURREALDB_DATABASE` is per-profile today, and `hermes memory setup` writes it there.

**It does not hide tool-written notes outside `/home/`.** They are the user's own
curated files in a database the user chose, and the shared root is where an agent
hands something over. Raw transcripts are the part that must not cross over. (Inside
`/home/`, the home rule above is stricter and does hide them.) If this ever needs to
change, `_is_mine` is still the single place.

**It does not leave the default profile unprefixed.** That was less churn, but it
isolates in one direction only: `<memories>/<day>/` for the default and
`<memories>/<name>/<day>/` for the rest means the default profile still recalls every
named profile's transcripts. Symmetry is worth the extra path segment.

`_recall` over-fetches `RECALL_LIMIT * 4` so the filtering cannot cost a result slot —
`_rank` below drops whole classes of hit, not the odd one. Nearly free — `search_text`
reads every matching row's content regardless of the limit (see `fs.py:586`).

## Recall reserves slots rather than excluding the memory tree

`_rank` drops the current conversation's own turns outright, then lets filed turns take
at most `MEMORY_SLOTS` (2) of `RECALL_LIMIT` (5). Notes keep their fused rank; turns are
appended after them, never interleaved.

The problem is that recall reads the tree it writes. One turn per file means short
documents, which BM25 favours, so after a few sessions the top five is verbatim
transcript rather than the `/preferences/` and `/projects/` notes the README calls the
whole point. The worst case is a session's own first turn outranking everything on the
query that produced it, which is why `_is_this_session` exists as a separate, absolute
filter — a conversation quoting itself back is never useful, at any rank.

It matches on the **basename**, not the path prefix: `path_for` puts the session in the
filename and the date in the folder, so a conversation running past midnight, or a
resumed one, spans day folders under a single id. It must also return `False` for an
empty session id, since `_slug("")` is `"session"` and would filter unrelated files.

Rejected: a **hard partition**, every note above every filed turn. With ten over-fetched
hits, a note-rich tree would mean a past turn is never recalled at all — and sometimes a
past turn is the answer. Also rejected: **excluding the current session only**, the cheap
version. It fixes the worst case above and leaves the drift, which is the part that gets
worse every session.

Consequence worth knowing: the `-compressed` transcripts are caught by both filters —
by the basename test while their session is live, by the slot cap afterwards. They used
to rank low purely because BM25 penalises long documents, which was luck, not a design.

## The recall query is trimmed, the search tool's is not

`_recall_query` strips stopwords and caps at `RECALL_TERMS` (12) before searching;
`search_text` still binds the raw string for callers that pass one. The asymmetry is
the point: a human types three words into the search tool, while recall gets a whole
prompt, on every turn, on the thread `MemoryManager` joins with an 8s timeout. Every
word left in is another `OR` branch, and `search_text` reads the content of every row
any branch matched back into Python to rank it (`fs.py:586`).

It borrows `fs._scoring_terms` rather than keeping its own stopword list — that
function already drops them in written order, and keeps them all when a query is
nothing else, so `"the who"` still searches for something. Copying the list would be
one import fewer and one more thing to drift.

What this does **not** buy is much ranking change: BM25 already scores stopwords at
zero. The win is fetch volume, latency, and a tail of zero-score matches that no longer
exists. Worth stating, because the obvious next step — trimming inside `search_text`
for everyone — would change the search tool's semantics and move
`test_ranking_quality_over_a_realistic_corpus`, for a gain that was never the ranking.

The embedding is still built from the whole sentence. Reading one is the single thing
that arm does better.

## Only primary agents write

`sync_turn` returns early unless `agent_context` is `"primary"`. Subagent, cron and
flush turns are not the user talking, and the ABC is explicit that filing them
corrupts what the agent believes about the user.

The other gate is on the exchange, not the prompt: a trivial prompt is filed anyway
when the answer runs past `TRIVIAL_ANSWER_CHARS` (400). Gating on
`is_trivial_prompt(user_content)` alone dropped "go ahead" followed by a 2000-word
design document — the prompt is where the signal usually is, but not where the content
is. Length as a proxy for signal is crude and cheap; the alternative is classifying the
answer, on the turn-completion path, for a decision that is already best-effort.

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

`on_memory_write` applies each delta to `<memories>/<profile>/builtin/<target>.md`, so that
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
verbatim, tool calls included — and returns a pointer to the file for the summary
prompt.

Filed through `path_for` with a `-compressed` session suffix, so it inherits the profile
subtree, the day folder and the collision-free stamp without a signature change. It must
stay a single write: the whole compression pass runs on a pooled daemon thread under a
host timeout, and work still in flight when it expires is discarded
(`conversation_compression.py:29-45`).

These are long files inside the tree recall reads; `_rank` above is what keeps them
from dominating it, rather than the incidental fact that BM25 penalises long documents.

## `sync_turn` takes the turn's tool calls out of `messages`

`messages` is the **whole conversation**, not the turn — `turn_finalizer.py:707` →
`run_agent.py:4075` → `memory_manager.py:675`, and Hermes only passes it because
`_provider_sync_accepts_messages` inspects the signature and finds the parameter there.
So `_tool_lines` walks back to the last `role == "user"` message and reads only what
follows it; without that slice every turn file re-lists the entire session.

It writes tool *calls*, not tool results: which files were read and which commands ran
is frequently the substance of a turn and appears in neither message's text, while the
results are the bulk of the tokens and mostly reproducible from the call. Same
`→ name(args)` shape `_render` writes for compression, so the two file kinds read alike;
capped at `TOOL_LINES` (10) and `TOOL_LINE_CHARS` (200) because a single write call's
arguments can be an entire file, and this rides on every turn.

The upstream doc asks providers to "document what parts of `messages` are sent
off-device". Nothing is: it goes to the user's own SurrealDB. The README says so
explicitly anyway, because "memory provider" reads as "cloud" to most people now.

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

## The fallback `_TRIVIAL_RE` is known drift, and stays

`agent/memory_provider.py` says of `TRIVIAL_PROMPT_RE`: "Single source of truth shared
by the core per-turn prefetch gate and provider-side classifiers so the two can never
drift apart." The copy in the `ImportError` branch is a second one — but it only runs
where the import fails, which is outside Hermes: tests, linting, a plain `pip install`.

The trailing-punctuation class differs (`[\W_]*` against upstream's explicit set, so the
local one is strictly wider), and no input has been found where they disagree. Accepted,
because the alternative is skipping the gate entirely when the import fails, and a
provider that files every "thanks" outside Hermes is worse than one whose regex is a
shade loose there.

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
