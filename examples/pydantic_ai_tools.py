"""pydantic-ai tool definitions backed by PySurrealFs.

This example shows how to expose SurrealFs operations to an agent using a
`FunctionToolset`. The functions stay thin so you can swap the client setup
(mem vs ws) without touching the tool definitions.
"""

import asyncio
from pathlib import Path
from typing import Callable

import logfire
from pydantic import BaseModel, Field
from pydantic_ai import Agent, FunctionToolset
from surrealfs_py import PySurrealFs  # type: ignore

# Remote backend; SurrealDB must be running at this endpoint.
fs = PySurrealFs.connect_ws("ws://localhost:8000")

_ = logfire.configure(send_to_logfire="if-token-present")
logfire.instrument_pydantic_ai()
logfire.instrument_anthropic()

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


class TouchArgs(BaseModel):
    path: str = Field(..., description="Path to create or update")


async def touch(args: TouchArgs) -> str:
    return run_tool(lambda: fs.touch(args.path))


class MkdirArgs(BaseModel):
    path: str = Field(..., description="Directory path to create (parents included)")


async def mkdir(args: MkdirArgs) -> str:
    return run_tool(lambda: fs.mkdir(args.path))


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


TOOLSET = FunctionToolset()
TOOLSET.add_function(ls, description=load_description("ls", "List files in SurrealFs"))
TOOLSET.add_function(cat, description=load_description("cat", "Read a file"))
TOOLSET.add_function(
    tail, description=load_description("tail", "Read the last N lines of a file")
)
TOOLSET.add_function(
    write_file, description=load_description("write-file", "Write file contents")
)
TOOLSET.add_function(
    touch, description=load_description("touch", "Create a file if missing")
)
TOOLSET.add_function(
    mkdir, description=load_description("mkdir", "Create a directory (with parents)")
)
TOOLSET.add_function(cp, description=load_description("cp", "Copy a file"))
TOOLSET.add_function(cd, description=load_description("cd", "Change working directory"))
TOOLSET.add_function(
    pwd, description=load_description("pwd", "Print working directory")
)


async def demo() -> None:
    agent = Agent(
        model="claude-haiku-4-5-20251001",
        toolsets=[TOOLSET],
        system_prompt=(
            "You are a helpful filesystem agent. Use the SurrealFs tools to"
            " manage files; prefer absolute paths under / unless instructed"
            " otherwise."
        ),
    )

    result = await agent.run(
        "Create /demo/hello.txt containing 'hello world', then show its content",
    )
    print(result.output)


if __name__ == "__main__":
    asyncio.run(demo())
