---
title: Querying with surreal sql
---

# Querying with `surreal sql`

`surreal sql` opens a SurrealQL REPL against a running server, and also accepts
piped input for non-interactive use in scripts and CI.

For the SurrealQL statements themselves (SELECT, CREATE, RELATE, DEFINE, …) use
the **surrealql** skill — this page covers driving the shell.

## Connection flags

| Flag (aliases) | Env var | Default |
| --- | --- | --- |
| `-e, --endpoint` | — | `ws://localhost:8000` |
| `--namespace` (`--ns`) | `SURREAL_NAMESPACE` | — |
| `--database` (`--db`) | `SURREAL_DATABASE` | — |
| `-u, --username` (`--user`) | `SURREAL_USER` | — |
| `-p, --password` (`--pass`) | `SURREAL_PASS` | — |
| `-t, --token` | `SURREAL_TOKEN` | — |
| `--auth-level` | `SURREAL_AUTH_LEVEL` | `root` |

## Output & input flags

| Flag | Effect |
| --- | --- |
| `--pretty` | Pretty-print responses |
| `--json` | Emit results as JSON |
| `--multi` | Allow multi-line statements (newline does not submit; use `;`) |
| `--hide-welcome` | Suppress the welcome banner |

## Interactive REPL

```bash
surreal sql --endpoint ws://localhost:8000 \
  --username root --password root \
  --namespace main --database main --pretty
```

Inside the REPL you can switch context without reconnecting:

```surql
USE NS app DB production;
SELECT * FROM user LIMIT 5;
```

## Token authentication

`--token` replaces `--username`/`--password`:

```bash
surreal sql --endpoint http://localhost:8000 \
  --namespace main --database main --token "$SURREAL_JWT"
```

## Non-interactive (piped) use

Pipe a `.surql` file or a here-string in for scripting. The REPL reads from
stdin and exits when input ends:

```bash
# Run a file of statements
cat queries.surql | surreal sql \
  --endpoint http://localhost:8000 \
  --user root --pass root --ns main --db main

# One-off query, JSON output for downstream tooling
echo 'SELECT count() FROM user GROUP ALL;' | surreal sql \
  --user root --pass root --ns main --db main --json --hide-welcome
```

For bulk data loads prefer `surreal import` over piping into `sql`; see
[import-export.md](import-export.md).

## Tips

- Put credentials in `SURREAL_USER` / `SURREAL_PASS` (or `SURREAL_TOKEN`) so
  they stay out of shell history and `ps` output.
- `--endpoint` accepts `ws://`/`wss://` and `http://`/`https://`. Use the `wss`/
  `https` variants for TLS-terminated remote servers.
