"""Apply the SurrealFS schema from the command line.

    python -m surrealfs.schema
    python -m surrealfs.schema --url ws://localhost:8000/rpc --namespace notes
    python -m surrealfs.schema --print          # dump the DDL, connect to nothing

Connection details come from flags, falling back to the same ``SURREALDB_*``
environment variables the examples use. This is the one place in the package
that opens a connection; the library itself never does.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys

from . import apply_schema, schema_sql


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="python -m surrealfs.schema",
        description="Define the SurrealFS `file` table on a SurrealDB 3.x server.",
    )
    parser.add_argument(
        "--url",
        default=os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"),
        help="server URL (env: SURREALDB_URL)",
    )
    parser.add_argument(
        "--username",
        default=os.environ.get("SURREALDB_USER", "root"),
        help="(env: SURREALDB_USER)",
    )
    parser.add_argument(
        "--password",
        default=os.environ.get("SURREALDB_PASS", "root"),
        help="(env: SURREALDB_PASS)",
    )
    parser.add_argument(
        "--namespace",
        default=os.environ.get("SURREALDB_NAMESPACE", "surrealfs"),
        help="(env: SURREALDB_NAMESPACE)",
    )
    parser.add_argument(
        "--database",
        default=os.environ.get("SURREALDB_DATABASE", "demo"),
        help="(env: SURREALDB_DATABASE)",
    )
    parser.add_argument(
        "--print",
        action="store_true",
        dest="print_only",
        help="print the DDL instead of applying it",
    )
    return parser.parse_args(argv)


async def _apply(args: argparse.Namespace) -> None:
    from surrealdb import AsyncSurreal

    db = AsyncSurreal(args.url)
    try:
        await db.signin({"username": args.username, "password": args.password})
        await db.use(args.namespace, args.database)
        await apply_schema(db)
    finally:
        await db.close()


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)

    if args.print_only:
        print(schema_sql())
        return 0

    try:
        asyncio.run(_apply(args))
    except Exception as exc:  # noqa: BLE001 - a CLI should not show a traceback
        print(f"Failed to apply schema to {args.url}: {exc}", file=sys.stderr)
        return 1

    target = f"{args.namespace}/{args.database} on {args.url}"
    print(f"Applied schema to {target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
