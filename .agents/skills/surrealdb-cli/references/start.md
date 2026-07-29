---
title: Running a Server
---

# Running a Server

`surreal start [OPTIONS] [PATH]` launches a SurrealDB server. The optional
`PATH` selects the storage backend; when omitted it defaults to `memory`.

```
surreal start [OPTIONS] [PATH]
```

## Storage backends

| `PATH` | Backend | Persistence |
| --- | --- | --- |
| `memory` (default) | In-memory | Ephemeral — lost on exit |
| `rocksdb:mydata.db` | RocksDB (embedded, single-node) | On disk |
| `surrealkv://mydata` | SurrealKV (embedded, supports versioning) | On disk |
| `tikv://127.0.0.1:2379` | TiKV (distributed cluster) | Distributed |
| `indxdb://mydata` | IndexedDB (browser/WASM) | Browser storage |

`memory` is ideal for tests and demos; `rocksdb`/`surrealkv` for local
persistence; `tikv` for horizontally scaled deployments.

## Key flags

| Flag | Default | Purpose |
| --- | --- | --- |
| `-u, --username` | — | Root username to create on first start (env `SURREAL_USER`) |
| `-p, --password` | — | Root password (env `SURREAL_PASS`) |
| `--unauthenticated` | off | Disable authentication (local/throwaway only) |
| `-b, --bind` | `127.0.0.1:8000` | Listen address `host:port` |
| `-l, --log` | `info` | `none`, `error`, `warn`, `info`, `debug`, `trace` |
| `--no-banner` | off | Suppress the startup banner |

`PATH` also reads from the `SURREAL_PATH` environment variable.

## Examples

```bash
# In-memory dev server with root credentials
surreal start --user root --pass root memory

# Persistent RocksDB store with debug logging
surreal start --log debug --user root --pass root rocksdb:mydata.db

# Persistent SurrealKV store
surreal start --user root --pass root surrealkv://mydata

# Expose on all interfaces, custom port (see caveat below)
surreal start --bind 0.0.0.0:8000 --user root --pass root rocksdb:mydata.db

# Unauthenticated throwaway instance — never expose this
surreal start --unauthenticated memory
```

### Docker

```bash
# In-memory
docker run --rm --pull always -p 8000:8000 surrealdb/surrealdb:latest \
  start --user root --pass root memory

# Persistent: mount a volume and store under /data
docker run --rm --pull always -p 8000:8000 -v surreal-data:/data \
  surrealdb/surrealdb:latest start --user root --pass root rocksdb:/data/mydata.db
```

Pin an explicit image tag (e.g. `surrealdb/surrealdb:v3.1.3`) in production
rather than `latest`.

## Caveats

- `--bind 0.0.0.0:...` exposes the server on every network interface. Only do
  this behind a firewall or trusted network, and never together with
  `--unauthenticated`.
- A `memory` server loses all data when the process stops — do not use it for
  anything you need to keep.
- After a major version upgrade, an on-disk store may need
  `surreal fix <PATH>` before it will start. See
  [maintenance.md](maintenance.md).
