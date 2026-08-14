# Keeping the vectors current under Hermes

Only needed with `SURREALFS_SEMANTIC=1`, and needed by **both** Hermes surfaces —
the [tool plugin](../surrealfs/integrations/hermes/README.md) and the [memory
provider](../surrealfs/integrations/hermes_memory/README.md). Nothing embeds a
file as it is written, so semantic search and semantic recall see only what the
indexer has already reached.

Hermes has no way for a plugin to declare a daemon of its own — the plugin API
registers tools, hooks, skills, commands and providers, and
`terminal(background=true)` processes are session-scoped — but its scheduler runs
scripts without an agent, which covers this without a systemd unit or a terminal
you have to remember to leave open.

The indexer needs the `embed` extra, which neither plugin's install pulls in:

```bash
uv pip install --python ~/.hermes/hermes-agent/venv/bin/python \
  "surrealfs[embed] @ git+https://github.com/surrealdb/surrealfs.git"

mkdir -p "${HERMES_HOME:-$HOME/.hermes}/scripts"
cat > "${HERMES_HOME:-$HOME/.hermes}/scripts/surrealfs-embed.sh" <<'EOF'
#!/usr/bin/env bash
# Cron sanitizes the subprocess env and OPENAI_API_KEY is on the blocklist, so the
# key has to come from a file — as do the SURREALDB_* connection details. Add both
# to $HERMES_HOME/.env if they are not there already.
set -a; . "${HERMES_HOME:-$HOME/.hermes}/.env"; set +a
exec ~/.hermes/hermes-agent/venv/bin/python -m surrealfs.embed --once
EOF

hermes cron create 'every 5m' --no-agent --name surrealfs-embed \
  --script surrealfs-embed.sh
```

`--no-agent` skips the inference layer entirely — no tokens, no model, just the
script — so a `--once` pass per tick replaces the polling loop `just embed` runs.
The script must live under `$HERMES_HOME/scripts/`; paths outside it are rejected.

Running SurrealFS from a checkout instead? Take the extra from your working copy
rather than the venv, and source that checkout's `.env`:

```bash
set -a; . "$HOME/repos/surrealfs/.env"; set +a
exec uv run --project "$HOME/repos/surrealfs" --extra embed \
  python -m surrealfs.embed --once
```

An idle pass prints nothing, and empty stdout is a silent tick, so you hear from
this only when it has news: `embedded 4 file(s)` gets delivered, and a pass that
fails (no API key, database down) exits non-zero, which Hermes reports as an alert
rather than swallowing. `hermes cron runs` has the history.

Two things to know before relying on it. The scheduler ticks inside the gateway
daemon, so this runs only while that does — `hermes gateway install` if it is not
already a service — and vectors go stale, without breaking anything, whenever it
is down: search and recall keep working, on their full-text arm. And a five-minute
period means a file written now is matchable *by meaning* in up to five minutes;
it is matchable by term immediately. Lower the interval if that gap matters, at
one `SELECT` per tick that returns nothing when there is no work.
