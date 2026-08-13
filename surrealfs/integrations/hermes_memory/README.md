# SurrealFS as Hermes memory

Files every completed turn into a SurrealDB-backed filesystem, and recalls context
before each turn by searching it.

## Install

Hermes finds memory providers by scanning directories, not through the
`hermes_agent.plugins` entry point, so this one is installed by symlink even when the
package is pip-installed. The directory name is the provider name.

```bash
ln -s "$PWD/surrealfs/integrations/hermes_memory" \
  "${HERMES_HOME:-~/.hermes}/plugins/surrealfs-memory"
hermes memory setup surrealfs-memory
```

`setup` installs `surrealfs` into Hermes' own virtualenv — it has to be importable by
`~/.hermes/hermes-agent/venv/bin/python`, and `plugin.yaml` declares it as a
`pip_dependencies` entry so `hermes update` reinstalls it after a venv rebuild. Running
from a checkout is the one case to do by hand, since that installs the *published*
package over your working copy:

```bash
uv pip install --python ~/.hermes/hermes-agent/venv/bin/python -e .
```

Setup also activates the provider in `~/.hermes/config.yaml` — only one memory provider
can be active at a time, and the equivalent by hand is:

```yaml
memory:
  provider: surrealfs-memory
```

Confirm with `hermes memory`, which lists discovered providers and whether each
reports itself available, then `hermes surrealfs-memory status`, which actually connects
— names the database it resolved and counts the turns filed there.

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
recall matches on meaning only for what the indexer has already reached. Hermes has
no way for a plugin to declare a daemon of its own, but its scheduler runs scripts
with no agent attached, which saves you a systemd unit:

```bash
mkdir -p "${HERMES_HOME:-$HOME/.hermes}/scripts"
cat > "${HERMES_HOME:-$HOME/.hermes}/scripts/surrealfs-embed.sh" <<'EOF'
#!/usr/bin/env bash
# Cron sanitizes the subprocess env and OPENAI_API_KEY is on the blocklist, so the
# key has to come from a file — as do the SURREALDB_* connection details.
set -a; . "$HOME/repos/surrealfs/.env"; set +a
exec uv run --project "$HOME/repos/surrealfs" --extra embed \
  python -m surrealfs.embed --once
EOF

hermes cron create 'every 5m' --no-agent --name surrealfs-embed \
  --script surrealfs-embed.sh
```

`--no-agent` skips inference entirely — no tokens, no model — so a `--once` pass per
tick replaces the polling loop. The script has to live under `$HERMES_HOME/scripts/`;
paths outside it are rejected. Pip-installed rather than a checkout? Point the script
at Hermes' own interpreter instead, installed with the extra
(`"surrealfs[embed] @ git+..."`):

```bash
exec ~/.hermes/hermes-agent/venv/bin/python -m surrealfs.embed --once
```

An idle pass prints nothing and empty stdout is a silent tick, so this speaks up only
with news (`embedded 4 file(s)`) or a non-zero exit, which arrives as an alert instead
of being swallowed. `hermes cron runs` has the history.

The scheduler ticks inside the gateway daemon, so this runs only while that does
(`hermes gateway install`), and the period is how long a new file takes to become
recallable *by meaning* — it is matchable by term immediately, and recall keeps
working on its full-text arm the whole time the vectors are behind.

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

Independent installs. The tool plugin
(`surrealfs.integrations.hermes`) registers tools and a skill through the entry point;
this registers a memory provider by directory. Either works without the other, though
they are better together — the provider's recall surfaces what the agent wrote with
the tools.
