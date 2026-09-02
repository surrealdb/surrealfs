"""The note-taking agent behind the browser's chat panel.

Its filesystem is the one the page is showing, so anything it writes appears in
the tree, and anything the user writes is there for it to read.
"""

from __future__ import annotations

from pydantic_ai import Agent
from pydantic_ai.capabilities import WebFetch, WebSearch

from ..integrations.pydantic_ai import build_fs_toolset
from ..tools import ToolContext

INSTRUCTIONS = """\
You organise the user's thoughts, conversations, and notes into a well-structured
filesystem. You have a persistent filesystem; treat it as your memory.

The user reads this filesystem too, so it is also how you hand over a plan, a
document, or a to-do list. Write files they can open on their own.

Conventions to follow:
- /home/agent/ is yours — working notes, running to-dos, anything you keep for
  your own benefit. Nothing exists there yet, so start by listing / to see what
  is around rather than reading a file blind. Homes are private (0700); other
  users' homes are not yours to read, and the filesystem enforces it.
- Record anything you learn about the user's preferences under /preferences/, one
  file per topic: /preferences/voice.md, /preferences/workflow.md.
- Give each project or task a folder under /projects/<project_name>/, with the
  notes and the current to-do list inside it.

Files have unix permissions. Everything outside /home/ is shared read-write,
which is how you hand work to the user; `ls` shows the mode and owner, and
`chmod` changes them. Only reach for `chmod` when something genuinely should
not be read by others.

Before answering, consider whether anything in this conversation belongs in a
file. Prefer `edit` over `write_file` when changing part of an existing file, and
search before you create — the note you want may already exist.
"""


def build_chat_agent(
    model: str = "anthropic:claude-haiku-4-5",
    enable_web_tools: bool = True,
    semantic: bool = False,
) -> Agent[ToolContext, str]:
    """Build the agent.

    Note the ``anthropic:`` prefix on the model name: pydantic-ai validates bare
    strings against its own list of known models, so a plain ``claude-opus-5``
    raises ``UserError: Unknown model``. The prefix selects the provider
    explicitly and passes the ID straight through.

    ``semantic=True`` makes the `search` tool hybrid, which only pays off if the
    deps carry an embedder: ``ToolContext(fs=fs, embed=embed)``.
    """
    return Agent(
        model,
        toolsets=[build_fs_toolset(semantic=semantic)],
        instructions=INSTRUCTIONS,
        deps_type=ToolContext,
        # Server-side tools are `capabilities` in pydantic-ai 2.x (`builtin_tools`
        # from 1.x is gone). Pass the *capability* wrappers from
        # pydantic_ai.capabilities, not the WebSearchTool/WebFetchTool objects:
        # `capabilities` also accepts plain callables, so a raw tool object is
        # taken for one and invoked, failing at request time with
        # "'WebSearchTool' object is not callable".
        capabilities=[WebSearch(), WebFetch()] if enable_web_tools else [],
        # pydantic-ai defaults Anthropic to max_tokens=4096, which a `write_file`
        # of a decent-sized note blows through mid-tool-call: the run then dies
        # with "token limit exceeded while generating a tool call" and truncated
        # arguments. Haiku 4.5 allows 64k output; 16k is headroom without
        # licensing a runaway.
        model_settings={"max_tokens": 16000},
    )
