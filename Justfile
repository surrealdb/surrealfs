set dotenv-load := true

default:
    @just --list

cli:
    cargo run --quiet

agent:
    uv run --env-file .env python/surrealfs_ai/surrealfs_ai/__init__.py

build:
    cd python/surrealfs_py && uv run maturin develop --uv
