set dotenv-load := true

default:
    @just --list

run:
    cargo run --quiet
