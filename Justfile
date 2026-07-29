set dotenv-load := true

default:
    @just --list

cli:
    cargo run --quiet

agent:
    uv run --env-file .env python/surrealfs_ai/surrealfs_ai/__init__.py

build:
    cd python/surrealfs_py && uv run maturin develop --uv

stub-gen:
    MACOSX_DEPLOYMENT_TARGET=26.2 cargo run --bin stub_gen
    # cargo run --bin stub_gen
