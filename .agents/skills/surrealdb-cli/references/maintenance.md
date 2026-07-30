---
title: Maintenance & Operations
---

# Maintenance & Operations

Operational commands for versioning, upgrades, readiness checks, storage
repair, and SurrealML models. These commands are version-sensitive — confirm
flags with `surreal <command> --help` for your installed binary.

## version

```bash
surreal version
```

Prints the local CLI version. Combined with an endpoint it also reports the
remote server version:

```bash
surreal version --endpoint http://localhost:8000
```

## is-ready

Returns exit code `0` once the server accepts connections and non-zero
otherwise — ideal for CI gates, container healthchecks, and startup scripts.
The alias `surreal isready` also works.

```bash
surreal is-ready --endpoint http://localhost:8000

# Wait for the server in a script
until surreal is-ready -e http://localhost:8000 2>/dev/null; do
  sleep 1
done
echo "SurrealDB is up"
```

## upgrade

Replaces the running binary with another release.

| Flag | Effect |
| --- | --- |
| `--version <VERSION>` | Install a specific version |
| `--beta` / `--alpha` / `--nightly` | Install the latest pre-release of that channel |
| `--dry-run` | Show what would happen without replacing the binary |

```bash
surreal upgrade                      # latest stable
surreal upgrade --version 3.1.3      # pin a version
surreal upgrade --dry-run            # preview only
```

If SurrealDB was installed via a package manager (Homebrew) or runs in Docker,
upgrade through that tool instead of `surreal upgrade`.

## fix

After a major upgrade, an on-disk store may use an older storage format. `fix`
migrates it in place. Point it at the same `PATH` used by `surreal start`, and
run it while the server is stopped.

```bash
surreal fix rocksdb:mydata.db
```

## ml — SurrealML models

`surreal ml` imports and exports [SurrealML](https://surrealdb.com/docs/surrealml)
models. Both subcommands take the standard connection flags plus `--namespace`
and `--database`; export also requires `--name` and `--version`.

```bash
# Import a model file
surreal ml import --endpoint http://localhost:8000 \
  --user root --pass root --ns main --db main model.surml

# Export a specific model + version
surreal ml export --endpoint http://localhost:8000 \
  --user root --pass root --ns main --db main \
  --name my_model --version 0.1.0 model.surml
```

Models are scoped to a namespace and database, so target the same `--ns`/`--db`
on import and export.
