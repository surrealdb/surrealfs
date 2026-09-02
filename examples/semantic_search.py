"""Wiring up semantic search with OpenAI embeddings.

SurrealFS never calls an embedding model itself -- you supply the function, so
you choose the provider and pay for exactly the calls you want. The schema's
HNSW index expects 1536 dimensions, which matches text-embedding-3-small.

    just db
    export OPENAI_API_KEY=...
    uv run python examples/semantic_search.py
"""

from __future__ import annotations

import asyncio
import os

from surrealdb import AsyncSurreal

from surrealfs import SurrealFs, apply_schema
from surrealfs.embed import INDEXER_VERSION, make_embedder
from surrealfs.integrations.pydantic_ai import build_fs_toolset  # noqa: F401
from surrealfs.tools import ToolContext


async def main() -> None:
    embed = make_embedder()
    db = AsyncSurreal(os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"))
    await db.signin({"username": "root", "password": "root"})
    await db.use("surrealfs", "demo")
    await apply_schema(db)
    fs = SurrealFs(db, user="demo")

    await fs.write_text("/notes/invoicing.md", "How to send an invoice and get paid.")
    await fs.write_text("/notes/lunch.md", "Good sandwich places near the office.")

    # Embed anything new or changed. Cheap to re-run: rows whose content has not
    # changed since they were embedded are skipped.
    count = await fs.reindex_embeddings(embed, version=INDEXER_VERSION)
    print(f"Embedded {count} file(s)")

    # Note the query shares no words with the file it should find.
    for hit in await fs.search_semantic(await embed("how do I get paid"), k=2):
        print(f"  {hit.path}  distance={hit.score:.4f}")

    # To offer this to an agent, pass the embedder in the context and turn the
    # tool on:  build_fs_toolset(semantic=True) with ToolContext(fs, embed=embed)
    ctx = ToolContext(fs=fs, embed=embed)
    print("semantic tool ready:", ctx.embed is not None)

    await db.close()


if __name__ == "__main__":
    asyncio.run(main())
