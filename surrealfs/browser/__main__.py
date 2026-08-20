"""Run the SurrealFS file browser.

    surrealfs-browser
    python -m surrealfs.browser --port 8080
    python -m surrealfs.browser --auth-level database --namespace team

Credentials come from a `.env` in the working directory (or any parent), or from
the environment directly, which always wins over the file:

    SURREALDB_URL=wss://….surreal.cloud/rpc
    SURREALDB_USER=…
    SURREALDB_PASS=…
    SURREALDB_NAMESPACE=…
    SURREALDB_DATABASE=…
    SURREALDB_AUTH_LEVEL=database      # root (default) | namespace | database
    OPENAI_API_KEY=…                   # optional: semantic search
    ANTHROPIC_API_KEY=…                # optional: the chat panel

The server binds loopback. `--host 0.0.0.0` exposes it to the network, where
anyone who can reach the port has the same access to the database as these
credentials -- the page has no login of its own.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys

from ..integrations._connect import AUTH_LEVELS


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="surrealfs-browser",
        description="Browse and edit a SurrealFS `file` table in a web UI.",
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=7933)
    # These default to None and are pushed back into the environment, because the
    # environment is what `_connect.connect()` reads. Spelling the defaults out
    # here too would give them two places to drift apart in.
    for flag, env, default in (
        ("--url", "SURREALDB_URL", "ws://localhost:8000/rpc"),
        ("--username", "SURREALDB_USER", "root"),
        ("--password", "SURREALDB_PASS", "root"),
        ("--namespace", "SURREALDB_NAMESPACE", "surrealfs"),
        ("--database", "SURREALDB_DATABASE", "demo"),
        ("--auth-level", "SURREALDB_AUTH_LEVEL", "root"),
    ):
        parser.add_argument(flag, help=f"(env: {env}, default: {default})")
    return parser.parse_args(argv)


def _apply_to_environment(args: argparse.Namespace) -> None:
    for name, env in (
        ("url", "SURREALDB_URL"),
        ("username", "SURREALDB_USER"),
        ("password", "SURREALDB_PASS"),
        ("namespace", "SURREALDB_NAMESPACE"),
        ("database", "SURREALDB_DATABASE"),
        ("auth_level", "SURREALDB_AUTH_LEVEL"),
    ):
        if (value := getattr(args, name)) is not None:
            os.environ[env] = value


def main(argv: list[str] | None = None) -> int:
    try:
        from dotenv import find_dotenv, load_dotenv

        from . import serve
    except ImportError as exc:
        print(f"{exc.name} is missing. Install the browser extra:", file=sys.stderr)
        print('    pip install "surrealfs[browser]"', file=sys.stderr)
        return 1

    # Before the flags, so an explicit flag still wins; `override=False` is the
    # default, so a variable already exported wins over the file.
    #
    # `usecwd=True` is not optional: plain `find_dotenv()` walks up from the
    # *calling module's* directory, which is inside site-packages -- or, when run
    # from a checkout, the surrealfs repo, whose own .env then silently overrides
    # the one the user is standing in.
    load_dotenv(find_dotenv(usecwd=True))
    args = _parse_args(argv)
    if args.auth_level is not None and args.auth_level not in AUTH_LEVELS:
        print(f"--auth-level must be one of {', '.join(AUTH_LEVELS)}", file=sys.stderr)
        return 2
    _apply_to_environment(args)

    try:
        asyncio.run(serve(host=args.host, port=args.port))
    except KeyboardInterrupt:
        return 0
    except Exception as exc:  # noqa: BLE001 - a CLI should not show a traceback
        print(f"Browser stopped: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
