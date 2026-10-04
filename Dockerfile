# Multi-stage Dockerfile for SurrealFS (Official Multi-Arch Image)
# ==============================================================================

# Stage 1: Build the React SPA browser UI using Bun
FROM oven/bun:1.2-alpine AS ui-builder
WORKDIR /build/surrealfs/browser/ui
COPY surrealfs/browser/ui/package.json surrealfs/browser/ui/bun.lock ./
RUN bun install --frozen-lockfile
COPY surrealfs/browser/ui/ ./
RUN bun run build

# Stage 2: Production runtime image with Python 3.12 and FUSE 3
FROM python:3.12-slim-bookworm

LABEL org.opencontainers.image.title="SurrealFS" \
      org.opencontainers.image.description="An agent-native distributed filesystem on SurrealDB" \
      org.opencontainers.image.source="https://github.com/surrealdb/surrealfs"

# Install FUSE runtime and essential tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    fuse3 \
    libfuse3-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy uv binary for rapid dependency management
COPY --from=ghcr.io/astral-sh/uv:0.5.21 /uv /uvx /bin/

# Copy packaging metadata
COPY pyproject.toml README.md ./

# Copy built browser static assets from Stage 1
COPY --from=ui-builder /build/surrealfs/browser/static ./surrealfs/browser/static

# Copy SurrealFS library & integrations
COPY surrealfs ./surrealfs

# Install package with all extras
RUN uv pip install --system --no-cache ".[mcp,browser,embed,agent-memory,crdt]"

# Default runtime configuration
ENV PYTHONUNBUFFERED=1 \
    SURREALDB_URL=ws://127.0.0.1:8000/rpc \
    SURREALDB_NAMESPACE=surrealfs \
    SURREALDB_DATABASE=demo \
    SURREALDB_AUTH_LEVEL=record

# 7933 for Browser UI, 8000 default SurrealDB port
EXPOSE 7933 8000

ENTRYPOINT ["surrealfs"]
CMD ["status"]
