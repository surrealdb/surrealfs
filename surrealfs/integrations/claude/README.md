# SurrealFS for Claude

A plugin that bundles [the SurrealFS MCP server](../mcp/README.md) with the
`/brain` skill: the fifteen filesystem tools, `brain_recall` for the Spectron
memory behind them, and a skill that knows how to use both.

This page is the Claude-specific half — installing, and the two places Desktop
behaves differently. **What the server reads, what the tools are, and how to check
it works all live in [the MCP README](../mcp/README.md)**, because none of it is
specific to Claude.

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

Then write the configuration to `~/.config/surrealfs/env` — see
[Configuration](../mcp/README.md#configuration) for the file and every variable in
it. The plugin's `.mcp.json` deliberately passes no `env` block at all, so nothing
about SurrealDB needs to be in your environment.

### Installing from a local checkout

Before this is pushed, or while working on it, nothing needs editing — the
package source in [`.mcp.json`](.mcp.json) is
`${SURREALFS_SOURCE:-git+https://github.com/surrealdb/surrealfs.git}`, so one
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
uv sync --extra mcp
./.venv/bin/surrealfs-mcp --selftest
```

`claude plugin marketplace update surrealfs` picks up changes to `plugin.json`,
`.mcp.json` or the skill, since those are copied in at install time.

To remove it again:

```bash
claude plugin uninstall surrealfs@surrealfs
claude plugin marketplace remove surrealfs
```

## Claude Desktop

Two Desktop-specific traps.

**Desktop does not read a plugin's bundled `.mcp.json`** — that is a Claude Code
feature. In Desktop the skill loads from the plugin but the server has to go in
`claude_desktop_config.json` yourself, which needs permission to add a connector:

```json
{
  "mcpServers": {
    "surrealfs": {
      "command": "uvx",
      "args": ["--from", "surrealfs[mcp]", "surrealfs-mcp"]
    }
  }
}
```

The file lives at `~/Library/Application Support/Claude/claude_desktop_config.json`
on macOS and `%APPDATA%\Claude\claude_desktop_config.json` on Windows. No `env`
block is needed — the server reads `~/.config/surrealfs/env` however it is
launched. What you cannot rely on is your shell: Desktop launches the server with
a bare environment, so a project `.env` or an `export` in `.zshrc` reaches
nothing.

**Desktop launches servers with a minimal `PATH`** that excludes `~/.local/bin`,
so `"command": "uvx"` fails with nothing logged. Give an absolute path.

Installed this way the server brings no skill with it, so add the `/brain` skill
separately: Settings → Capabilities → Skills, and point it at
[`skills/brain/`](skills/brain). The plugin does both in one step.

## When a client shows no tools

Run [`surrealfs-mcp --selftest`](../mcp/README.md#checking-it-works) first — it
names the layer that failed. If it passes and the client still lists nothing, the
client never launched the server. Its log:

- **Claude Code** — `claude mcp list`, or `/mcp` in a session.
- **Claude Desktop** — `~/Library/Logs/Claude/mcp.log` and
  `~/Library/Logs/Claude/mcp-server-surrealfs.log`. An empty `mcp.log` and no
  per-server log means Desktop launched nothing.

## Example

[`examples/company-brain/`](https://github.com/surrealdb/surrealfs/tree/main/examples/company-brain)
— a devsecops company brain: two Claude Desktop routines file Snyk, Drata,
SonarQube, Okta and Slack state into `/brain/acme/`, and `/brain plan my
next high-priority task` answers from the files plus Spectron. Runs with no
vendor accounts.

## See also

- [SurrealFS over MCP](../mcp/README.md) — the server itself, its configuration,
  its tools, and every other client that can run it.
- [SurrealFS as a Hermes toolset](../hermes/README.md) — the same tools for
  Hermes, where they *are* prefixed and there is no MCP.
