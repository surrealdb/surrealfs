# The authenticated file browser, for a Cloudflare Container.
#
# Built from the repository root (`image_build_context: "../.."` in
# wrangler.jsonc), because it installs this package from source.
#
# It ships `surrealfs/browser/static/`, which is gitignored build output: run
# `just ui` before `wrangler deploy` or this image serves an API with no page in
# front of it. Same trap as `hatch build`; the app says so on startup rather
# than 404ing every asset.
FROM python:3.12-slim

COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv

WORKDIR /app

# Dependencies first, so editing the package does not re-resolve them.
COPY pyproject.toml uv.lock README.md ./
RUN uv sync --frozen --no-install-project --extra browser-sso

COPY surrealfs ./surrealfs
RUN uv sync --frozen --extra browser-sso

ENV PATH="/app/.venv/bin:$PATH" \
    PYTHONUNBUFFERED=1

# Fails loudly at build time rather than at 3am: the page is a separate build
# step that lives outside this Dockerfile, and forgetting it is easy.
RUN test -f surrealfs/browser/static/index.html || \
    (echo "surrealfs/browser/static is empty -- run \`just ui\` before deploying" && false)

EXPOSE 7933

# Binds 0.0.0.0 by default: the container is only addressable through its Worker.
CMD ["surrealfs-browser-sso"]
