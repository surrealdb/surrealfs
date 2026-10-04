"""The unified SurrealFS command-line interface.

Usage:
    surrealfs status
    surrealfs schema apply [--record-auth]
    surrealfs users [add|list|del]
    surrealfs lock [acquire|release|list]
    surrealfs history <path>
    surrealfs restore <path> <generation>
    surrealfs grep <pattern> [--path <prefix>] [--glob <glob>]
    surrealfs init
    surrealfs mcp
    surrealfs browser
    surrealfs embed
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
import time
from pathlib import Path

from . import __version__
from .fs import SurrealFs
from .integrations._connect import (
    RECORD_LEVEL,
    agent_user,
    connected,
)


def _format_size(num_bytes: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if abs(num_bytes) < 1024.0:
            return f"{num_bytes:3.1f} {unit}"
        num_bytes /= 1024.0
    return f"{num_bytes:.1f} TB"


async def _status_cmd(args: argparse.Namespace) -> int:
    url = os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc")
    ns = os.environ.get("SURREALDB_NAMESPACE", "surrealfs")
    db_name = os.environ.get("SURREALDB_DATABASE", "demo")
    auth_level = os.environ.get("SURREALDB_AUTH_LEVEL", RECORD_LEVEL).lower()

    t0 = time.monotonic()
    try:
        async with connected() as db:
            ping_ms = (time.monotonic() - t0) * 1000
            fs = SurrealFs(db, user=agent_user())

            # Count files and active locks
            files_res = await fs._query("RETURN count(SELECT id FROM file);")
            files_count = files_res if isinstance(files_res, int) else 0

            locks_query = (
                "RETURN count(SELECT id FROM file_lock WHERE expires_at > time::now());"
            )
            locks_res = await fs._query(locks_query)
            locks_count = locks_res if isinstance(locks_res, int) else 0

            print(f"SurrealFS v{__version__}")
            print("-" * 40)
            print(f"Server:       {url} ({ping_ms:.1f}ms latency)")
            print(f"Namespace:    {ns}")
            print(f"Database:     {db_name}")
            print(f"Auth Level:   {auth_level} (identity: {fs.user})")
            print(f"Total Files:  {files_count}")
            print(f"Active Leases:{locks_count}")
            return 0
    except Exception as err:
        print(f"Error connecting to SurrealDB ({url}): {err}", file=sys.stderr)
        return 1


async def _schema_apply_cmd(args: argparse.Namespace) -> int:
    from .schema import apply_schema

    async with connected() as db:
        print(f"Applying SurrealFS schema (record_auth={args.record_auth})...")
        await apply_schema(db, record_auth=args.record_auth)
        print("Schema applied successfully.")
        return 0


async def _lock_cmd(args: argparse.Namespace) -> int:
    action = args.lock_action
    async with connected() as db:
        fs = SurrealFs(db, user=agent_user())
        if action == "acquire":
            res = await fs.acquire_lock(
                args.path, ttl_seconds=args.ttl, reason=args.reason
            )
            print(f"Acquired lease on {args.path} for {args.ttl}s: {res}")
            return 0
        elif action == "release":
            await fs.release_lock(args.path)
            print(f"Released lease on {args.path}")
            return 0
        elif action == "list":
            rows = await fs._query(
                "SELECT path, holder, expires_at, reason FROM file_lock "
                "WHERE expires_at > time::now();"
            )
            if not rows or not isinstance(rows, list):
                print("No active leases.")
                return 0
            print(f"{'PATH':<30} {'HOLDER':<15} {'EXPIRES AT':<25} {'REASON'}")
            print("-" * 80)
            for r in rows:
                print(
                    f"{str(r.get('path', '')):<30} {str(r.get('holder', '')):<15} "
                    f"{str(r.get('expires_at', '')):<25} {str(r.get('reason', ''))}"
                )
            return 0
        else:
            print("Specify lock action: acquire, release, or list", file=sys.stderr)
            return 1


async def _history_cmd(args: argparse.Namespace) -> int:
    async with connected() as db:
        fs = SurrealFs(db, user=agent_user())
        history = await fs.history(args.path, limit=args.limit)
        if not history:
            print(f"No history found for {args.path}")
            return 0
        print(f"History for {args.path}:")
        print(f"{'GEN':<5} {'OP':<8} {'AUTHOR':<15} {'CREATED AT':<25} {'SIZE/REASON'}")
        print("-" * 75)
        for h in history:
            created = str(h.created_at or "")[:19]
            author = h.author or "unknown"
            op = getattr(h, "op", "write") or "write"
            reason = getattr(h, "reason", "") or ""
            print(f"{h.generation:<5} {op:<8} {author:<15} {created:<25} {reason}")
        return 0


async def _restore_cmd(args: argparse.Namespace) -> int:
    async with connected() as db:
        fs = SurrealFs(db, user=agent_user())
        entry = await fs.restore(args.path, args.generation)
        print(f"Restored {entry.path} to generation {entry.generation}")
        return 0


async def _grep_cmd(args: argparse.Namespace) -> int:
    async with connected() as db:
        fs = SurrealFs(db, user=agent_user())
        matches = await fs.grep(
            args.pattern,
            path_prefix=args.path,
            glob=args.glob,
            limit=args.limit,
            is_regex=args.regex,
            case_sensitive=not args.ignore_case,
        )
        if not matches:
            return 0
        for m in matches:
            print(f"{m.path}:{m.line_number}:{m.line}")
        return 0


def _init_cmd(args: argparse.Namespace) -> int:
    """Zero-provisioning onboarding: apply schema, create user, write config."""
    config_dir = Path.home() / ".config" / "surrealfs"
    config_file = config_dir / "env"

    url = args.url or os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc")
    username = args.username or os.environ.get("SURREALDB_USER", "root")
    password = args.password or os.environ.get("SURREALDB_PASS", "root")
    namespace = args.namespace or os.environ.get("SURREALDB_NAMESPACE", "surrealfs")
    database = args.database or os.environ.get("SURREALDB_DATABASE", "demo")
    account = args.account or "admin"
    account_pass = args.account_pass or "surrealfs-secret"

    print("Initializing SurrealFS...")
    print(f"Target: {url} (NS: {namespace}, DB: {database})")

    async def _do_init():
        from surrealdb import AsyncSurreal

        from .schema import apply_schema
        from .users import add

        async with AsyncSurreal(url) as db:
            await db.signin({"username": username, "password": password})
            await db.use(namespace, database)
            print("Applying schema with record auth enabled...")
            await apply_schema(db, record_auth=True)

            print(f"Creating record user '{account}'...")
            try:
                await add(db, account, account_pass)
                print(f"Created user '{account}'.")
            except Exception as e:
                print(f"User provisioning: {e}")

        # Write config file
        config_dir.mkdir(parents=True, exist_ok=True)
        env_content = (
            f"SURREALDB_URL={url}\n"
            f"SURREALDB_NAMESPACE={namespace}\n"
            f"SURREALDB_DATABASE={database}\n"
            f"SURREALDB_AUTH_LEVEL=record\n"
            f"SURREALDB_USER={account}\n"
            f"SURREALDB_PASS={account_pass}\n"
        )
        config_file.write_text(env_content, encoding="utf-8")
        # Restrict permissions
        os.chmod(config_file, 0o600)
        print(f"Wrote record-auth configuration to {config_file}")
        print("SurrealFS initialized successfully.")

    asyncio.run(_do_init())
    return 0


def _run_coroutine(coro):
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(coro)
    import concurrent.futures

    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
        return pool.submit(asyncio.run, coro).result()


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        prog="surrealfs",
        description="SurrealFS unified management CLI",
    )
    parser.add_argument(
        "-v", "--version", action="version", version=f"surrealfs {__version__}"
    )

    subparsers = parser.add_subparsers(dest="command", help="Available subcommands")

    # status
    subparsers.add_parser(
        "status", help="Inspect server status, latency, and auth level"
    )

    # schema
    schema_parser = subparsers.add_parser("schema", help="Manage SurrealFS schema")
    schema_sub = schema_parser.add_subparsers(dest="schema_action")
    schema_apply = schema_sub.add_parser("apply", help="Apply SurrealFS schema")
    schema_apply.add_argument(
        "--record-auth", action="store_true", help="Enable record-auth DDL"
    )

    # users
    users_parser = subparsers.add_parser(
        "users", help="Manage record-auth users (delegates to surrealfs.users)"
    )
    users_parser.add_argument("users_args", nargs=argparse.REMAINDER)

    # lock
    lock_parser = subparsers.add_parser("lock", help="Manage swarm advisory leases")
    lock_sub = lock_parser.add_subparsers(dest="lock_action")

    lock_acq = lock_sub.add_parser("acquire", help="Acquire a lease on a path")
    lock_acq.add_argument("path", help="Path to lock")
    lock_acq.add_argument(
        "--ttl", type=int, default=60, help="TTL in seconds (default: 60)"
    )
    lock_acq.add_argument("--reason", default="advisory lease", help="Reason for lease")

    lock_rel = lock_sub.add_parser("release", help="Release a lease on a path")
    lock_rel.add_argument("path", help="Path to unlock")

    lock_sub.add_parser("list", help="List active leases")

    # history
    hist_parser = subparsers.add_parser(
        "history", help="Show version history of a file"
    )
    hist_parser.add_argument("path", help="File path")
    hist_parser.add_argument(
        "--limit", type=int, default=20, help="Maximum versions to show"
    )

    # restore
    restore_parser = subparsers.add_parser(
        "restore", help="Restore a file to an earlier version"
    )
    restore_parser.add_argument("path", help="File path")
    restore_parser.add_argument("generation", type=int, help="Generation to restore to")

    # grep
    grep_parser = subparsers.add_parser(
        "grep", help="Search text files for lines matching pattern"
    )
    grep_parser.add_argument("pattern", help="Search pattern or regex")
    grep_parser.add_argument("--path", help="Optional path prefix filter")
    grep_parser.add_argument("--glob", help="Optional glob file filter (e.g. *.md)")
    grep_parser.add_argument("--limit", type=int, default=100, help="Maximum matches")
    grep_parser.add_argument(
        "-r", "--regex", action="store_true", help="Treat pattern as regex"
    )
    grep_parser.add_argument(
        "-i",
        "--ignore-case",
        action="store_true",
        help="Case-insensitive search",
    )

    # init
    init_parser = subparsers.add_parser(
        "init",
        help="Zero-provisioning onboarding: apply schema, create user, write config",
    )
    init_parser.add_argument("--url", help="SurrealDB URL")
    init_parser.add_argument("--username", help="System username (default: root)")
    init_parser.add_argument("--password", help="System password (default: root)")
    init_parser.add_argument("--namespace", help="Namespace (default: surrealfs)")
    init_parser.add_argument("--database", help="Database (default: demo)")
    init_parser.add_argument("--account", help="New record user name (default: admin)")
    init_parser.add_argument("--account-pass", help="New record user password")

    # mcp
    subparsers.add_parser("mcp", help="Run SurrealFS MCP server over stdio")

    # browser
    subparsers.add_parser("browser", help="Run SurrealFS browser web UI")

    # embed
    embed_parser = subparsers.add_parser("embed", help="Run embedding indexer daemon")
    embed_parser.add_argument(
        "--once", action="store_true", help="Run single pass and exit"
    )

    args = parser.parse_args(argv)

    if not args.command:
        parser.print_help()
        sys.exit(0)

    if args.command == "status":
        sys.exit(_run_coroutine(_status_cmd(args)))
    elif args.command == "schema":
        if args.schema_action == "apply":
            sys.exit(_run_coroutine(_schema_apply_cmd(args)))
        else:
            schema_parser.print_help()
            sys.exit(1)
    elif args.command == "users":
        from .users import main as users_main

        users_main(args.users_args)
        sys.exit(0)
    elif args.command == "lock":
        sys.exit(_run_coroutine(_lock_cmd(args)))
    elif args.command == "history":
        sys.exit(_run_coroutine(_history_cmd(args)))
    elif args.command == "restore":
        sys.exit(_run_coroutine(_restore_cmd(args)))
    elif args.command == "grep":
        sys.exit(_run_coroutine(_grep_cmd(args)))
    elif args.command == "init":
        sys.exit(_init_cmd(args))
    elif args.command == "mcp":
        from .integrations.mcp import main as mcp_main

        mcp_main()
    elif args.command == "browser":
        from .browser.__main__ import main as browser_main

        browser_main()
    elif args.command == "embed":
        from .embed import main as embed_main

        embed_args = ["--once"] if args.once else []
        embed_main(embed_args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
