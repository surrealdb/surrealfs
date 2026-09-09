# SurrealFS over MCP

An MCP server: the fifteen filesystem tools, over stdio as `surrealfs-mcp`. A
SurrealDB URL is the only thing it needs, and nothing here is specific to one
client — Claude Code, Claude Desktop, Cursor, Zed, Codex or a hand-rolled client
all get the same fifteen tools.

[Spectron](#optional-spectron-memory) is optional and off by default. Configure it
and a sixteenth tool appears, plus a mirror of every text file written through the
server; leave it alone and everything above still works.

**On Claude, install [the plugin](../claude/README.md) instead.** One command, no
admin rights, and it brings the skills with it. This page is the server
underneath: what it reads, what it offers, and how to check it works.

## Install

```bash
pip install "surrealfs[mcp] @ git+https://github.com/surrealdb/surrealfs.git"
```

Or nothing at all — `uvx` fetches the package on launch, which is what the
configs below do.

Until the first release, the `git+…` source is load-bearing: `surrealfs` is not
on PyPI, so a bare `surrealfs[mcp]` resolves to nothing. Drop it once it is.

## Wire it into a client

Every MCP client takes the same three fields. The shape:

```json
{
  "mcpServers": {
    "surrealfs": {
      "command": "uvx",
      "args": [
        "--from",
        "surrealfs[mcp] @ git+https://github.com/surrealdb/surrealfs.git",
        "surrealfs-mcp"
      ]
    }
  }
}
```

No `env` block, because none is needed: the server reads its own configuration
from `~/.config/surrealfs/env` (below) however it was launched. Add one only to
override that.

Where the file goes:

| Client | File |
|---|---|
| Claude Code | `.mcp.json` in the project, or `claude mcp add` — but prefer [the plugin](../claude/README.md) |
| Claude Desktop | `~/Library/Application Support/Claude/claude_desktop_config.json` (`%APPDATA%\Claude\` on Windows) — but prefer [the plugin](../claude/README.md) |
| Cursor | `~/.cursor/mcp.json`, or `.cursor/mcp.json` in the project |
| Codex CLI | `~/.codex/config.toml`, as a `[mcp_servers.surrealfs]` table |
| Zed | `context_servers` in `settings.json` |

One trap that is not ours and bites everywhere: some clients launch servers with
a **minimal `PATH`** that excludes `~/.local/bin`, so `"command": "uvx"` fails
with nothing written to any log. Give an absolute path if the server never starts.

## Configuration

Every setting is an environment variable, read from `~/.config/surrealfs/env` or
from the environment, whichever has it — the environment wins:

| Variable | Default | Purpose |
|---|---|---|
| `SURREALDB_URL` | none — the server refuses without it | server to connect to |
| `SURREALDB_USER` / `SURREALDB_PASS` | `root` / `root` | credentials |
| `SURREALDB_NAMESPACE` / `SURREALDB_DATABASE` | `surrealfs` / `demo` | where the `file` table lives |
| `SURREALFS_AGENT_USER` | your unix username | the agent's home under `/home` |
| `SURREALFS_SEMANTIC` | unset | `1` makes `search` match on meaning too |
| `SURREALFS_ENV_FILE` | `$XDG_CONFIG_HOME/surrealfs/env` | where the above are read from |

The four `SPECTRON_*` variables go in the same file — see
[Optional: Spectron memory](#optional-spectron-memory).

```bash
mkdir -p ~/.config/surrealfs
cat > ~/.config/surrealfs/env <<'EOF'
SURREALDB_URL=wss://your-instance.surreal.cloud/rpc
SURREALDB_USER=your-user
SURREALDB_PASS=your-password
SURREALDB_NAMESPACE=your-namespace
SURREALDB_DATABASE=your-database
SURREALFS_AGENT_USER=agent-yourname
EOF
chmod 600 ~/.config/surrealfs/env
```

A file, not exports, for two reasons. The server is launched by your MCP client
rather than by your terminal, so a project `.env` or a shell profile is not
reliably in scope. And `SURREALDB_URL` and friends are the names *every* SurrealDB
tool reads — exporting them to configure this one server would quietly repoint
your other work at the same database.

Anything that *is* set still wins, so a client's `env` block and a shell export
both keep working; the file is a default, not a mandate. `SURREALFS_ENV_FILE`
names a different path, and `XDG_CONFIG_HOME` is honoured. It holds a password
and an API key, hence the `chmod 600` — the server warns on stderr if it is
readable by anyone else.

With no config file and no `SURREALDB_URL` anywhere, the server refuses to start
and says so. That is deliberate: falling back to `ws://localhost:8000/rpc` and the
`demo` database would connect *successfully* to an empty filesystem, and an agent
that finds an empty brain reports a clean bill of health for a company it never
reached.

`SURREALFS_SEMANTIC` needs `OPENAI_API_KEY` and the `embed` extra, and only pays
off once the vectors exist — see
[semantic search](https://github.com/surrealdb/surrealfs/blob/main/docs/semantic-search.md).

## The tools

The same fifteen as every other surface, generated from `surrealfs/tools/`, and
**unprefixed** — unlike the Hermes plugin's `surrealfs_*`. An MCP client namespaces
tools by server, so a prefix here would read as `surrealfs:surrealfs_ls`.

With Spectron configured there is a sixteenth, `brain_recall` — see below.

## Optional: Spectron memory

Spectron is SurrealDB's hosted memory layer, and it is a managed service: it needs
a context id and an API key. **Everything above works without it.** Set no key and
the server is a filesystem server, does not advertise `brain_recall` at all, and
says so once on stderr at startup.

To turn it on, install the extra and add two variables to the same config file:

```bash
pip install "surrealfs[mcp,spectron] @ git+https://github.com/surrealdb/surrealfs.git"
```

```bash
SPECTRON_CONTEXT_ID=your-context
SPECTRON_API_KEY=your-key
```

| Variable | Default | Purpose |
|---|---|---|
| `SPECTRON_CONTEXT_ID` | unset | the context (tenant) to file into |
| `SPECTRON_API_KEY` | unset | bearer token |
| `SPECTRON_URL` | `https://srv1.spectron.aws-usw2.surreal.cloud` | Spectron host |
| `SPECTRON_SCOPE` | `brain` | scope path to file under and lens queries to |
| `SURREALFS_MIRROR_ROOT` | `/` | only mirror writes under this path |

A key with the extra missing is refused loudly, naming the extra — it is not
treated as "unconfigured", because a memory layer that silently files nothing for
someone who paid for one is the worse failure.

### `brain_recall`, the sixteenth tool

It queries Spectron, not the filesystem: it answers what the current files no
longer say — a superseded version of a file, entities and relationships extracted
out of the prose, context filed by a session you never saw. It takes a question,
not a keyword, and each hit is labelled with the SurrealFS path it was filed from
so you can open the file next.

### Mirroring on write

`write_file`, `edit` and `touch` upload the file's new contents to Spectron as a
document, in the same call. Not a separate tool the model is asked to call
afterwards: a memory layer that depends on remembering to update it is not one.
`write_bytes` is left out — Spectron indexes prose, and a base64 PNG is not prose.

Uploads deduplicate by content hash, so re-mirroring an unchanged file creates
nothing. A file that *did* change becomes a second document rather than replacing
the first, which is the point: the superseded version stays recallable after the
filesystem has moved on.

A mirror failure is appended to the tool's result — `(brain sync failed: …)` —
rather than raised. The write to SurrealFS has already happened and must not be
rolled back over it, but swallowing the failure would let the memory layer drift
out of date with nobody the wiser.

Set `SURREALFS_MIRROR_ROOT=/brain` to confine mirroring to the brain and keep the
rest of the filesystem out of Spectron.

## Checking it works

```bash
surrealfs-mcp --selftest
```

It prints the config file it read, the server and database it reached, the
identity it acts as, the tool count, `ls /`, and — if Spectron is configured —
whether it answers, then `OK` or a line naming the layer that failed. Exit code 0 only when
everything it checked worked.

It exists because every misconfiguration has the same symptom: the model says it
has no such tools. That one message covers a client that never launched the
server, a missing config file, an unreachable database, a wrong namespace, and a
Spectron key that does not work (when there is one). The selftest separates them.

An empty database is reported as a **failure**, not a success, even though the
connection worked:

```
ls /      home

WARN      connected, but this database is empty apart from /home.
          Check SURREALDB_NAMESPACE and SURREALDB_DATABASE -- a
          wrong pair connects successfully to the wrong filesystem.
```

That is the one failure worth being paranoid about. A wrong namespace connects,
authenticates and lists nothing, so an agent reports a clean brain for a database
it never reached.

If the selftest passes but a client still shows no tools, the client never
launched the server — look for its log. Diagnostics go to stderr, never to stdout,
which is the transport. See [the plugin README](../claude/README.md) for where
Claude Code and Claude Desktop file theirs.

The two launch failures that write nothing anywhere: a bare `"command": "uvx"`
against a minimal `PATH`, and a `--from` spec the client did not expand (a
plugin's `.mcp.json` supports `${VAR}` but **not** `${VAR:-default}`, so a
default written there arrives verbatim and `uvx` exits with `Failed to parse`).
Run the client's exact `command` and `args` in a terminal to see which.

## Example

[`examples/company-brain/`](https://github.com/surrealdb/surrealfs/tree/main/examples/company-brain)
— a devsecops company brain: two Claude Desktop routines file Snyk, Drata,
SonarQube, Okta and Slack state into `/brain/acme/`, and `/brain plan my
next high-priority task` answers from the files, plus Spectron if it is
configured. Runs with no vendor accounts.

## See also

- [The Claude plugin](../claude/README.md) — this server bundled with the `/brain`
  and `/brain-memory` skills, installable in one command with no admin rights.
- [SurrealFS as a Hermes toolset](../hermes/README.md) — the same tools for
  Hermes, where they *are* prefixed and there is no MCP.
- [SurrealFS as Hermes memory](../hermes_memory/README.md) — automatic
  turn-by-turn filing, the closest thing to this one's mirror.
