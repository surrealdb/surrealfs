---
name: surrealdb-cli
description: "Use the `surreal` command-line binary to run a SurrealDB server (in-memory, RocksDB, SurrealKV, or TiKV), open an interactive or piped SQL REPL, import and export databases, check server readiness, upgrade the binary, repair storage, and manage SurrealML models. Use this skill whenever users run, connect to, back up, restore, or operate SurrealDB from the terminal. Triggers: surreal start, surreal sql, surreal import, surreal export, surreal isready, surreal upgrade, surreal fix, surreal ml, SurrealDB CLI, surreal server."
metadata:
  author: surrealdb
  version: "0.1.0"
---

# SurrealDB CLI

A skill for driving the [`surreal`](https://surrealdb.com/docs/surrealdb/cli) command-line binary — SurrealDB's all-in-one tool for running a server, querying it, and moving data in and out.

This skill covers operating the `surreal` binary itself. To write the SurrealQL that you run with it, use the **surrealql** skill; for declarative schema migrations and type generation, use the **surrealkit** skill.

## When to use this skill

Reference these guidelines when:

- Starting a SurrealDB server, locally or in a container (`surreal start`)
- Opening the SQL shell or piping queries from a file (`surreal sql`)
- Importing or exporting a database for backups, seeding, or migration (`surreal import` / `surreal export`)
- Gating scripts or CI on server availability (`surreal is-ready`)
- Upgrading the binary or repairing on-disk storage after an upgrade (`surreal upgrade` / `surreal fix`)
- Importing or exporting SurrealML models (`surreal ml`)

## Installation

| Method | Command |
| --- | --- |
| curl (macOS / Linux) | `curl -sSf https://install.surrealdb.com \| sh` |
| Windows (PowerShell) | `iwr https://windows.surrealdb.com -useb \| iex` |
| Homebrew | `brew install surrealdb/tap/surreal` |
| Docker | `docker run --rm --pull always -p 8000:8000 surrealdb/surrealdb:latest start` |

Verify with `surreal version`. Pin a specific image tag (not `latest`) for production. See the [install docs](https://surrealdb.com/install) for the current set of methods.

## Command map

| Command | Purpose | Reference |
| --- | --- | --- |
| `surreal start [PATH]` | Run the database server with a chosen storage backend | [start.md](references/start.md) |
| `surreal sql` | Interactive REPL or piped, non-interactive querying | [sql.md](references/sql.md) |
| `surreal import <FILE>` | Load a SurrealQL script into a database | [import-export.md](references/import-export.md) |
| `surreal export [FILE]` | Dump a database to SurrealQL (file or stdout) | [import-export.md](references/import-export.md) |
| `surreal is-ready` | Exit 0 when the server is accepting connections (alias `isready`) | [maintenance.md](references/maintenance.md) |
| `surreal version` | Print local tool and remote server versions | [maintenance.md](references/maintenance.md) |
| `surreal upgrade` | Replace the binary with a newer release | [maintenance.md](references/maintenance.md) |
| `surreal fix [PATH]` | Migrate on-disk storage to the current format | [maintenance.md](references/maintenance.md) |
| `surreal ml <sub>` | Import / export SurrealML models | [maintenance.md](references/maintenance.md) |

`surreal validate` (SurrealQL file checking) and formatting are covered by the **surrealql** skill. Newer subcommands (`mcp`, `module`) also exist — run `surreal help` and `surreal <command> --help` to confirm the surface of an installed version.

## Connection flags & env vars

`sql`, `import`, `export`, and `ml` share the same connection flags. Each flag has a `SURREAL_*` environment variable fallback; an explicit flag wins.

| Flag (aliases) | Env var | Notes |
| --- | --- | --- |
| `-e, --endpoint` | — | `ws://localhost:8000` for `sql`; `http://localhost:8000` for `import`/`export` |
| `--namespace` (`--ns`) | `SURREAL_NAMESPACE` | Required for `import`/`export`/`ml` |
| `--database` (`--db`) | `SURREAL_DATABASE` | Required for `import`/`export`/`ml` |
| `-u, --username` (`--user`) | `SURREAL_USER` | |
| `-p, --password` (`--pass`) | `SURREAL_PASS` | |
| `-t, --token` | `SURREAL_TOKEN` | JWT; use instead of username/password |
| `--auth-level` | `SURREAL_AUTH_LEVEL` | `root` (default), `namespace`/`ns`, or `database`/`db` |

Pass secrets via env vars rather than flags so they do not leak into shell history or process listings.

## Quick start

```bash
# 1. Start an authenticated in-memory server (foreground)
surreal start --user root --pass root memory

# 2. In another shell, open the SQL REPL on namespace/database "main"
surreal sql --endpoint ws://localhost:8000 \
  --user root --pass root --ns main --db main --pretty

# 3. Load a dump into that database
surreal import --endpoint http://localhost:8000 \
  --user root --pass root --ns main --db main backup.surql
```

## Rules & conventions

- **Authentication is on by default.** `surreal start` needs `--user`/`--pass`; only use `--unauthenticated` for throwaway local instances.
- **Choose a storage backend deliberately.** `memory` (the default) is ephemeral — data vanishes on exit. Use `rocksdb:`/`surrealkv://` for persistence and `tikv://` for distributed clusters. See [start.md](references/start.md).
- **Export defaults to stdout.** `surreal export` writes to `-` (stdout) unless you give a file path; redirect or pipe accordingly.
- **Gate CI with `is-ready`.** It returns a non-zero exit code until the server accepts connections — ideal in healthchecks and scripts.
- **Run `fix` after a major upgrade** if the server reports an incompatible storage version, pointing it at the same `PATH` used by `start`.
- **The CLI evolves.** Confirm flags and subcommands against `surreal <command> --help` for the installed version; this skill reflects v3.x.

## References

- Running a server and storage backends — [references/start.md](references/start.md)
- Interactive and piped querying — [references/sql.md](references/sql.md)
- Importing and exporting data — [references/import-export.md](references/import-export.md)
- Versioning, upgrades, readiness, repair, and ML models — [references/maintenance.md](references/maintenance.md)
