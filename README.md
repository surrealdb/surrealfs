<br>

<p align="center">
    <img width=120 src="https://raw.githubusercontent.com/surrealdb/icons/main/surreal.svg" />
</p>

<h1 align="center">SurrealDB file system</h1><br/>
<p align="center">A filesystem-based memory layer for agents, with hybrid search, backed by SurrealDB.</p>

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
plus the tools to hand it to a model: files and folders, full-text and semantic
search, all queryable with SurrealQL because it is just a table.

![The file browser](assets/surrealfs-browser.png)

*The file browser is a standalone tool that can be used to browse and edit files in a SurrealDB database.*

## Requirements

- Python 3.12+
- A SurrealDB 3.x server.

## Install

```bash
pip install surrealfs                    # core, and the Hermes plugin
pip install "surrealfs[pydantic-ai]"     # + the pydantic-ai toolset
pip install "surrealfs[browser]"         # + the file browser
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

fs = SurrealFs(db, user="alice")  # who you are; ROOT bypasses permissions
await fs.write_text("/notes/today.md", "# Today\n- ship the refactor")
await fs.edit("/notes/today.md", "ship", "shipped")
await fs.ls("/notes")
await fs.search_text("refactor")
```

You create and own the connection. SurrealFS never connects, signs in, or
selects a namespace.

`user` is required and has no default. Files carry a unix owner and mode, and
the database cannot tell SurrealFS who is asking (every credential is a system
credential), so the identity comes from you. See [Permissions](#permissions).

## The plain async API

`SurrealFs` returns structured `FileEntry` and `SearchHit` objects, not
pre-formatted strings. Build whatever integration you like on top.

|          |                                                               |
| -------- | ------------------------------------------------------------- |
| Read     | `read_text` `read_bytes` `tail` `ls` `glob` `stat` `exists`   |
| Write    | `write_text` `write_bytes` `edit` `touch` `mkdir`             |
| Organise | `mv` `cp` `rm` `chmod`                                        |
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
`surrealfs/tools/docs/`. Edit them as prose; they are prompt text. The memory
provider is the odd one out: it exposes no tools of its own, because the Hermes
plugin's already cover explicit reads and writes.

## The file browser

Point the agents at a shared SurrealDB (a Cloud instance, say)
and every person on the team can run the browser on their
own machine to see and edit the same filesystem the agents are writing to:

```bash
pip install "surrealfs[browser]"
surrealfs-browser                # http://127.0.0.1:7933
```

Tree on the left, file on the right, chat with the note-taking agent on the far
right. Text is editable and saves back to the table, markdown renders with a
source toggle, agent-authored HTML renders in a sandboxed iframe, images display,
and the search box is the same hybrid `fs.search` the agent's own tool calls.
Conversations are files too, under `/_sessions/`: open one from the tree and it
replays.

Credentials come from a `.env` in the working directory, or the environment
directly, which wins over the file:

```bash
SURREALDB_URL=wss://….surreal.cloud/rpc
SURREALDB_USER=…
SURREALDB_PASS=…
SURREALDB_NAMESPACE=…
SURREALDB_DATABASE=…
SURREALDB_AUTH_LEVEL=database    # root (default) | namespace | database
OPENAI_API_KEY=…                 # optional: makes search hybrid
ANTHROPIC_API_KEY=…              # optional: enables the chat panel
```

`SURREALDB_AUTH_LEVEL` picks which kind of user the credentials are, since the
server infers that from the signin payload: a database-scoped Cloud credential
needs `database`. All three are system users, which is why SurrealFS enforces
permissions itself rather than leaning on the database; `SURREALFS_USER` (or
`--user`) picks who the browser acts as, defaulting to root, which sees every
private home. Every variable has a command-line form too
(`surrealfs-browser --help`).

The server binds loopback. `--host 0.0.0.0` exposes it, and then anyone who can
reach the port has whatever access those credentials do. The page has no login
of its own.

## Semantic search

`fs.search` runs a full-text arm and a vector arm and fuses them by rank.
SurrealFS never calls an embedding model: you pass the embedder, so you pick
the provider, and without one search stays full-text only. An OpenAI embedder
and a daemon that keeps the vectors current ship in `surrealfs.embed`:

```bash
just embed              # poll every 5s; --once for a single pass
```

The queries are custom SurrealQL functions in the schema, not Python, so any
client gets the same ranked, permission-filtered retrieval:

```surql
SELECT path, rrf_score FROM fn::sfs_hybrid_search("how do I get paid", $qvec, 5, NONE, 'alice');
```

`fn::sfs_search_text` and `fn::sfs_search_semantic` are the single arms. The last
argument is who is asking: a system credential passes it, a record credential
overrides it. `SurrealFs` calls these too, so there is only one definition.

See [Semantic search](docs/semantic-search.md) for the ranking, `match="all"`,
the SurrealQL surface, and wiring it into the agent toolset.

## Permissions

Files carry a unix `owner` and `mode`, `/home/<user>` is private, everything
else is shared, and `chmod` moves things between the two. `SurrealFs` enforces
it. See [Permissions](docs/permissions.md) for the rules and the reasoning.

### Letting the database enforce it too (optional)

The above is a real boundary for an agent, since no tool exposes raw SurrealQL,
but not one against anyone holding the database credential. If several agents share
a database, opt in:

```bash
python -m surrealfs.schema --record-auth   # adds the `user` table + record access
python -m surrealfs.users add alice        # creates alice and /home/alice (0700)
```

```bash
SURREALDB_AUTH_LEVEL=record SURREALDB_USER=alice SURREALDB_PASS=…
```

Now SurrealDB itself refuses to return or modify anything in another user's
home, whatever query it is asked. Nothing else changes: the tools, the modes
and `chmod` behave the same, and `SurrealFs` picks its identity up from the
credential instead of the environment. Deployments that do not opt in are
unaffected: the permissions clause is inert for system credentials.

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

A folder is just a row with no `content`, no `file` bytes, and no `symlink`,
also computed, as `is_folder`. `hash` is maintained by an event, and there are
HNSW and BM25 indexes for the two search modes.

`owner`, `mode` and the computed `gate` are the permission model. See
[Permissions](#permissions) below and `docs/permissions.md`.

You can apply it without writing any code:

```bash
python -m surrealfs.schema                     # uses SURREALDB_* env vars
python -m surrealfs.schema --print             # dump the DDL, connect to nothing
```

## Development

```bash
just db        # start SurrealDB in one terminal
just schema    # apply the file table to it
just test      # run the suite (starts its own server if needed)
just check     # lint + test
just browser   # the file browser on :7933
just agent     # the note-taking chat agent on :7932
just loop "…"  # the framework-free Anthropic tool-use loop
```

Tests start a throwaway `surreal` server; set `SURREALFS_TEST_URL` to reuse one
you already have.
