# SurrealFS for Claude

A plugin that bundles [the SurrealFS MCP server](../mcp/README.md) with two
skills: the fifteen filesystem tools, `/brain` for working in the shared company
brain, and `/brain-memory` for the optional [agent
memory](../mcp/README.md#optional-agent-memory) behind it. Agent memory is a hosted
service with an API key; without one the plugin is complete and `/brain` is the
whole story.

This page is the Claude-specific half — installing, and where Desktop differs.
**What the server reads, what the tools are, and how to check it works all live
in [the MCP README](../mcp/README.md)**, because none of it is specific to
Claude.

## Install as a plugin

This is the one to use, in Claude Desktop and in Claude Code alike. A plugin
carries its own local MCP server, so it needs no admin permission where adding a
connector does — the only route in on a Team or Enterprise plan without owner
rights.

**Claude Desktop** (and claude.ai, and Cowork) — Customize in the left sidebar →
**Plugins** → **Add from a repository**, and give it
`https://github.com/surrealdb/surrealfs`. Then install `surrealfs` from it. The
repo root's [`.claude-plugin/marketplace.json`](../../../.claude-plugin/marketplace.json)
is what Claude reads there.

**Claude Code**:

```
/plugin marketplace add surrealdb/surrealfs
/plugin install surrealfs@surrealfs
```

Either way the plugin brings the MCP server and both skills together, so
there is nothing else to install and no client config to edit. In Claude Code the
tools arrive namespaced — `mcp__plugin_surrealfs_surrealfs__ls` and so on — and
the skills as `/surrealfs:brain` and `/surrealfs:brain-memory`. The latter needs
agent memory: without a key the server does not offer `brain_recall`, and the skill
says so and hands back to `/brain`.

Then write the configuration to `~/.config/surrealfs/env` — see
[Configuration](../mcp/README.md#configuration) for the file and every variable in
it. The plugin's `.mcp.json` deliberately passes no `env` block at all, so nothing
about SurrealDB needs to be in your environment.

### Installing from a local clone

Before this is pushed, or while working on it, nothing needs editing. A
marketplace can be a local directory, and `SURREALFS_SOURCE` redirects the
package the server is built from at your working tree:

```bash
export SURREALFS_SOURCE=/path/to/surrealfs   # in your shell profile

claude plugin marketplace add /path/to/surrealfs
claude plugin install surrealfs@surrealfs
```

`SURREALFS_SOURCE` is the one variable that *must* be exported rather than
filed: it decides what `uvx` builds, so it is read before the server exists to
load anything. It is also ours alone, so it cannot clash with another tool.
Unset it once the branch is pushed and the plugin goes back to the git URL.

Its default lives in [`scripts/surrealfs-mcp`](scripts/surrealfs-mcp), the
launcher that [`.mcp.json`](.mcp.json) names, and **not** in `.mcp.json` itself:
a plugin's MCP config expands plain `${VAR}` only, so a `${VAR:-default}`
written there reaches `uvx` as literal text and `uvx` exits with
`Failed to parse` — the server never starts, and the client says only that the
connection closed. A shell does support the default, so that is where the choice
is made. The launcher also finds `uvx` by absolute path, for the minimal `PATH`
Desktop launches servers with.

Local directory marketplaces are a Claude Code feature; Desktop takes a git URL.
To try a local clone there, push a branch and point **Add from a repository** at
it.

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
`.mcp.json`, the launcher or the skill, since those are copied in at install
time.

To remove it again:

```bash
claude plugin uninstall surrealfs@surrealfs
claude plugin marketplace remove surrealfs
```

## Claude Desktop

The plugin above is the Desktop route too, and it is the one to use: it brings
the server and the skill in one step, and needs no permission to add a
connector. Two things about Desktop are worth knowing anyway.

**Desktop launches servers with a bare environment**, so nothing your shell
exports reaches them: no project `.env`, no `export` in `.zshrc`. That is the
whole reason the server reads `~/.config/surrealfs/env` itself — see
[Configuration](../mcp/README.md#configuration). `SURREALFS_SOURCE` is the one
exception, and it is only for a local clone, which is a Claude Code affair.

**That bare environment has a minimal `PATH`** that excludes `~/.local/bin`, so
a bare `"command": "uvx"` fails with nothing written to any log. The plugin's
launcher searches for `uvx` by absolute path for exactly this; a hand-written
config has to give one itself.

Adding it as a connector by hand instead — for an older Desktop, or to run a
build the plugin does not point at — means editing
`claude_desktop_config.json`, which needs permission to add a connector:

```json
{
  "mcpServers": {
    "surrealfs": {
      "command": "/Users/you/.local/bin/uvx",
      "args": [
        "--from",
        "surrealfs[mcp,agent-memory] @ git+https://github.com/surrealdb/surrealfs.git",
        "surrealfs-mcp"
      ]
    }
  }
}
```

The file lives at `~/Library/Application Support/Claude/claude_desktop_config.json`
on macOS and `%APPDATA%\Claude\claude_desktop_config.json` on Windows. No `env`
block is needed — the server reads `~/.config/surrealfs/env` however it is
launched. Note the absolute `uvx`, and the explicit `git+…` source: `surrealfs`
is not on PyPI yet, so a bare `surrealfs[mcp]` cannot resolve. Keep
`agent-memory` in the extras if you file into agent memory: without it every
mirror raises, and only the tool result says so.

A connector added this way brings no skill with it, so add them separately:
Settings → Capabilities → Skills, pointed at [`skills/brain/`](skills/brain) and,
if you use agent memory, [`skills/brain-memory/`](skills/brain-memory).
The plugin does both at once.

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
next high-priority task` answers from the files, plus agent memory if it is
configured. Runs with no vendor accounts.

## See also

- [SurrealFS over MCP](../mcp/README.md) — the server itself, its configuration,
  its tools, and every other client that can run it.
- [SurrealFS as a Hermes toolset](../hermes/README.md) — the same tools for
  Hermes, where they *are* prefixed and there is no MCP.
