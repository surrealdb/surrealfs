"""``hermes surrealfs-memory status`` — the check :meth:`is_available` cannot make.

The ABC forbids network calls in ``is_available``, so a provider that reports itself
available can still fail every turn against an unreachable server — and Hermes swallows
those failures into a ``logger.warning``. This is where a user finds out instead: it
connects, names the database it resolved, and counts what is filed there.

Hermes imports this module during argparse setup — before the provider itself loads, and
inside a bare ``except Exception`` — so nothing here may import ``surrealfs`` at module
level. A missing package would silently delete the subcommand in the exact state it
exists to diagnose.
"""

from __future__ import annotations

import asyncio
import os

__all__ = ["register_cli"]


def register_cli(subparser) -> None:
    """Build the ``hermes surrealfs-memory`` tree. Called during argparse setup.

    Sets ``func`` here rather than leaving it to Hermes, which looks the handler up as
    ``getattr(cli_mod, f"{provider_name}_command")`` — and ``surrealfs-memory_command``
    is not a valid identifier, so that lookup can only ever return ``None`` for us.
    """
    subs = subparser.add_subparsers(dest="surrealfs_memory_command")
    subs.add_parser("status", help="Connect, and report the database and filed turns")

    def dispatch(args) -> int:
        if getattr(args, "surrealfs_memory_command", None) == "status":
            return _status()
        subparser.print_help()
        return 0

    subparser.set_defaults(func=dispatch)


def _status() -> int:
    try:
        from surrealfs import SurrealFs
        from surrealfs.integrations._connect import connected
        from surrealfs.integrations.hermes_memory import SurrealFsMemory
    except ImportError as exc:  # The state `pip_dependencies` exists to prevent.
        print(f"\n  surrealfs is not importable by Hermes' interpreter: {exc}")
        print("  Run: hermes memory setup surrealfs-memory\n")
        return 1

    provider = SurrealFsMemory()
    provider.initialize("", hermes_home=_hermes_home())
    root = provider.root()

    print("\nsurrealfs-memory\n" + "─" * 40)
    # The provider's own schema already pairs each setting with its env var and default,
    # so read the resolved values off that rather than repeating them here.
    for field in provider.get_config_schema():
        if field.get("secret"):
            continue
        value = os.environ.get(field["env_var"]) or field.get("default", "")
        print(f"  {field['key'] + ':':12} {value}")
    print(f"  {'turns in:':12} {root}")

    async def count() -> int:
        async with connected() as db:
            return len(await SurrealFs(db).glob(f"{root}/**/*.md"))

    try:
        filed = asyncio.run(count())
    except Exception as exc:
        print(f"\n  Unreachable: {exc}\n")
        return 1
    print(f"  {'filed:':12} {filed} turns\n")
    return 0


def _hermes_home() -> str:
    """Hermes' home, which is what picks the profile subtree turns are filed under.

    Absent when this is run outside Hermes, which :func:`_profile` reads as the default
    profile — the same thing it does for an older Hermes that passes no ``hermes_home``.
    """
    try:
        from hermes_constants import get_hermes_home

        return str(get_hermes_home())
    except Exception:
        return ""
