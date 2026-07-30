---
title: Import & Export
---

# Import & Export

`surreal import` loads a SurrealQL script into a database; `surreal export`
dumps a database back out as SurrealQL. Together they cover backups, restores,
seeding, and moving data between instances.

Both share the standard connection flags (`-e/--endpoint`, `--ns`, `--db`,
`-u/--user`, `-p/--pass`, `-t/--token`, `--auth-level`) with `SURREAL_*` env
fallbacks. The endpoint defaults to `http://localhost:8000`, and `--namespace`
and `--database` are required.

## Import

```
surreal import [OPTIONS] --namespace <NS> --database <DB> <FILE>
```

`<FILE>` is a path to a `.surql` script and is required.

```bash
# Restore a dump with root credentials
surreal import --endpoint http://localhost:8000 \
  --user root --pass root --ns main --db main backup.surql

# With token auth
surreal import --token "$SURREAL_JWT" --ns main --db main seed.surql
```

## Export

```
surreal export [OPTIONS] --namespace <NS> --database <DB> [FILE]
```

`[FILE]` is optional and defaults to `-` (stdout), so the export can be
redirected or piped.

```bash
# Full database dump to a file
surreal export --endpoint http://localhost:8000 \
  --user root --pass root --ns main --db main backup.surql

# Stream to stdout (e.g. pipe straight into another instance)
surreal export --user root --pass root --ns main --db main | \
  surreal import --endpoint http://other:8000 \
    --user root --pass root --ns main --db main /dev/stdin
```

### Filtering what gets exported

By default everything is exported. Use these flags to scope the dump (most take
an optional `true`/`false`, e.g. `--records false`):

| Flag | Selects |
| --- | --- |
| `--only` | Export only the resources explicitly enabled by other flags |
| `--tables [names]` | All tables, or a comma-separated subset |
| `--tables-exclude <names>` | Tables to omit |
| `--records [bool]` | Row data |
| `--users [bool]` | System users |
| `--accesses [bool]` | Access methods |
| `--params [bool]` | Database params |
| `--functions [bool]` | Functions |
| `--analyzers [bool]` | Full-text analyzers |
| `--versions [bool]` | Versioned records |

(`--apis`, `--buckets`, `--modules`, and `--configs` are also available on
recent versions.)

```bash
# Schema-only backup: definitions but no row data
surreal export --user root --pass root --ns main --db main \
  --records false schema.surql

# Export just two tables
surreal export --user root --pass root --ns main --db main \
  --only --tables user,order subset.surql
```

## Backup / restore pattern

```bash
# Back up nightly
surreal export --user root --pass root --ns main --db main \
  "backup-$(date +%F).surql"

# Restore into a fresh database
surreal import --user root --pass root --ns main --db restored \
  backup-2026-06-15.surql
```

Keep credentials in `SURREAL_USER`/`SURREAL_PASS`/`SURREAL_TOKEN` for
unattended/cron use rather than passing them as flags.
