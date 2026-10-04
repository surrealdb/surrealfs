# Docker & Cloud Infrastructure (§18)

SurrealFS provides official multi-arch Linux container images (`amd64` and `arm64`) with full support for:
- The Python async SDK and unified `surrealfs` CLI
- The React Browser Studio UI
- The Model Context Protocol (MCP) server
- The embedding indexer daemon
- FUSE 3 userspace filesystem mounting

---

## Quickstart with Docker Compose

To spin up a full SurrealFS cluster including SurrealDB 3.x, schema initialization, record-auth user provisioning, and the Browser UI:

```bash
docker compose up -d
```

### Services Started:

1. **`surrealdb`**: SurrealDB 3.x database server on port `8000`.
2. **`surrealfs-init`**: One-shot setup job that connects using root credentials, executes `surrealfs schema apply --record-auth`, and provisions record user accounts (`agent-1`, `indexer`).
3. **`surrealfs-browser`**: SurrealFS Browser Studio on `http://localhost:7933` signed in with record auth.
4. **`surrealfs-embed`**: Continuous embedding daemon automatically indexing updated documents.
5. **`surrealfs-mcp`**: Agent-native MCP server.

---

## Container Image Usage

### Pulling or Building

```bash
docker build -t surrealfs:latest .
```

### Inspecting Status

```bash
docker run --rm \
  -e SURREALDB_URL=ws://host.docker.internal:8000/rpc \
  -e SURREALDB_USER=root \
  -e SURREALDB_PASS=root \
  -e SURREALDB_AUTH_LEVEL=root \
  surrealfs:latest status
```

### Running the Browser UI

```bash
docker run -d --name surrealfs-browser \
  -p 7933:7933 \
  -e SURREALDB_URL=ws://surrealdb:8000/rpc \
  -e SURREALDB_USER=agent-1 \
  -e SURREALDB_PASS=agent-secret \
  -e SURREALDB_AUTH_LEVEL=record \
  surrealfs:latest browser --host 0.0.0.0 --port 7933
```

---

## Docker Compose Volume Sidecar Pattern

In Kubernetes or Docker environments where agents run as native Linux processes expecting standard POSIX file paths, SurrealFS can be mounted via FUSE as a shared volume sidecar:

```yaml
services:
  surrealfs-mount:
    image: surrealfs:latest
    cap_add:
      - SYS_ADMIN
    devices:
      - /dev/fuse:/dev/fuse
    security_opt:
      - apparmor:unconfined
    volumes:
      - shared-brain:/mnt/brain:rshared
    environment:
      - SURREALDB_URL=ws://surrealdb:8000/rpc
      - SURREALDB_USER=agent-worker
      - SURREALDB_PASS=worker-secret
      - SURREALDB_AUTH_LEVEL=record
    command: ["mount", "/mnt/brain"]

  agent-worker:
    image: python:3.12-slim
    depends_on:
      - surrealfs-mount
    volumes:
      - shared-brain:/mnt/brain:rslave
    command: ["python", "-c", "import time, os; print(os.listdir('/mnt/brain'))"]

volumes:
  shared-brain:
```
