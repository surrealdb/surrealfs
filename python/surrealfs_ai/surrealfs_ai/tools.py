"""pydantic-ai tool definitions backed by PySurrealFs.

This example shows how to expose SurrealFs operations to an agent using a
`FunctionToolset`. The functions stay thin so you can swap the client setup
(mem vs ws) without touching the tool definitions.
"""

from pathlib import Path
from typing import Any, Callable

from pydantic import BaseModel, Field
from pydantic_ai import FunctionToolset

# TODO: generate types
from surrealfs_py import PySurrealFs  # type: ignore

DOCS_DIR = Path(__file__).with_name("tool_docs")


def run_tool(call: Callable[[], str]) -> str:
    try:
        return call()
    except Exception as e:
        return f"error: {e}"


def load_description(tool_name: str, fallback: str) -> str:
    path = DOCS_DIR / f"{tool_name}.md"
    if path.exists():
        text = path.read_text(encoding="utf-8").strip()
        if text:
            return text
    return fallback


class LsArgs(BaseModel):
    path: str = Field("/", description="Path to list; absolute or relative to cwd")
    all: bool = Field(False, description="Include dotfiles")
    long: bool = Field(False, description="Show size info")
    recursive: bool = Field(False, description="Recurse into subdirectories")
    dir_only: bool = Field(False, description="List only directories")
    human: bool = Field(False, description="Use human-readable sizes")


class GlobArgs(BaseModel):
    pattern: str = Field(..., description="Glob pattern; absolute or relative to cwd")


def build_toolset(ns: str, db: str) -> FunctionToolset[Any]:
    # Remote backend; SurrealDB must be running at this endpoint.
    # TODO: move this into agent deps
    fs = PySurrealFs.connect_ws("ws://localhost:8000", ns, db)

    async def ls(args: LsArgs) -> str:
        return run_tool(
            lambda: fs.ls(
                path=args.path,
                all=args.all,
                long=args.long,
                recursive=args.recursive,
                dir_only=args.dir_only,
                human=args.human,
            )
        )

    async def glob(args: GlobArgs) -> str:
        return run_tool(lambda: fs.glob(args.pattern))

    class CatArgs(BaseModel):
        path: str = Field(..., description="File to read")

    async def cat(args: CatArgs) -> str:
        return run_tool(lambda: fs.cat(args.path))

    class TailArgs(BaseModel):
        path: str = Field(..., description="File to read")
        n: int = Field(10, description="Number of lines from the end")

    async def tail(args: TailArgs) -> str:
        return run_tool(lambda: fs.tail(args.path, args.n))

    class WriteFileArgs(BaseModel):
        path: str = Field(..., description="Destination path")
        content: str = Field(..., description="File contents to write")

    async def write_file(args: WriteFileArgs) -> str:
        return run_tool(lambda: fs.write_file(args.path, args.content))

    class EditArgs(BaseModel):
        path: str = Field(..., description="File to edit")
        old: str = Field(..., description="Substring or pattern to replace")
        new: str = Field(..., description="Replacement text")
        replace_all: bool = Field(
            False,
            description="Replace all occurrences (default replaces first only)",
        )

    async def edit(args: EditArgs) -> str:
        return run_tool(
            lambda: fs.edit(args.path, args.old, args.new, args.replace_all)
        )

    class TouchArgs(BaseModel):
        path: str = Field(..., description="Path to create or update")

    async def touch(args: TouchArgs) -> str:
        return run_tool(lambda: fs.touch(args.path))

    class MkdirArgs(BaseModel):
        path: str = Field(
            ..., description="Directory path to create (parents included)"
        )
        parents: bool = Field(False, description="Create parent directories as needed")

    async def mkdir(args: MkdirArgs) -> str:
        return run_tool(lambda: fs.mkdir(args.path, args.parents))

    class CpArgs(BaseModel):
        src: str = Field(..., description="Source file path")
        dest: str = Field(..., description="Destination file path")

    async def cp(args: CpArgs) -> str:
        return run_tool(lambda: fs.cp(args.src, args.dest))

    class CdArgs(BaseModel):
        target: str = Field(..., description="Directory to switch into")

    async def cd(args: CdArgs) -> str:
        return run_tool(lambda: fs.cd(args.target))

    async def pwd() -> str:
        return run_tool(fs.pwd)

    TOOLSET: FunctionToolset[Any] = FunctionToolset()
    TOOLSET.add_function(
        ls,
        description=load_description("ls", "List files in SurrealFs"),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        glob,
        description=load_description("glob", "Match paths using a glob pattern"),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        cat, description=load_description("cat", "Read a file"), takes_ctx=False
    )
    TOOLSET.add_function(
        tail,
        description=load_description("tail", "Read the last N lines of a file"),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        write_file,
        description=load_description("write-file", "Write file contents"),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        edit,
        description=load_description(
            "edit", "Replace text in a file (optionally all occurrences)"
        ),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        touch,
        description=load_description("touch", "Create a file if missing"),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        mkdir,
        description=load_description("mkdir", "Create a directory (with parents)"),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        cp, description=load_description("cp", "Copy a file"), takes_ctx=False
    )
    TOOLSET.add_function(
        cd,
        description=load_description("cd", "Change working directory"),
        takes_ctx=False,
    )
    TOOLSET.add_function(
        pwd,
        description=load_description("pwd", "Print working directory"),
        takes_ctx=False,
    )

    return TOOLSET


instructions = (
    "You are a helpful assistant that organizes my thoughts, conversations, notes, into a well-structured text file system."
    "Every time you learn something about my preferences, store it in a file in the /preferences folder. For example, create files like /preferences/food.md, /preferences/music.md, /preferences/books.md, etc."
    "When I talk about a project or task, organize the notes and current to-do list in a /project/<project_name> folder. For example, /project/social_media/2026/post_calendar_january.md or /project/support/solutions/vector_index.md"
    "Write your main notes in /notes.md, and read them every time we interact."
    "Before you answer, consider updating the /notes.md file with your latest thoughts and insights."
)
