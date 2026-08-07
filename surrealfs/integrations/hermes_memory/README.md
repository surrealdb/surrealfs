# SurrealFS as Hermes memory

Files every completed turn into a SurrealDB-backed filesystem, and recalls context
before each turn by searching it.

## Install

Hermes finds memory providers by scanning directories, not through the
`hermes_agent.plugins` entry point, so this one is installed by symlink even when the
package is pip-installed. The directory name is the provider name.

```bash
ln -s "$PWD/surrealfs/integrations/hermes_memory" ~/.hermes/plugins/surrealfs-memory
```

`surrealfs` also has to be importable by Hermes' own interpreter
(`~/.hermes/hermes-agent/venv/bin/python`).

Then activate it in `~/.hermes/config.yaml` — only one memory provider can be active
at a time:

```yaml
memory:
  provider: surrealfs-memory
```

Confirm with `hermes memory`, which lists discovered providers and whether each
reports itself available.

## Configuration

All environment variables, so there is no config file to write:

| Variable | Default | Purpose |
|---|---|---|
| `SURREALDB_URL` | `ws://localhost:8000/rpc` | server to connect to |
| `SURREALDB_USER` / `SURREALDB_PASS` | `root` / `root` | credentials |
| `SURREALDB_NAMESPACE` / `SURREALDB_DATABASE` | `surrealfs` / `demo` | where the `file` table lives |
| `SURREALFS_MEMORY_DIR` | `/memory` | folder to file turns under |
| `SURREALFS_SEMANTIC` | unset | `1` makes recall match on meaning too |

`SURREALFS_SEMANTIC` needs `OPENAI_API_KEY` and the `embed` extra, and only pays off
once the vectors exist — run `python -m surrealfs.embed` to keep them current.

## What it does

**Capture.** Each turn becomes its own file:

```
/memory/2026-08-06/a1b2c3-004.md
```

```markdown
# Turn 4 — 2026-08-06 14:22 UTC

session: a1b2c3

## User

how do I get paid for the consulting work

## Assistant

You invoice monthly through ...
```

One file per turn rather than one per session, because a search hit is then a single
exchange with its snippet centred on the match. Turns whose prompt carries no signal
— "ok", "thanks", slash commands — are skipped.

**Recall.** Before each turn, the provider searches the **whole** filesystem, not just
`/memory`. The notes the agent wrote itself under `/preferences/` and `/projects/` are
the highest-signal memories present, and excluding them would mean the bundled
`surrealfs:notes` skill and this provider ignored each other's work.

**No tools of its own.** `get_tool_schemas()` returns nothing: the 14 `surrealfs_*`
tools from the sibling plugin already cover explicit reads and writes, and a second
set would collide in Hermes' flat tool namespace.

## Relationship to the tool plugin

Independent installs. The tool plugin
(`surrealfs.integrations.hermes`) registers tools and a skill through the entry point;
this registers a memory provider by directory. Either works without the other, though
they are better together — the provider's recall surfaces what the agent wrote with
the tools.
