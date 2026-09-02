"""Provision the record users that `record` auth signs in as.

    python -m surrealfs.users add alice
    python -m surrealfs.users list

Only needed if you opted into record auth (`apply_schema(record_auth=True)`).
Without it SurrealFS still enforces permissions -- in `SurrealFs`, against the
identity the process is configured with -- and no user rows exist at all.

This connects with the same ``SURREALDB_*`` variables as everything else, and
must do so as a *system* user: creating a `user` row is exactly what record
access forbids, deliberately, so that nobody can register a username and take
over the home it names.
"""

from __future__ import annotations

import argparse
import asyncio
import getpass
import os
import secrets
import sys

from .fs import ROOT, SurrealFs, raise_for_status
from .integrations._connect import _slug, connect
from .paths import home_of


async def add(db, name: str, password: str) -> str:
    """Create the user and their home directory. Returns the home path.

    The home is made by the ordinary `mkdir`, as root, because `default_owner`
    already hands a `/home/<name>` folder to the name it carries -- there is no
    separate ownership rule to keep in step here.

    :data:`~surrealfs.fs.ROOT` is refused. It is the bypass identity, not a
    name: `owner` DEFAULTs to it and `/home` is seeded to it, so a record user
    called `root` would own every row nobody claimed -- including `/home`, which
    `mode`'s field permission would then let them chmod to 0700, gating every
    other user out of their own home. The guard is here rather than in the CLI
    so that no caller can go around it; `record_auth.surql` refuses the signin
    as well.
    """
    if name == ROOT:
        raise ValueError(f"{ROOT!r} is the bypass identity, not a record user")
    raise_for_status(
        await db.query_raw(
            "CREATE type::record('user', $name) "
            "SET password = crypto::argon2::generate($password)",
            {"name": name, "password": password},
        )
    )
    home = home_of(name)
    await SurrealFs(db, user=ROOT).mkdir(home, parents=True)
    return home


async def listing(db) -> list[str]:
    results = raise_for_status(
        await db.query_raw("SELECT VALUE record::id(id) FROM user")
    )
    return sorted(results[-1] or [])


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="python -m surrealfs.users",
        description="Manage the SurrealFS record-auth users.",
    )
    sub = parser.add_subparsers(dest="command", required=True)
    creating = sub.add_parser("add", help="create a user and their home folder")
    creating.add_argument("name", help="username; also the record id and /home folder")
    creating.add_argument(
        "--password",
        help="omit to be prompted, or pass --generate (env: SURREALFS_NEW_PASSWORD)",
    )
    creating.add_argument(
        "--generate", action="store_true", help="generate a password and print it once"
    )
    sub.add_parser("list", help="list the provisioned usernames")
    return parser.parse_args(argv)


def _password(args: argparse.Namespace) -> tuple[str, bool]:
    """The password to set, and whether it must be shown to the operator."""
    if args.generate:
        return secrets.token_urlsafe(24), True
    if args.password:
        return args.password, False
    if from_env := os.environ.get("SURREALFS_NEW_PASSWORD"):
        return from_env, False
    # A tty prompt keeps it out of the shell history and the process list.
    return getpass.getpass(f"Password for {args.name}: "), False


async def _run(args: argparse.Namespace) -> int:
    db = await connect()
    try:
        if args.command == "list":
            names = await listing(db)
            print("\n".join(names) if names else "(no users provisioned)")
            return 0

        name = _slug(args.name)
        if name != args.name:
            print(f"Using {name!r} (a username is one path segment)", file=sys.stderr)
        password, show = _password(args)
        if not password:
            print("Refusing to set an empty password.", file=sys.stderr)
            return 1
        home = await add(db, name, password)
        print(f"Created user {name} with home {home} (0700)")
        if show:
            print(f"Password: {password}")
        print(
            "Sign in with SURREALDB_AUTH_LEVEL=record "
            f"SURREALDB_USER={name} SURREALDB_PASS=..."
        )
        return 0
    finally:
        await db.close()


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    try:
        return asyncio.run(_run(args))
    except Exception as exc:  # noqa: BLE001 - a CLI should not show a traceback
        print(f"Failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
