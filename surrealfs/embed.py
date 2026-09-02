"""OpenAI embeddings, plus a daemon that keeps the `file` table's vectors current.

    python -m surrealfs.embed                   # poll forever, every 5s
    python -m surrealfs.embed --once            # one pass, then exit
    python -m surrealfs.embed --interval 30

Two prerequisites: the schema must already exist (`python -m surrealfs.schema`) --
the daemon only reads and writes rows, it never touches DDL -- and `OPENAI_API_KEY`
must be set. Connection details come from the same ``SURREALDB_*`` environment
variables the rest of the project uses.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
from collections.abc import Awaitable, Callable

from .fs import ROOT, SurrealFs

EMBED_MODEL = "text-embedding-3-small"  # 1536 dimensions, matching the HNSW index
INDEXER_VERSION = f"openai:{EMBED_MODEL}"


def make_embedder() -> Callable[[str], Awaitable[list[float]]]:
    """Build the embedding function.

    The import and the client both happen here rather than at module level: the
    package's runtime dependencies do not include `openai` (it is the `embed`
    extra), and constructing the client eagerly would require an API key just to
    import this module.
    """
    from openai import AsyncOpenAI

    client = AsyncOpenAI()

    async def embed(text: str) -> list[float]:
        response = await client.embeddings.create(model=EMBED_MODEL, input=text)
        return response.data[0].embedding

    return embed


async def index_forever(
    fs: SurrealFs,
    embed: Callable[[str], Awaitable[list[float]]],
    *,
    interval: float = 5.0,
    batch: int = 32,
    once: bool = False,
) -> None:
    """Re-embed new and changed files, then poll for more.

    `reindex_embeddings` is incremental and idempotent, so an idle pass costs one
    SELECT that returns nothing.
    """
    while True:
        try:
            count = await fs.reindex_embeddings(
                embed, version=INDEXER_VERSION, batch=batch
            )
        except Exception as exc:  # noqa: BLE001 - a daemon outlives transient failures
            if once:
                raise  # but a single pass must report failure in its exit code
            print(f"reindex failed: {exc}", file=sys.stderr, flush=True)
        else:
            # Silent when there is nothing to do, so the log is only ever news.
            if count:
                print(f"embedded {count} file(s)", flush=True)
        if once:
            return
        await asyncio.sleep(interval)


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="python -m surrealfs.embed",
        description="Keep the SurrealFS `file` table's embeddings up to date.",
    )
    parser.add_argument(
        "--interval", type=float, default=5.0, help="seconds between passes"
    )
    parser.add_argument(
        "--batch", type=int, default=32, help="rows to embed per query round trip"
    )
    parser.add_argument(
        "--once", action="store_true", help="make a single pass instead of polling"
    )
    return parser.parse_args(argv)


async def _run(args: argparse.Namespace) -> None:
    from surrealdb import AsyncSurreal

    embed = make_embedder()
    db = AsyncSurreal(os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"))
    try:
        await db.signin(
            {
                "username": os.environ.get("SURREALDB_USER", "root"),
                "password": os.environ.get("SURREALDB_PASS", "root"),
            }
        )
        await db.use(
            os.environ.get("SURREALDB_NAMESPACE", "surrealfs"),
            os.environ.get("SURREALDB_DATABASE", "demo"),
        )
        await index_forever(
            # Root: the indexer has to read every file in order to embed it,
            # including the ones inside private homes.
            SurrealFs(db, user=ROOT),
            embed,
            interval=args.interval,
            batch=args.batch,
            once=args.once,
        )
    finally:
        await db.close()


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    try:
        asyncio.run(_run(args))
    except KeyboardInterrupt:
        return 0
    except Exception as exc:  # noqa: BLE001 - a CLI should not show a traceback
        print(f"Indexer stopped: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
