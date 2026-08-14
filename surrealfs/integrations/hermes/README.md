# SurrealFS as a Hermes toolset

The 14 `surrealfs_*` tools plus a bundled skill, for
[Hermes](https://hermes-agent.nousresearch.com). Registered through the
`hermes_agent.plugins` entry point, so installing the package is what makes the
plugin discoverable. Hermes runs from its own virtualenv, so install it there
rather than into your project:

```bash
uv pip install --python ~/.hermes/hermes-agent/venv/bin/python \
  "surrealfs @ git+https://github.com/surrealdb/surrealfs.git"
hermes plugins enable surrealfs
hermes chat --toolsets surrealfs
```

To run from a checkout instead, symlink the plugin directory into Hermes' plugin
folder. The package still has to be importable by Hermes' interpreter:

```bash
ln -s "$PWD/surrealfs/integrations/hermes" ~/.hermes/plugins/surrealfs
```

Nothing here imports Hermes: the plugin contract is a `register` function and a
manifest, so there is no dependency to install.

## The prefix

The tools arrive prefixed — `surrealfs_ls`, `surrealfs_cat`,
`surrealfs_write_file` — because Hermes keeps one flat tool namespace that its own
`read_file`/`write_file` already sit in. The prefix also tells the model which
filesystem it is addressing: Hermes' file tools are the local disk, `surrealfs_*`
is the shared one in SurrealDB. The tool descriptions say so too, on every
surface — see `surrealfs/tools/docs/ls.md`.

## Connection and search

The plugin opens its own connection per call, reading the same `SURREALDB_*`
environment variables as everything else here and defaulting to a local server.
Set `SURREALFS_SEMANTIC=1` for hybrid search — which also means installing the
`embed` extra and scheduling the indexer, see [Keeping the vectors current under
Hermes](https://github.com/surrealdb/surrealfs/blob/main/docs/hermes-indexer.md).

Handlers are registered `is_async=True`, so Hermes awaits them on whichever loop
it is using, and every result — errors included — comes back as JSON.

## The bundled skill

`surrealfs:notes` (`skills/notes/SKILL.md`) teaches the agent to keep its notes
and memories here and how to lay them out — `/home/<agent>/` for its own working
files, `/preferences/` for what it learns about the user, `/projects/<name>/` per
project.

Hermes advertises plugin skills nowhere, though: they are absent from the system
prompt's `<available_skills>` index *and* from `skills_list()`, and resolve only
by exact qualified name. So the plugin also registers a `pre_llm_call` hook that
names the skill on the first turn of each session. Hermes appends that to the user
message and never persists it, so the system prompt stays byte-stable and the
prompt cache holds. Remove the hook and the skill becomes unreachable.

## See also

[SurrealFS as Hermes memory](https://github.com/surrealdb/surrealfs/blob/main/surrealfs/integrations/hermes_memory/README.md)
— a separate plugin that files every turn and recalls context before each one.
Either works without the other, though they are better together: the provider's
recall surfaces what the agent wrote with these tools.
