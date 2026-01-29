set dotenv-load := true

default:
    @just --list

run:
    cargo run --quiet

build:
    cd python/surrealfs_py && uv run maturin develop --uv
