set dotenv-load := true

default:
    @just --list

# Start a local SurrealDB 3.x server (in-memory).
db:
    surreal start --allow-all -u root -p root --bind 127.0.0.1:8000 memory

# Start a local SurrealDB 3.x server with data persisted to ./demo-db.
db-persist:
    surreal start --allow-all -u root -p root --bind 127.0.0.1:8000 rocksdb:demo-db

# Apply the file table schema to the local server.
schema *ARGS:
    uv run python -m surrealfs.schema {{ARGS}}

test:
    uv run pytest --quiet --no-summary

lint:
    uv run ruff check .
    surreal validate surrealfs/schema/*.surql

fmt:
    uv run ruff format .

# Build the browser's React UI into surrealfs/browser/static. Needs bun.
ui:
    cd surrealfs/browser/ui && bun install && bun run build

# The vite dev server, proxying /api and /raw to `just server` on :7933.
ui-dev:
    cd surrealfs/browser/ui && bun install && bun run dev

# Run the unified Rust SurrealFS Server (REST API + WebDAV + S3 + MCP + Chat + Embeddings + Web UI) on http://127.0.0.1:7933
server *ARGS: ui
    cargo run -p surrealfs-server -- --bind 127.0.0.1:7933 {{ARGS}}

# Alias for `just server`
browser *ARGS: (server ARGS)

# Verify Cloudflare Worker Wasm compilation (`wasm32-unknown-unknown`)
worker-check:
    cargo check -p surrealfs-core -p surrealfs-server --target wasm32-unknown-unknown

# Build UI + deploy SurrealFS Server Wasm Worker to Cloudflare Workers
deploy *ARGS: ui
    npx wrangler deploy {{ARGS}}

# Run the MCP server over stdio, as an MCP client launches it.
mcp:
    uv run --extra mcp --extra agent-memory surrealfs-mcp

# Run the framework-free Anthropic tool-use loop.
loop *ARGS:
    uv run --extra demo python examples/anthropic_loop.py {{ARGS}}

check: lint test
