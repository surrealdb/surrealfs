"""SurrealFS -- a filesystem for agents, backed by SurrealDB.

Point it at a connected ``AsyncSurreal`` handle and give an agent tools to read,
write, organise, and search files:

    from surrealdb import AsyncSurreal
    from surrealfs import SurrealFs, apply_schema

    db = AsyncSurreal("ws://localhost:8000/rpc")
    await db.signin({"username": "root", "password": "root"})
    await db.use("surrealfs", "demo")
    await apply_schema(db)

    fs = SurrealFs(db, user="alice")   # or user=ROOT to bypass permissions
    await fs.write_text("/notes/today.md", "# Today\\n")
    await fs.ls("/notes")

Files carry a unix ``owner`` and ``mode``. Everything is shared read-write
except ``/home/<user>``, which is created 0700; ``chmod`` is how you tighten or
open anything else.

Three integration surfaces are built on the shared tool registry:

* :mod:`surrealfs.tools` -- the tool registry, shared by the surfaces below.
* :mod:`surrealfs.integrations.pydantic_ai` -- a pydantic-ai ``FunctionToolset``.
* :mod:`surrealfs.integrations.json_tools` -- plain JSON Schema tool definitions
  and a dispatcher, for raw Anthropic or OpenAI tool-calling loops.
* :mod:`surrealfs.integrations.hermes` -- a Hermes plugin.
"""

from __future__ import annotations

from .errors import (
    AlreadyExists,
    DirectoryNotEmpty,
    InvalidPath,
    IsADirectory,
    NotADirectory,
    NotATextFile,
    NotFound,
    PermissionDenied,
    QueryError,
    SurrealFsError,
)
from .fs import ROOT, SurrealFs, default_mode
from .models import FOLDER_CONTENT_TYPE, FileEntry, SearchHit, format_mode
from .schema import apply_schema, schema_sql

__version__ = "0.2.0"

__all__ = [
    "FOLDER_CONTENT_TYPE",
    "AlreadyExists",
    "DirectoryNotEmpty",
    "FileEntry",
    "InvalidPath",
    "IsADirectory",
    "NotADirectory",
    "NotATextFile",
    "ROOT",
    "NotFound",
    "PermissionDenied",
    "QueryError",
    "SearchHit",
    "SurrealFs",
    "SurrealFsError",
    "__version__",
    "apply_schema",
    "default_mode",
    "format_mode",
    "schema_sql",
]
