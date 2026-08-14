<br>

<p align="center">
    <img width=120 src="https://raw.githubusercontent.com/surrealdb/icons/main/surreal.svg" />
</p>

<h1 align="center">SurrealFS</h1><br/>
<p align="center">A filesystem for agents, backed by SurrealDB.</p>

<br>

<p align="center">
    <a href="https://surrealdb.com"><img src="https://cdn.jsdelivr.net/gh/glincker/thesvg@main/public/icons/surrealdb/default.svg" alt="SurrealDB" width="48" height="48" /></a>
    &nbsp;
    <a href="https://github.com/surrealdb/surrealfs/blob/main/examples/semantic_search.py"> <img src="https://cdn.jsdelivr.net/gh/glincker/thesvg@main/public/icons/python/default.svg" alt="Python" width="48" height="48" /></a>
    &nbsp;
    <a href="https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/pydantic_ai/README.md"><img src="https://cdn.jsdelivr.net/gh/glincker/thesvg@main/public/icons/pydantic/default.svg" alt="Pydantic" width="48" height="48" /></a>
    &nbsp;
    <a href="https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/hermes/README.md"><img src="https://cdn.jsdelivr.net/gh/glincker/thesvg@main/public/icons/nousresearch-hermes/default.svg" alt="NousResearch (Hermes)" width="48" height="48" /></a>
    &nbsp;
    <a href="https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/json_tools/README.md"><img src="https://cdn.jsdelivr.net/gh/glincker/thesvg@main/public/icons/openai/default.svg" alt="OpenAI" width="48" height="48" /></a>
    &nbsp;
    <a href="https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/json_tools/README.md"><img src="https://cdn.jsdelivr.net/gh/glincker/thesvg@main/public/icons/anthropic/default.svg" alt="Anthropic" width="48" height="48" /></a>
</p>

<br>

<p align="center">
    <a href="https://surrealdb.com/discord"><img src="https://img.shields.io/discord/902568124350599239?label=discord&style=flat-square&color=5a66f6"></a>
    &nbsp;
    <a href="https://x.com/surrealdb"><img alt="X (formerly Twitter) Follow" src="https://img.shields.io/twitter/follow/surrealdb"></a>
    &nbsp;
    <a href="https://www.linkedin.com/company/surrealdb/"><img src="https://img.shields.io/badge/linkedin-connect_with_us-0a66c2.svg?style=flat-square"></a>
    &nbsp;
    <a href="https://www.youtube.com/@SurrealDB"><img src="https://img.shields.io/badge/youtube-subscribe-fc1c1c.svg?style=flat-square"></a>
</p>

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

![Hermes integration](assets/surrealfs-browser.png)

## Requirements

- Python 3.12+
- **A SurrealDB 3.x server.**.

## Install

```bash
pip install surrealfs                    # core, and the Hermes plugin
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
await apply_schema(db)  # defines the `file` table; safe to re-run

fs = SurrealFs(db)
await fs.write_text("/notes/today.md", "# Today\n- ship the refactor")
await fs.edit("/notes/today.md", "ship", "shipped")
await fs.ls("/notes")
await fs.search_text("refactor")
```

You create and own the connection. SurrealFS never connects, signs in, or
selects a namespace — so the `file` table's `owner = $auth.id` permissions key
off _your_ auth context.

## The plain async API

`SurrealFs` returns structured `FileEntry` and `SearchHit` objects, not
pre-formatted strings. Build whatever integration you like on top.

|          |                                                               |
| -------- | ------------------------------------------------------------- |
| Read     | `read_text` `read_bytes` `tail` `ls` `glob` `stat` `exists`   |
| Write    | `write_text` `write_bytes` `edit` `touch` `mkdir`             |
| Organise | `mv` `cp` `rm`                                                |
| Search   | `search` `search_text` `search_semantic` `reindex_embeddings` |

## Integrations

Four ways to hand it to an agent, each with its own README:

|                                                                          |                                                                       |                          |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------- | ------------------------ |
| **[pydantic-ai](surrealfs/integrations/pydantic_ai/README.md)**          | a `FunctionToolset`, with recoverable mistakes raised as `ModelRetry` | `surrealfs[pydantic-ai]` |
| **[Raw JSON tool schemas](surrealfs/integrations/json_tools/README.md)** | plain dicts in Anthropic or OpenAI shape, no framework                | core                     |
| **[Hermes](surrealfs/integrations/hermes/README.md)**                    | the 14 `surrealfs_*` tools plus a bundled notes skill                 | core                     |
| **[Hermes memory](surrealfs/integrations/hermes_memory/README.md)**      | files every completed turn, and recalls context before each one       | core                     |

The first three tool surfaces are generated from one registry in
`surrealfs/tools/`, so they cannot drift apart. Tool descriptions are markdown in
`surrealfs/tools/docs/` — edit them as prose; they are prompt text. The memory
provider is the odd one out: it exposes no tools of its own, because the Hermes
plugin's already cover explicit reads and writes.

## Semantic search

SurrealFS never calls an embedding model. You provide the function, so you pick
the provider:

```python
count = await fs.reindex_embeddings(embed, version="openai:text-embedding-3-small")
hits = await fs.search("how do I get paid", vector=await embed("how do I get paid"))
```

`fs.search` runs both arms and fuses them by rank — full-text scores are BM25 and
vector scores are distances, so there is nothing sane to normalise. Without a
`vector` it is full-text only. `search_text` and `search_semantic` remain
available if you want one arm on its own.

The full-text arm matches a file that shares **any** term with the query, so
`"how do I get paid"` returns candidates even without embeddings — the vector arm is
what makes it rank the invoicing note by _meaning_ rather than by shared words. That
default favours recall because the caller sees a ranked list of snippets and can
dismiss a weak hit at a glance, but cannot dismiss one it never saw.

Ranking is Okapi BM25, computed in Python over the tokens SurrealDB's own analyzer
produces for the query and each matching file — the same stemming the index matched
with, so a file saying "Prefers" scores for a query of "prefer" instead of scoring
zero. Rare words count for more than common ones, repetition saturates, and long
files are not rewarded for sprawl. Stopwords still match; they just get no vote.

What it cannot do is read intent: a query word that is beside the point still pulls
in files dense with it. That is the vector arm's job.

Pass `match="all"` when you want a precise filter and an empty result is a real
answer:

```python
hits = await fs.search("invoice 2026", match="all")   # both terms must appear
```

Re-running the indexer is cheap: it skips files whose content has not changed
since they were embedded. Agents get one `search` tool either way; pass the
embedder in the context and opt in to make it hybrid —
`build_fs_toolset(semantic=True)` with `ToolContext(fs=fs, embed=embed)`. See
`examples/semantic_search.py`.

An OpenAI embedder ships in `surrealfs.embed` (`make_embedder`,
`text-embedding-3-small`, the 1536 dimensions the HNSW index expects), along with
a daemon that keeps every vector current no matter what wrote the row:

```bash
just embed              # poll every 5s; --once for a single pass
```

It needs `OPENAI_API_KEY` and an already-applied schema (`just schema`). Under
Hermes there is no daemon to leave running — see [Keeping the vectors current
under Hermes](docs/hermes-indexer.md).

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

You can apply it without writing any code:

```bash
python -m surrealfs.schema                     # uses SURREALDB_* env vars
python -m surrealfs.schema --include-user
python -m surrealfs.schema --print             # dump the DDL, connect to nothing
```

## Development

```bash
just db        # start SurrealDB in one terminal
just schema    # apply the file table to it
just test      # run the suite (starts its own server if needed)
just check     # lint + test
just agent     # the note-taking chat agent on :7932
just loop "…"  # the framework-free Anthropic tool-use loop
```

Tests start a throwaway `surreal` server; set `SURREALFS_TEST_URL` to reuse one
you already have.
