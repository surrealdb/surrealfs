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
- **A SurrealDB 3.x server.** The schema uses `COMPUTED` fields and `FULLTEXT`
  indexes, neither of which exists in 2.x.

  The embedded engine (`mem://`, `file://` via `surrealdb[embedded]`) is not
  usable yet. As of `surrealdb-embedded` 3.0.0a4 it parses the schema, but its
  3.0.0-alpha core returns `null` for computed fields on any read that goes
  through an index — so `path` and `is_folder` come back empty from `ls`, path
  resolution, and search. The same query is correct against a 3.2.x server.

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
off *your* auth context.

## Five ways to give it to an agent

### 1. The plain async API

`SurrealFs` returns structured `FileEntry` and `SearchHit` objects, not
pre-formatted strings. Build whatever integration you like on top.

| | |
|---|---|
| Read | `read_text` `read_bytes` `tail` `ls` `glob` `stat` `exists` |
| Write | `write_text` `write_bytes` `edit` `touch` `mkdir` |
| Organise | `mv` `cp` `rm` |
| Search | `search` `search_text` `search_semantic` `reindex_embeddings` |

### 2. pydantic-ai

```python
from pydantic_ai import Agent
from surrealfs.integrations.pydantic_ai import build_fs_toolset
from surrealfs.tools import ToolContext

agent = Agent(
    "anthropic:claude-opus-5",  # the provider prefix is required by pydantic-ai
    deps_type=ToolContext,
    toolsets=[build_fs_toolset()],
)

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
tools = tool_definitions(flavor="anthropic")  # or flavor="openai"

# inside your tool-use loop:
result = await call_tool(ctx, block.name, block.input)
```

See `examples/anthropic_loop.py` for a complete loop in ~40 lines.

### 4. Hermes

The package ships a [Hermes](https://hermes-agent.nousresearch.com) plugin,
registered through the `hermes_agent.plugins` entry point. Hermes runs from its
own virtualenv, so install it there rather than into your project:

```bash
uv pip install --python ~/.hermes/hermes-agent/venv/bin/python \
  "surrealfs @ git+https://github.com/surrealdb/surrealfs.git"
hermes plugins enable surrealfs
hermes chat --toolsets surrealfs
```

The tools arrive prefixed — `surrealfs_ls`, `surrealfs_cat`,
`surrealfs_write_file` — because Hermes keeps one flat tool namespace that its own
`read_file`/`write_file` already sit in. The prefix also tells the model which
filesystem it is addressing: Hermes' file tools are the local disk, `surrealfs_*`
is the shared one in SurrealDB. The tool descriptions say so too, on every
surface — see `surrealfs/tools/docs/ls.md`.

The plugin opens its own connection per call, reading the same `SURREALDB_*`
environment variables as everything else here and defaulting to a local server;
set `SURREALFS_SEMANTIC=1` for hybrid search — which also means installing the
`embed` extra and scheduling the indexer, see
[Keeping the vectors current under Hermes](#keeping-the-vectors-current-under-hermes).
Handlers are registered
`is_async=True`, so Hermes awaits them on whichever loop it is using, and every
result — errors included — comes back as JSON.

It also bundles a skill, `surrealfs:notes`
(`surrealfs/integrations/hermes/skills/notes/SKILL.md`), which teaches the agent to
keep its notes and memories here and how to lay them out — `/home/<agent>/` for its
own working files, `/preferences/` for what it learns about the user,
`/projects/<name>/` per project. Hermes advertises plugin skills nowhere, though:
they are absent from the system prompt's `<available_skills>` index *and* from
`skills_list()`, and resolve only by exact qualified name. So the plugin also
registers a `pre_llm_call` hook that names the skill on the first turn of each
session. Hermes appends that to the user message and never persists it, so the
system prompt stays byte-stable and the prompt cache holds. Remove the hook and the
skill becomes unreachable.

To run from a checkout instead, symlink the plugin directory into Hermes' plugin
folder. The package still has to be importable by Hermes' interpreter:

```bash
ln -s "$PWD/surrealfs/integrations/hermes" ~/.hermes/plugins/surrealfs
```

### 5. Hermes memory provider

The tools above are memory the agent *chooses* to write. A second, independent
plugin closes the loop: Hermes hands it every completed turn, and asks it for
context before every turn.

```bash
ln -s "$PWD/surrealfs/integrations/hermes_memory" ~/.hermes/plugins/surrealfs-memory
# then in ~/.hermes/config.yaml:
#   memory:
#     provider: surrealfs-memory
```

The symlink is not an alternative to installing — Hermes discovers memory providers
by scanning `$HERMES_HOME/plugins/<name>/`, never the entry point, and the directory
name *is* the provider name. Each turn is filed as its own file in the agent's home,
under `/home/<agent>/memories/<profile>/`, skipping ones whose prompt carries no
signal and anything that is not a primary agent. Recall searches the whole filesystem
rather than just that folder, so the agent's own `/preferences/` and `/projects/`
notes come back too — they are the strongest signal in there — while other agents'
homes and other profiles' transcripts do not. One database can therefore serve
several agents and their users; `SURREALFS_AGENT_USER` names the home when the unix
account does not distinguish them. Set `SURREALFS_SEMANTIC=1`
to have recall match on meaning too, which needs the indexer running — see
[Keeping the vectors current under Hermes](#keeping-the-vectors-current-under-hermes).
See `surrealfs/integrations/hermes_memory/README.md`.

All four tool surfaces are generated from one registry in `surrealfs/tools/`, so
they cannot drift apart. Tool descriptions are markdown in
`surrealfs/tools/docs/` — edit them as prose; they are prompt text.

### Keeping the vectors current under Hermes

Only needed with `SURREALFS_SEMANTIC=1`, and needed by **both** Hermes surfaces
above: nothing embeds a file as it is written, so semantic search and semantic
recall see only what the indexer has already reached. Hermes has no way for a
plugin to declare a daemon of its own — the plugin API registers tools, hooks,
skills, commands and providers, and `terminal(background=true)` processes are
session-scoped — but its scheduler runs scripts without an agent, which covers
this without a systemd unit or a terminal you have to remember to leave open:

```bash
mkdir -p ~/.hermes/scripts
cat > ~/.hermes/scripts/surrealfs-embed.sh <<'EOF'
#!/usr/bin/env bash
# Cron sanitizes the subprocess env and OPENAI_API_KEY is on the blocklist, so
# the key has to come from a file. Same for the SURREALDB_* connection details.
set -a; . "$HOME/repos/surrealfs/.env"; set +a
exec uv run --project "$HOME/repos/surrealfs" --extra embed \
  python -m surrealfs.embed --once
EOF

hermes cron create 'every 5m' --no-agent --name surrealfs-embed \
  --script surrealfs-embed.sh
```

`--no-agent` skips the inference layer entirely — no tokens, no model, just the
script — so `--once` per tick replaces the polling loop `just embed` runs. The
script must live under `$HERMES_HOME/scripts/`; paths outside it are rejected.

An idle pass prints nothing, and empty stdout is a silent tick, so you hear from
this only when it has news: `embedded 4 file(s)` gets delivered, and a pass that
fails (no API key, database down) exits non-zero, which Hermes reports as an
alert rather than swallowing. `hermes cron runs` has the history.

Two things to know before relying on it. The scheduler ticks inside the gateway
daemon, so this runs only while that does — `hermes gateway install` if it is not
already a service — and vectors go stale, without breaking anything, whenever it
is down: search keeps working, on its full-text arm. And a five-minute period
means a file written now is searchable by meaning in up to five minutes; it is
immediately searchable by term. Lower the interval if that gap matters, at one
`SELECT` per tick that returns nothing when there is no work.

If the package is pip-installed rather than run from a checkout, install it into
Hermes' virtualenv with the extra (`"surrealfs[embed] @ git+..."`) and have the
script call that interpreter directly:

```bash
exec ~/.hermes/hermes-agent/venv/bin/python -m surrealfs.embed --once
```

It still needs `OPENAI_API_KEY` and the `SURREALDB_*` variables sourced from a
file of your own, for the same reason.

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
what makes it rank the invoicing note by *meaning* rather than by shared words. That
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

It needs `OPENAI_API_KEY` and an already-applied schema (`just schema`).

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
