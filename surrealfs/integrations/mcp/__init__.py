"""SurrealFS as an MCP server, for any client that speaks MCP.

The surface is the same fifteen filesystem tools as every other integration,
generated from `surrealfs.tools`. It serves over stdio as `surrealfs-mcp`, so
Claude Code, Claude Desktop, Cursor, Zed, Codex or a hand-rolled client all get
the same thing.

Nothing here is specific to one client. The Claude *plugin* that wraps this
server -- its manifest, its bundled `.mcp.json` and the skills -- lives next door
in `surrealfs/integrations/claude/`.

Configuration is read from ``~/.config/surrealfs/env``, so none of the
``SURREALDB_*`` names, which every other SurrealDB tool on the machine also
reads, has to be exported for this.

Spectron -- the hosted memory layer in `spectron.py` -- is optional, and nothing
above needs it. Configure it and two things appear: a sixteenth tool,
`brain_recall`, and a mirror of every text file written through this server. The
mirror lives in the dispatch path rather than in a tool the model is asked to call
afterwards: a memory layer that depends on remembering to update it is not one.
Leave it unconfigured and `brain_recall` is not advertised at all, because a tool
whose only answer is "not configured" is worse than no tool.

See `README.md` beside this file, and `examples/company-brain/` for the demo.
"""

from __future__ import annotations

import asyncio
import os
import sys
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any

from ...errors import SurrealFsError
from ...paths import normalize
from ...tools import ToolContext
from ..json_tools import call_tool, tool_definitions
from . import spectron

__all__ = [
    "RECALL_TOOL",
    "config_path",
    "main",
    "run_tool",
    "selftest",
    "serve",
    "tool_specs",
]

SERVER_NAME = "surrealfs"
RECALL_TOOL = "brain_recall"

# Where the server reads its own configuration, so nothing has to be exported
# globally. A dedicated file rather than a project `.env`: this process is
# launched by the MCP client, from a working directory that is not yours, and
# the `SURREALDB_*` names are shared with every other SurrealDB tool on the
# machine -- exporting them for this server would reach all of them.
CONFIG_DIR = "surrealfs"
CONFIG_NAME = "env"

# The tools that leave new text behind for Spectron to read. `write_bytes` is
# absent on purpose: Spectron indexes prose, and a base64 PNG is not prose.
MIRRORED = frozenset({"write_file", "edit", "touch"})

RECALL_DESCRIPTION = """\
Recall from Spectron, the memory layer behind SurrealFS. Use it *after* reading
the relevant files, not instead: it answers what the filesystem no longer says --
superseded versions of a file, entities and relationships extracted out of the
prose, context filed by someone else's session. Ask a question, not a keyword."""

RECALL_SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {
        "query": {
            "type": "string",
            "description": "What you want to know, phrased as a question",
            "minLength": 1,
        },
        "k": {
            "type": "integer",
            "description": "How many results to return",
            "default": spectron.RECALL_K,
        },
    },
    "required": ["query"],
}

Mirror = Callable[[str, str], Awaitable[None]]


def tool_specs(
    *, semantic: bool = False, recall: bool | None = None
) -> list[dict[str, Any]]:
    """Every tool this server offers, as ``{name, description, schema}``.

    Deliberately unprefixed, unlike the Hermes plugin's `surrealfs_*`: an MCP
    client namespaces tools by server, so prefixing here reads as
    `surrealfs:surrealfs_ls`.

    `recall` decides whether `brain_recall` is among them; the default asks
    Spectron whether it is configured. Advertising it unconditionally would put a
    tool in front of the model whose every answer is "Spectron is not configured",
    which it cannot act on and cannot fix.
    """
    if recall is None:
        recall = spectron.configured()
    specs = [
        {
            "name": definition["name"],
            "description": definition["description"],
            "schema": definition["input_schema"],
        }
        for definition in tool_definitions(semantic=semantic)
    ]
    if recall:
        specs.append(
            {
                "name": RECALL_TOOL,
                "description": RECALL_DESCRIPTION,
                "schema": RECALL_SCHEMA,
            }
        )
    return specs


async def run_tool(
    ctx: ToolContext,
    name: str,
    arguments: dict[str, Any],
    *,
    semantic: bool = False,
    mirror: Mirror = spectron.mirror,
) -> str:
    """Run one tool and mirror to Spectron whatever it wrote.

    Errors come back as text, the same as `json_tools.call_tool`: a tool-calling
    loop needs something the model can read and correct.

    A mirror failure is appended to the result rather than raised. The write to
    SurrealFS has already happened and must not be rolled back over it -- but
    swallowing the failure would let the memory layer drift out of date with
    nobody the wiser, so it is said out loud where the model and the user both
    see it.

    `RECALL_TOOL` is handled whether or not Spectron is configured, even though
    `tool_specs` only advertises it when it is: a client holding a tool list from
    before the key was removed has to get "not configured" back, not "unknown
    tool".
    """
    if name == RECALL_TOOL:
        try:
            return await spectron.recall(
                str(arguments.get("query", "")),
                int(arguments.get("k") or spectron.RECALL_K),
            )
        except Exception as exc:  # noqa: BLE001 -- any transport failure is the model's to report
            return f"Error: could not reach Spectron: {exc}"

    result = await call_tool(ctx, name, arguments, semantic=semantic)
    path = _to_mirror(name, arguments, result)
    if path is None:
        return result
    try:
        await mirror(path, await ctx.fs.read_text(path))
    except SurrealFsError:
        # The file is gone or is not text after all -- there is nothing to mirror,
        # and the tool result already says what happened.
        return result
    except Exception as exc:  # noqa: BLE001 -- see the docstring
        return f"{result}\n\n(brain sync failed for {path}: {exc})"
    return result


def _to_mirror(name: str, arguments: dict[str, Any], result: str) -> str | None:
    """The path to mirror after this call, or None."""
    if name not in MIRRORED or result.startswith("Error:"):
        return None
    raw = arguments.get("path")
    if not isinstance(raw, str) or not raw:
        return None
    try:
        path = normalize(raw)
    except SurrealFsError:
        return None
    root = normalize(os.environ.get("SURREALFS_MIRROR_ROOT", "/"))
    if root == "/" or path == root or path.startswith(f"{root}/"):
        return path
    return None


async def serve() -> None:
    """Serve MCP over stdio until the client disconnects."""
    # The SDK, not this package: Python 3 has no implicit relative imports, so
    # `mcp` here resolves to the top-level distribution even though this module
    # is `surrealfs.integrations.mcp`. Deliberate, and it only looks wrong.
    import mcp.types as types
    from mcp.server.lowlevel.server import Server
    from mcp.server.stdio import stdio_server

    from ... import __version__
    from ...fs import SurrealFs
    from ...schema import apply_schema
    from .._connect import agent_user, connect

    semantic = _semantic()
    recall = spectron.configured()
    if not recall:
        # Said out loud, for the same reason as the `SURREALFS_SEMANTIC` line
        # below: a tool that is simply absent looks identical, from the client
        # side, to a server that failed to start.
        print(
            "surrealfs-mcp: Spectron is not configured, so this is a filesystem "
            "server only and `brain_recall` is not offered. Set "
            "SPECTRON_CONTEXT_ID and SPECTRON_API_KEY to enable it.",
            file=sys.stderr,
        )
    db = await connect()
    try:
        # Usually a no-op -- the table is already there on a shared database, and
        # the credential may hold no right to define one. Same answer either way:
        # an un-applied schema is no reason to refuse to serve.
        await apply_schema(db)
    except Exception as exc:  # noqa: BLE001 -- any DDL failure, same answer
        print(f"surrealfs-mcp: could not apply the schema: {exc}", file=sys.stderr)
    embed = None
    if semantic:
        # Only when asked for. `surrealfs.embed` imports `openai` at module
        # level, and that lives in the `embed` extra, not `mcp` -- importing
        # it unconditionally made the server fail to start for everyone who
        # installed the plugin, which a client reports only as "no such tools".
        try:
            from ...embed import make_embedder

            embed = make_embedder()
        except ImportError as exc:
            print(
                f"surrealfs-mcp: SURREALFS_SEMANTIC is set but {exc.name} is "
                "missing, so `search` will be full-text only. Install: pip "
                "install 'surrealfs[mcp,embed]'",
                file=sys.stderr,
            )
            semantic = False
    ctx = ToolContext(fs=SurrealFs(db, user=agent_user()), embed=embed)
    listed = [
        types.Tool(
            name=spec["name"],
            description=spec["description"],
            inputSchema=spec["schema"],
        )
        for spec in tool_specs(semantic=semantic, recall=recall)
    ]

    async def on_list_tools(_ctx: Any, _params: Any) -> types.ListToolsResult:
        return types.ListToolsResult(tools=listed)

    async def on_call_tool(_ctx: Any, params: Any) -> types.CallToolResult:
        text = await run_tool(
            ctx, params.name, params.arguments or {}, semantic=semantic
        )
        return types.CallToolResult(content=[types.TextContent(type="text", text=text)])

    server = Server(
        SERVER_NAME,
        version=__version__,
        instructions=(
            "A persistent filesystem shared with other agents and with the people "
            "who own them"
            + (", plus the Spectron memory behind it" if recall else "")
            + ". Not the local disk."
        ),
        on_list_tools=on_list_tools,
        on_call_tool=on_call_tool,
    )
    try:
        async with stdio_server() as (read, write):
            await server.run(read, write, server.create_initialization_options())
    finally:
        await db.close()


async def selftest() -> int:
    """Check the server can do its job, and say what it found.

    `surrealfs-mcp --selftest`. Exists because the failure everyone hits is a
    client not launching the server at all, and the symptom -- a model saying it
    has no such tools -- looks identical whether the fault is the config file, the
    database, the client, or Spectron. This separates them: run it, and whatever
    it prints is the layer to fix. Printing to stdout is safe here; nothing is
    speaking MCP on it.
    """
    from ...fs import SurrealFs
    from .._connect import agent_user, connected

    print(f"config    {config_path()}{'' if config_path().is_file() else '  (absent)'}")
    print(f"server    {os.environ.get('SURREALDB_URL')}")
    print(
        f"database  {os.environ.get('SURREALDB_NAMESPACE', 'surrealfs')}"
        f" / {os.environ.get('SURREALDB_DATABASE', 'demo')}"
    )
    print(f"acting as {agent_user()}")
    print(f"tools     {len(tool_specs(semantic=_semantic()))}")  # follows the env
    try:
        async with connected() as db:
            entries = await SurrealFs(db, user=agent_user()).ls("/")
    except Exception as exc:  # noqa: BLE001 -- the point is to report it, not raise
        print(f"\nFAIL      could not reach the filesystem: {exc}")
        return 1
    listing = ", ".join(e.filename for e in entries) or "(empty)"
    print(f"ls /      {listing}")
    if not [e for e in entries if e.filename != "home"]:
        # The failure this whole server is careful about: a wrong namespace or
        # database connects fine and simply has nothing in it, which reads as a
        # healthy empty brain rather than as a misconfiguration.
        print(
            "\nWARN      connected, but this database is empty apart from /home."
            "\n          Check SURREALDB_NAMESPACE and SURREALDB_DATABASE -- a"
            "\n          wrong pair connects successfully to the wrong filesystem."
        )
        return 1

    if not spectron.configured():
        print("spectron  not configured (SPECTRON_CONTEXT_ID / SPECTRON_API_KEY)")
        # Not a warning. A filesystem server is the product; Spectron is an extra
        # you opt into, and 0 here says so rather than nagging about a service
        # nobody has to buy.
        print("\nOK        filesystem reachable; Spectron memory off, so no")
        print(f"          {RECALL_TOOL} tool. Everything else works.")
        return 0
    # The default, not `.get('SPECTRON_URL')`: printing `None` for the host the
    # server is in fact about to talk to defeats the point of a selftest that
    # exists to name the layer that failed.
    url = os.environ.get("SPECTRON_URL") or spectron.DEFAULT_URL
    print(f"spectron  {url} scope={spectron.scope()}")
    try:
        first = (await spectron.recall("status", 1)).splitlines()[0]
    except Exception as exc:  # noqa: BLE001 -- same
        print(f"\nFAIL      Spectron unreachable: {exc}")
        return 1
    print(f"recall    {first}")
    print("\nOK        filesystem and Spectron both reachable")
    return 0


def main() -> int:
    """Console-script entry point (`surrealfs-mcp`).

    Nothing here may write to stdout -- it is the MCP transport, and one stray
    `print` corrupts the stream for the rest of the session. Diagnostics go to
    stderr, which Desktop files under its MCP logs.
    """
    if "--selftest" in sys.argv[1:]:
        loaded = _load_config()
        if not os.environ.get("SURREALDB_URL"):
            found = "exists but sets no SURREALDB_URL" if loaded else "absent"
            print(f"config    {config_path()}  ({found})")
            print("\nFAIL      nothing to connect to")
            return 1
        return asyncio.run(selftest())
    loaded = _load_config()
    if not os.environ.get("SURREALDB_URL"):
        # Refuse rather than fall back to ws://localhost:8000 and the `demo`
        # database. That connects *successfully* to an empty filesystem, and an
        # agent that finds an empty brain reports a clean bill of health for a
        # company it never reached -- the worst failure this server has.
        where = (
            f"{config_path()} exists but sets no SURREALDB_URL"
            if loaded
            else (f"no configuration found at {config_path()}")
        )
        print(
            f"surrealfs-mcp: {where}. Create it with at least:\n"
            "    SURREALDB_URL=wss://your-instance.surreal.cloud/rpc\n"
            "    SURREALDB_USER=...\n    SURREALDB_PASS=...\n"
            "    SURREALDB_NAMESPACE=...\n    SURREALDB_DATABASE=...",
            file=sys.stderr,
        )
        return 2
    if unset := _unexpanded():
        # For a hand-written client config -- the Desktop `env` block the README
        # keeps as a fallback -- which is a plain JSON file with no expansion of
        # any kind, so `"SURREALDB_PASS": "${SURREALDB_PASS}"` arrives as that
        # literal text. Refuse rather than connect with it: a live connection to
        # the wrong place is the worst failure this server has, and an agent that
        # finds an empty brain reports a clean risk board for a company it never
        # reached. (The plugin's own `.mcp.json` carries no `env` block at all --
        # see `tests/test_claude.py` -- so this only ever fires on a hand-rolled
        # one.)
        print(
            "surrealfs-mcp: not configured -- "
            + ", ".join(unset)
            + " "
            + ("is" if len(unset) == 1 else "are")
            + f" unset in the environment this server was launched with. Put them "
            f"in {config_path()}, or set them where your client can see them (a "
            "shell profile, or the `env` block of its MCP config).",
            file=sys.stderr,
        )
        return 2
    try:
        asyncio.run(serve())
    except KeyboardInterrupt:
        return 0
    except Exception as exc:  # noqa: BLE001 -- a traceback on stdout would be worse
        print(f"surrealfs-mcp: {exc}", file=sys.stderr)
        return 1
    return 0


def config_path() -> Path:
    """The file the server reads its configuration from.

    ``SURREALFS_ENV_FILE`` names it outright; otherwise it is
    ``$XDG_CONFIG_HOME/surrealfs/env``, falling back to ``~/.config/surrealfs/env``.
    """
    if explicit := os.environ.get("SURREALFS_ENV_FILE"):
        return Path(explicit).expanduser()
    base = os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config"
    return Path(base).expanduser() / CONFIG_DIR / CONFIG_NAME


def _load_config() -> Path | None:
    """Load the config file into the environment, if there is one.

    ``override=False``, so anything already set wins: an ``env`` block in the
    client's MCP config, or an export in the shell that launched it, still beats
    the file. That keeps the file a default rather than a mandate, and leaves the
    hand-written client configs documented in the READMEs working unchanged.
    """
    path = config_path()
    if not path.is_file():
        return None
    if path.stat().st_mode & 0o077:
        # It holds a database password and an API key.
        print(
            f"surrealfs-mcp: {path} is readable by other users; chmod 600 it.",
            file=sys.stderr,
        )
    try:
        from dotenv import load_dotenv
    except ImportError:  # pragma: no cover -- only without the `mcp` extra
        print(
            "surrealfs-mcp: python-dotenv is missing, so "
            f"{path} was ignored. Install: pip install 'surrealfs[mcp]'",
            file=sys.stderr,
        )
        return None
    # `interpolate=False`: this file holds a database password and an API key,
    # and dotenv otherwise rewrites `${...}` inside a value -- so a secret that
    # happens to contain one comes out mangled, or empty if nothing resolves it.
    load_dotenv(path, override=False, interpolate=False)
    return path


def _semantic() -> bool:
    return os.environ.get("SURREALFS_SEMANTIC", "").lower() in {"1", "true", "yes"}


def _unexpanded() -> list[str]:
    """Connection variables that arrived as an unexpanded ``${VAR}`` placeholder."""
    return [
        name
        for name, value in os.environ.items()
        if name.startswith(("SURREALDB_", "SURREALFS_", "SPECTRON_"))
        and value.startswith("${")
        and value.endswith("}")
    ]
