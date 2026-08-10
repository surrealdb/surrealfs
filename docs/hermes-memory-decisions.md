# Hermes memory provider — decisions

Choices made in `surrealfs/integrations/hermes_memory/` that a future change could
plausibly undo without realising what it was for. Rationale that fits in a docstring
lives in the docstring; this file is for the parts that span files, and for the
alternatives that were considered and rejected.

Open work is in [wrong](hermes-memory-review-wrong.md),
[missing](hermes-memory-review-missing.md) and
[improvements](hermes-memory-review-improvements.md).

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

Consequence for [missing #1](hermes-memory-review-missing.md) (`on_session_switch`):
there is no per-session state left to invalidate except `_session_id`, so that hook
is now a one-line reassignment rather than a buffer flush.

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
[improvements #3](hermes-memory-review-improvements.md), down-weighting the memory
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

## `hooks:` in `plugin.yaml` is documentation, not wiring

Nothing in Hermes reads it — memory discovery takes only `description` out of that
file. The plugin docs ask for it and it is the only place the implemented set is
written down, so it stays; it just will not break anything if it drifts, and adding
a hook there does not register one.

`kind: exclusive` in the same file *is* wiring, and load-bearing
(`hermes_cli/plugins.py:1418`).
