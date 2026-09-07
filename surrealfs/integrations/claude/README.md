# SurrealFS for Claude

An MCP server: the fifteen filesystem tools, plus `brain_recall` for the Spectron
memory layer behind them. Every text file written through it is mirrored into
Spectron on the way past.

It ships two ways. **As a plugin** — one command, no admin rights, works in Claude
Code and Claude Desktop, and brings the `/brain` skill with it. Or **as a bare MCP
server** you wire into `claude_desktop_config.json` yourself, which needs
permission to add a connector.

## Install as a plugin

This is the one to use, and the only one that works if you are on a Team or
Enterprise plan without owner rights: plugin-provided MCP servers need no admin
permission, where adding a connector does.

```
/plugin marketplace add surrealdb/surrealfs
/plugin install surrealfs@surrealfs
```

The plugin bundles the MCP server and the `/brain` skill together, so there is
nothing else to install and no client config to edit. Its tools arrive namespaced —
`mcp__plugin_surrealfs_surrealfs__ls` and so on — and the skill as
`/surrealfs:brain`.

Then write the configuration to `~/.config/surrealfs/env`:

```bash
mkdir -p ~/.config/surrealfs
cat > ~/.config/surrealfs/env <<'EOF'
SURREALDB_URL=wss://your-instance.surreal.cloud/rpc
SURREALDB_USER=your-user
SURREALDB_PASS=your-password
SURREALDB_NAMESPACE=your-namespace
SURREALDB_DATABASE=your-database
SURREALFS_AGENT_USER=claude-yourname

SPECTRON_CONTEXT_ID=your-context
SPECTRON_API_KEY=your-key
EOF
chmod 600 ~/.config/surrealfs/env
```

A file, not exports, for two reasons. The server is launched by Claude rather
than by your terminal, so a project `.env` or a shell profile is not reliably in
scope. And `SURREALDB_URL` and friends are the names *every* SurrealDB tool
reads — exporting them to configure this one server would quietly repoint your
other work at the same database.

Nothing about SurrealDB needs to be in the environment, and the plugin's
`.mcp.json` deliberately passes no `env` block at all. Anything that *is* set
still wins, so Claude Desktop's `env` block and a shell export both keep working;
the file is a default, not a mandate. `SURREALFS_ENV_FILE` names a different path,
and `XDG_CONFIG_HOME` is honoured.

With no config file and no `SURREALDB_URL` anywhere, the server refuses to start
and says so. That is deliberate: falling back to `ws://localhost:8000/rpc` and the
`demo` database would connect *successfully* to an empty filesystem, and an agent
that finds an empty brain reports a clean bill of health for a company it never
reached.

The file holds a password and an API key, hence the `chmod 600` — the server warns
on stderr if it is readable by anyone else.

### Installing from a local checkout

Before this is pushed, or while working on it, nothing needs editing — the
package source in
[`.mcp.json`](https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/claude/.mcp.json)
is `${SURREALFS_SOURCE:-git+https://github.com/surrealdb/surrealfs.git}`, so one
variable redirects it at your working tree:

```bash
export SURREALFS_SOURCE=/path/to/surrealfs   # in your shell profile

claude plugin marketplace add /path/to/surrealfs
claude plugin install surrealfs@surrealfs
```

`SURREALFS_SOURCE` is the one variable that *must* be exported rather than
filed: it decides what `uvx` builds, so it is read before the server exists to
load anything. It is also ours alone, so it cannot clash with another tool.

A marketplace can be a local directory, so `source` resolves against the
checkout. Unset `SURREALFS_SOURCE` once the branch is pushed and the plugin goes
back to the git URL.

For a one-off session with no install at all:

```bash
claude --plugin-dir ./surrealfs/integrations/claude
```

**Do not develop through `uvx`.** Pointed at a directory it caches the built
wheel and keeps serving it: neither `uv cache clean surrealfs` nor `--refresh`
nor `--reinstall` invalidates it, only `--no-cache`, which rebuilds on every
launch. Editing the source and seeing the old behaviour — with no error to say
so — costs an afternoon. For iteration, install the repo editable and run the
binary directly:

```bash
uv sync --extra claude
./.venv/bin/surrealfs-mcp --selftest
```

`claude plugin marketplace update surrealfs` picks up changes to `plugin.json`,
`.mcp.json` or the skill, since those are copied in at install time.

To remove it again:

```bash
claude plugin uninstall surrealfs@surrealfs
claude plugin marketplace remove surrealfs
```

## Install as a bare MCP server

Nothing to clone and nothing to keep up to date — `uvx` fetches the package on
launch. Add this to `claude_desktop_config.json` and restart Claude Desktop:

```json
{
  "mcpServers": {
    "surrealfs": {
      "command": "uvx",
      "args": ["--from", "surrealfs[claude]", "surrealfs-mcp"],
      "env": {
        "SURREALDB_URL": "wss://your-instance.surreal.cloud/rpc",
        "SURREALDB_USER": "your-user",
        "SURREALDB_PASS": "your-password",
        "SURREALFS_AGENT_USER": "claude-yourname",
        "SPECTRON_URL": "https://srv1.spectron.aws-usw2.surreal.cloud",
        "SPECTRON_CONTEXT_ID": "your-context",
        "SPECTRON_API_KEY": "your-key"
      }
    }
  }
}
```

The file lives at `~/Library/Application Support/Claude/claude_desktop_config.json`
on macOS and `%APPDATA%\Claude\claude_desktop_config.json` on Windows.

The `env` block is optional: the server reads `~/.config/surrealfs/env` however
it is launched, so the block is only for overriding it. What you cannot rely on is
your shell — Desktop launches the server with a bare environment, so a project
`.env` or an `export` in `.zshrc` reaches nothing. Diagnostics go to Desktop's MCP log
(`~/Library/Logs/Claude/mcp-server-surrealfs.log` on macOS), never to stdout,
which is the transport.

Installed this way the server brings no skill with it, so add the `/brain` skill
separately: Settings → Capabilities → Skills, and point it at
[`skills/brain/`](https://github.com/surrealdb/surrealfs/tree/main/surrealfs/integrations/claude/skills/brain).
The plugin does both in one step.

## Configuration

Every setting is an environment variable, read from `~/.config/surrealfs/env`
(see above) or from the environment, whichever has it — the environment wins:

| Variable | Default | Purpose |
|---|---|---|
| `SURREALDB_URL` | none — the server refuses without it | server to connect to |
| `SURREALDB_USER` / `SURREALDB_PASS` | `root` / `root` | credentials |
| `SURREALDB_NAMESPACE` / `SURREALDB_DATABASE` | `surrealfs` / `demo` | where the `file` table lives |
| `SURREALFS_AGENT_USER` | your unix username | the agent's home under `/home` |
| `SURREALFS_SEMANTIC` | unset | `1` makes `search` match on meaning too |
| `SURREALFS_MIRROR_ROOT` | `/` | only mirror writes under this path |
| `SPECTRON_URL` | `https://srv1.spectron.aws-usw2.surreal.cloud` | Spectron host |
| `SPECTRON_CONTEXT_ID` | unset | the context (tenant) to file into |
| `SPECTRON_API_KEY` | unset | bearer token |
| `SPECTRON_SCOPE` | `brain` | scope path to file under and lens queries to |
| `SURREALFS_ENV_FILE` | `$XDG_CONFIG_HOME/surrealfs/env` | where the above are read from |

Leave `SPECTRON_CONTEXT_ID` or `SPECTRON_API_KEY` unset and this is simply a
SurrealFS server: mirroring is a no-op and `brain_recall` says it is unconfigured.
Every filesystem tool still works.

`SURREALFS_SEMANTIC` needs `OPENAI_API_KEY` and the `embed` extra, and only pays
off once the vectors exist — see
[semantic search](https://github.com/surrealdb/surrealfs/blob/main/docs/semantic-search.md).

## The tools

The same fifteen as every other surface, generated from `surrealfs/tools/`, and
**unprefixed** — unlike the Hermes plugin's `surrealfs_*`. An MCP client namespaces
tools by server, so a prefix here would read as `surrealfs:surrealfs_ls`.

`brain_recall` is the sixteenth. It queries Spectron, not the filesystem: it
answers what the current files no longer say — a superseded version of a file,
entities and relationships extracted out of the prose, context filed by a session
you never saw. It takes a question, not a keyword, and each hit is labelled with
the SurrealFS path it was filed from so you can open the file next.

## Checking it works

```bash
surrealfs-mcp --selftest
```

It prints the config file it read, the server and database it reached, the
identity it acts as, the tool count, `ls /`, and whether Spectron answers —
then `OK` or a line naming the layer that failed. Exit code 0 only when
everything it checked worked.

It exists because every misconfiguration has the same symptom: the model says it
has no such tools. That one message covers a client that never launched the
server, a missing config file, an unreachable database, a wrong namespace, and a
Spectron key that does not work. The selftest separates them.

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
launched the server. Look for its log:

- **Claude Code** — `claude mcp list`, or `/mcp` in a session.
- **Claude Desktop** — `~/Library/Logs/Claude/mcp.log` and
  `~/Library/Logs/Claude/mcp-server-surrealfs.log`. An empty `mcp.log` and no
  per-server log means Desktop launched nothing.

Two Desktop-specific traps. It launches servers with a **minimal `PATH`** that
excludes `~/.local/bin`, so `"command": "uvx"` fails with nothing logged — give an
absolute path. And Desktop does **not** read a plugin's bundled `.mcp.json`; that
is a Claude Code feature. In Desktop the skill loads from the plugin but the
server has to go in `claude_desktop_config.json` as above.

## Mirroring on write

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

## Example

[`examples/company-brain/`](https://github.com/surrealdb/surrealfs/tree/main/examples/company-brain)
— a devsecops company brain: two Claude Desktop routines file Snyk, Drata,
SonarQube, Okta and Slack state into `/brain/acme/`, and `/brain plan my
next high-priority task` answers from the files plus Spectron. Runs with no
vendor accounts.

## See also

- [SurrealFS as a Hermes toolset](https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/hermes/README.md)
  — the same tools for Hermes, where they *are* prefixed and there is no MCP.
- [SurrealFS as Hermes memory](https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/hermes_memory/README.md)
  — automatic turn-by-turn filing, the closest thing to this one's mirror.
