# surrealfs

A filesystem for agents, backed by SurrealDB.

Give an agent somewhere durable to keep its work. SurrealFS is a `file` table
plus the tools to hand it to a model — files and folders, full-text and semantic
search, all queryable with SurrealQL because it is just a table.

Here is a tree one agent built for its user over a few conversations:

```
/
├── notes.md                           # Master profile & current projects overview
├── reminders.md                       # Daily to-dos
│
├── preferences/
│   ├── professional.md                # Work identity, tech preferences, leadership style
│   ├── hobbies.md                     # FPV drones, eSports, tennis, open source
│   └── content.md                     # Content creation preferences & LinkedIn strategy
│
└── projects/
    ├── knowledge_graphs_ai_presentation/
    │   ├── outline.md                 # 40-min structure (6 sections)
    │   └── talking_points.md          # Deep talking points, examples, Q&A prep
    │
    └── content/
        ├── ideas.md                   # Blog post ideas (series + individual)
        ├── demo_library.md            # 6 core demos (reusable across formats)
        │
        └── surrealdb_marketing/
            ├── enterprise_strategy.md # Unified comparison approach
            └── core_arguments.md      # 5 key arguments
```

## Requirements

- Python 3.12+
- **SurrealDB 3.x.** The schema uses `COMPUTED` fields and `FULLTEXT` indexes,
  neither of which exists in 2.x. The embedded engine bundled with the Python
  SDK (`mem://`, `file://`) is a 2.0.0 core and *cannot* run this schema — use a
  real server.

## Install

```bash
pip install surrealfs                    # core
pip install "surrealfs[pydantic-ai]"     # + the pydantic-ai toolset
```

## Quickstart

Start a database (`just db`), then:

```python
from surrealdb import AsyncSurreal
from surrealfs import SurrealFs, apply_schema

db = AsyncSurreal("ws://localhost:8000/rpc")
await db.signin({"username": "root", "password": "root"})
await db.use("surrealfs", "demo")
await apply_schema(db)               # defines the `file` table; safe to re-run

fs = SurrealFs(db)
await fs.write_text("/notes/today.md", "# Today\n- ship the refactor")
await fs.edit("/notes/today.md", "ship", "shipped")
await fs.ls("/notes")
await fs.search_text("refactor")
```

You create and own the connection. SurrealFS never connects, signs in, or
selects a namespace — so the `file` table's `owner = $auth.id` permissions key
off *your* auth context.

## Three ways to give it to an agent

### 1. The plain async API

`SurrealFs` returns structured `FileEntry` and `SearchHit` objects, not
pre-formatted strings. Build whatever integration you like on top.

| | |
|---|---|
| Read | `read_text` `read_bytes` `tail` `ls` `glob` `stat` `exists` |
| Write | `write_text` `write_bytes` `edit` `touch` `mkdir` |
| Organise | `mv` `cp` `rm` |
| Search | `search_text` `search_semantic` `reindex_embeddings` |

### 2. pydantic-ai

```python
from pydantic_ai import Agent
from surrealfs.integrations.pydantic_ai import build_fs_toolset
from surrealfs.tools import ToolContext

agent = Agent("claude-sonnet-5", deps_type=ToolContext,
              toolsets=[build_fs_toolset()])

await agent.run("Tidy up my notes", deps=ToolContext(fs=SurrealFs(db)))
```

Recoverable mistakes — a wrong path, text that is not in the file — are raised
as `ModelRetry` so the agent corrects itself. Real failures propagate.

### 3. Raw JSON tool schemas

No framework, just dicts:

```python
from surrealfs.integrations.json_tools import call_tool, tool_definitions
from surrealfs.tools import ToolContext

ctx = ToolContext(fs=SurrealFs(db))
tools = tool_definitions(flavor="anthropic")     # or flavor="openai"

# inside your tool-use loop:
result = await call_tool(ctx, block.name, block.input)
```

See `examples/anthropic_loop.py` for a complete loop in ~40 lines.

All three surfaces are generated from one registry in `surrealfs/tools/`, so
they cannot drift apart. Tool descriptions are markdown in
`surrealfs/tools/docs/` — edit them as prose; they are prompt text.

## Semantic search

SurrealFS never calls an embedding model. You provide the function, so you pick
the provider:

```python
count = await fs.reindex_embeddings(embed, version="openai:text-embedding-3-small")
hits = await fs.search_semantic(await embed("how do I get paid"), k=5)
```

Re-running the indexer is cheap: it skips files whose content has not changed
since they were embedded. To offer it as a tool, pass the embedder in the
context and opt in — `build_fs_toolset(semantic=True)` with
`ToolContext(fs=fs, embed=embed)`. See `examples/semantic_search.py`.

## The schema

`surrealfs/schema/file.surql` is the whole data model, and it is worth reading.
The key idea: a file's `parent` is a record link and `path` is a **computed**
field derived from the parent chain. Nothing stores a path, so renaming or
moving a folder is a single `UPDATE` and every descendant's path follows for
free.

```surql
DEFINE FIELD path ON file COMPUTED (
    ($this.parent.path || '') + '/' + <string>$this.filename
) TYPE string;
```

A folder is just a row with no `content`, no `file` bytes, and no `symlink` —
also computed, as `is_folder`. `hash` is maintained by an event, and there are
HNSW and BM25 indexes for the two search modes.

`apply_schema(db, include_user=True)` also defines the optional `user` table and
record access that the `owner` permissions need for real multi-tenancy.

## Development

```bash
just db        # start SurrealDB in one terminal
just test      # run the suite against it (starts its own server if needed)
just check     # lint + test
just agent     # the note-taking chat agent on :7932
```

Tests start a throwaway `surreal` server; set `SURREALFS_TEST_URL` to reuse one
you already have.
