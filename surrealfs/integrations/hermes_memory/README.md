# SurrealFS as Hermes memory

Files every completed turn into a SurrealDB-backed filesystem, and recalls context
before each turn by searching it.

## Install

No clone needed: `hermes plugins install` accepts a subdirectory of a repo and names what
it installs from `plugin.yaml`, which is how the directory lands as `surrealfs-memory` —
the name Hermes matches the active-provider setting against.

```bash
hermes plugins install surrealdb/surrealfs/surrealfs/integrations/hermes_memory
uv pip install --python ~/.hermes/hermes-agent/venv/bin/python \
  "surrealfs @ git+https://github.com/surrealdb/surrealfs.git"
hermes memory setup surrealfs-memory
```

Two installs, because they are two different things: the *directory* is what Hermes scans
to discover memory providers, and the *package* is what the provider imports. Nothing
merges those — providers are found by directory, never through the `hermes_agent.plugins`
entry point. The first command prints the second, from the `python_dependencies` entry in
`plugin.yaml`, and runs nothing: Hermes never installs a plugin's Python dependencies for
it, and the `pip_dependencies` entry it *would* have installed cannot name a git URL
(specs containing `@` or `://` are rejected as unsafe), so while `surrealfs` is unpublished
that install is yours to run. Publication closes the hole; nothing here changes.

Answer **N** to install's `Enable now?` prompt. That is the general plugin manager, which
this directory does not go through — `kind: exclusive` routes it to memory discovery, and
`hermes memory setup` is what activates it.

Setup is where the questions are: it walks every setting in the table below, offering the
current value or the default, writes what you change to `$HERMES_HOME/.env`, activates the
provider in `~/.hermes/config.yaml` — only one can be active at a time — and then
*connects*, so a wrong URL surfaces there rather than as a per-turn warning nobody reads.
Activating by hand is:

```yaml
memory:
  provider: surrealfs-memory
```

`hermes memory` lists discovered providers and whether each reports itself available;
`hermes surrealfs-memory status` re-runs the connection check any time, naming the
database it resolved and counting the turns filed there.

Upgrades are two commands for the same reason installs are: `hermes plugins update
surrealfs-memory` for the directory, and the `uv pip install` again for the package.
Rerun the second whenever `hermes memory` starts reporting the provider unavailable — a
venv rebuild strips it, and `hermes update` cannot heal a dependency it may not fetch.

Working on SurrealFS itself? Take both from a checkout instead — an editable install, and
a symlink in place of the copy `hermes plugins install` makes:

```bash
git clone https://github.com/surrealdb/surrealfs.git ~/repos/surrealfs
uv pip install --python ~/.hermes/hermes-agent/venv/bin/python -e ~/repos/surrealfs
ln -s ~/repos/surrealfs/surrealfs/integrations/hermes_memory \
  "${HERMES_HOME:-$HOME/.hermes}/plugins/surrealfs-memory"
hermes memory setup surrealfs-memory
```

## Configuration

All environment variables, so there is no config file to write:

| Variable | Default | Purpose |
|---|---|---|
| `SURREALDB_URL` | `ws://localhost:8000/rpc` | server to connect to |
| `SURREALDB_USER` / `SURREALDB_PASS` | `root` / `root` | credentials |
| `SURREALDB_NAMESPACE` / `SURREALDB_DATABASE` | `surrealfs` / `demo` | where the `file` table lives |
| `SURREALFS_AGENT_USER` | your unix username | the agent's home under `/home` |
| `SURREALFS_MEMORY_DIR` | `/home/<agent>/memories` | folder to file turns under |
| `SURREALFS_SEMANTIC` | unset | `1` makes recall match on meaning too |

`SURREALFS_SEMANTIC` needs `OPENAI_API_KEY` and the `embed` extra, and only pays off
once the vectors exist — see below.

Set `SURREALFS_AGENT_USER` whenever two agents would otherwise share a home — two
Hermes installs owned by different people against one database, or an agent running
under the same account as its human. `hermes-martin` alongside the human's own
`/home/martin` is the shape to aim for. On a VM or sandbox where the agent has an
account to itself, the default already matches the machine.

## Keeping the vectors current

Skip this unless `SURREALFS_SEMANTIC=1`. Nothing embeds a file as it is written, so
recall matches on meaning only for what the indexer has already reached — recall keeps
working on its full-text arm the whole time the vectors are behind. Scheduling the
indexer is one Hermes cron job, shared with the tool plugin:
[Keeping the vectors current under
Hermes](https://github.com/surrealdb/surrealfs/blob/main/docs/hermes-indexer.md).

## What it does

**Capture.** Each turn becomes its own file:

```
/home/hermes-martin/memories/default/2026-08-06/a1b2c3-142251003.md
```

```markdown
# 2026-08-06 14:22:51 UTC

session: a1b2c3

## User

how do I get paid for the consulting work

## Assistant

You invoice monthly through ...

## Tools

→ surrealfs_search({"query": "invoice"})
→ surrealfs_read({"path": "/projects/acme/contract.md"})
```

One file per turn rather than one per session, because a search hit is then a single
exchange with its snippet centred on the match. The `## Tools` section — which files
were read, which commands ran — is often the substance of the turn and appears in
neither message's text; it is omitted when the turn made no tool calls.

Turns where **both** sides carry no signal are skipped: a prompt like "ok", "thanks"
or a slash command, answered briefly. A trivial prompt with a long answer is still
filed, because "go ahead" followed by a design document is a turn worth keeping. So is
nothing that isn't a primary agent: a cron run's system prompt filed as a memory would
corrupt what the agent believes about the user.

Everything written here goes to the SurrealDB in your `SURREALDB_URL` and nowhere
else. That is worth saying out loud because "memory provider" now reads as "cloud" to
most people, and because Hermes hands providers the turn's full message list: this one
writes part of it (the tool calls above) to your own database, and it leaves your
machine only if you pointed it at a remote server yourself.

The name is a wall-clock stamp (`HHMMSSmmm`) rather than a turn number. A counter has
to live somewhere, and in-process is the one place it cannot: `hermes --resume` starts
a second process on the same session id, restarts counting at one, and a write
replaces — so turn one of the resumed conversation overwrote turn one of the original.

Two other things get filed, both under the same profile subtree:

- **Hermes' own memory.** When the built-in memory tool writes to `MEMORY.md` or
  `USER.md`, the same change is applied to `<memories>/<profile>/builtin/memory.md` or
  `.../user.md`. That is a mirror of what those files *say*, not a log of the writes,
  so recall reads the current state. Without it, Hermes' memory and SurrealFS would
  diverge permanently and recall would never see either file.
- **Messages about to be compressed away.** Before Hermes compresses a long
  conversation it asks each provider what to preserve; everyone else can only
  summarise, which is what the compressor is already doing. This writes the doomed
  messages out verbatim — tool calls included — and hands the compressor a pointer to
  the file, so the detail survives at full fidelity and the agent can read it back
  with `surrealfs_read`.

**Recall.** Before each turn, the provider searches the whole filesystem, not just its
own folder — everything except other agents' homes. The notes the agent wrote itself
under `/preferences/` and `/projects/` are the highest-signal memories present, and
excluding them would mean the bundled `surrealfs:notes` skill and this provider ignored
each other's work.

Which is also why filed turns are held to two of the five recalled slots, and why the
current conversation's own turns are never recalled at all. Recall reads the tree it
writes: one turn per file means short documents, which the ranking favours, so without
a cap the top five drifts into verbatim transcript and the curated notes — the point of
the whole thing — stop showing up.

The prompt is cut down to search terms before it goes anywhere near the index —
stopwords out, a dozen terms at most. A whole paragraph is not a query: every word in
it is another `OR` branch, and matching rows are read back in full to be ranked.

The search runs after the *previous* turn rather than during this one — Hermes' own
`queue_prefetch` / `prefetch` pair — so nothing waits on it, in particular not the
embedding round-trip `SURREALFS_SEMANTIC=1` adds. The cost is that a recall is one turn
behind: switch topic abruptly and the first turn on the new subject still carries
context for the old one.

## Homes and profiles

Turns are filed under `/home/<agent>/memories/<profile>/`. Two segments, because there
are two ways for memories to end up in one database that should not mix.

`<agent>` is `SURREALFS_AGENT_USER`, or the unix account the agent runs as. A database
shared between people holds one home per agent, plus the humans' own — the notes skill
already treats `/home/<your username>/` as the agent's working directory, so the
memories sit inside the folder the agent already owns. Recall reads that home and
nothing under any other.

`<profile>` comes from the `hermes_home` Hermes passes to `initialize()` — `default`
for `~/.hermes`, the directory name for anything under `<root>/profiles/`. It separates
one agent's own profiles, which share a home and a machine account.

Everything outside `/home/` stays shared: `/preferences/`, `/projects/`, and anything
else written with the `surrealfs_*` tools. That is the handover point — how an agent
gives something to its user, or to another agent working for them — and hiding it would
defeat the point of recalling it. Anything that must not be shared at all goes in a
`SURREALDB_DATABASE` of its own; `.env` lives in `$HERMES_HOME`, so that is already
per-profile.

Note that `hermes backup` captures none of this: it walks `HERMES_HOME`, and the
memories are in SurrealDB. Use `surreal export`.

**No tools of its own.** `get_tool_schemas()` returns nothing: the 14 `surrealfs_*`
tools from the sibling plugin already cover explicit reads and writes, and a second
set would collide in Hermes' flat tool namespace.

## Relationship to the tool plugin

Independent installs. The [tool
plugin](https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/hermes/README.md)
registers tools and a skill through the entry point;
this registers a memory provider by directory. Either works without the other, though
they are better together — the provider's recall surfaces what the agent wrote with
the tools.
