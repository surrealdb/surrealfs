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
    uv run pytest

lint:
    uv run ruff check .

fmt:
    uv run ruff format .

# Run the note-taking chat agent on http://127.0.0.1:7932
agent:
    uv run --extra demo python examples/chat_agent.py

# Run the framework-free Anthropic tool-use loop.
loop *ARGS:
    uv run --extra demo python examples/anthropic_loop.py {{ARGS}}

check: lint test
