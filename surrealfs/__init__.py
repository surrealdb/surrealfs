"""SurrealFS -- a filesystem for agents, backed by SurrealDB.

Point it at a connected ``AsyncSurreal`` handle and give an agent tools to read,
write, organise, and search files:

    from surrealdb import AsyncSurreal
    from surrealfs import SurrealFs, apply_schema

    db = AsyncSurreal("ws://localhost:8000/rpc")
    await db.signin({"username": "root", "password": "root"})
    await db.use("surrealfs", "demo")
    await apply_schema(db)

    fs = SurrealFs(db)
    await fs.write_text("/notes/today.md", "# Today\\n")
    await fs.ls("/notes")

Three integration surfaces are built on that core:

* :mod:`surrealfs.tools` -- the tool registry, shared by both surfaces below.
* :mod:`surrealfs.integrations.pydantic_ai` -- a pydantic-ai ``FunctionToolset``.
* :mod:`surrealfs.integrations.json_tools` -- plain JSON Schema tool definitions
  and a dispatcher, for raw Anthropic or OpenAI tool-calling loops.
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
    QueryError,
    SurrealFsError,
)
from .fs import SurrealFs
from .models import FOLDER_CONTENT_TYPE, FileEntry, SearchHit
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
    "NotFound",
    "QueryError",
    "SearchHit",
    "SurrealFs",
    "SurrealFsError",
    "__version__",
    "apply_schema",
    "schema_sql",
]
