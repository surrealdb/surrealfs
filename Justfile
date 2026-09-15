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

# Embed new and changed files, then keep polling for more. `--once` for a single pass.
embed *ARGS:
    uv run --extra embed python -m surrealfs.embed {{ARGS}}

test:
    uv run pytest --quiet --no-summary

lint:
    uv run ruff check .
    surreal validate surrealfs/schema/*.surql

fmt:
    uv run ruff format .

# Run the note-taking chat agent on http://127.0.0.1:7932
agent:
    uv run --extra demo python examples/chat_agent.py

# Build the browser's React UI into surrealfs/browser/static. Needs bun.
ui:
    cd surrealfs/browser/ui && bun install && bun run build

# The vite dev server, proxying /api and /raw to `just browser` on :7933.
ui-dev:
    cd surrealfs/browser/ui && bun install && bun run dev

# Browse the filesystem in a web UI on http://127.0.0.1:7933
browser *ARGS: ui
    uv run --extra browser python -m surrealfs.browser {{ARGS}}

# Loopback only: `--dev-identity` turns SSO off, so it is refused on any other
# bind. Needs `schema --record-auth --sso` and a provisioned user first.
# Browse as USER with the authenticated browser, on http://127.0.0.1:7933
browser-sso USER: ui
    uv run --extra browser-sso python -m surrealfs.browser.sso \
        --host 127.0.0.1 --dev-identity {{USER}}

# Deploy that browser to Cloudflare. See deploy/cloudflare/README.md.
deploy: ui
    cd deploy/cloudflare && npx wrangler deploy

# Run the MCP server over stdio, as an MCP client launches it.
# `agent-memory` too: the repo .env this dotenv-loads carries a key, and a key without
# that extra is refused. Drop it to see the filesystem-only surface.
mcp:
    uv run --extra mcp --extra agent-memory surrealfs-mcp

# Run the framework-free Anthropic tool-use loop.
loop *ARGS:
    uv run --extra demo python examples/anthropic_loop.py {{ARGS}}

check: lint test
