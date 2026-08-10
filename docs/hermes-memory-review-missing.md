# Hermes memory provider — what's missing

Contract surfaces `surrealfs/integrations/hermes_memory/` does not implement, or
declares incompletely. See also [improvements](hermes-memory-review-improvements.md)
and [decisions](hermes-memory-decisions.md).

Not everything here needs implementing. `handle_tool_call` (no tools),
`save_config` (env-var-only), `shutdown` (no long-lived connection) and
`backup_paths` (nothing on disk) are all correctly left at their defaults.

## 1. `on_session_switch`

Fires on `/resume`, `/branch`, `/reset`, `/new`, the gateway equivalents, and
context compression — every path that reassigns `session_id` without tearing the
provider down. The ABC asks providers caching per-session state to update it here.

`_session_id` is the fallback used when `sync_turn` arrives with `session_id=""`,
and it goes stale. It is now the only per-session state left, so this is a one-line
reassignment — see [decisions](hermes-memory-decisions.md).

## 2. Two hooks this backend is unusually well suited to

- **`on_memory_write(action, target, content, metadata)`** — mirrors built-in
  `MEMORY.md` / `USER.md` writes into the backend. Without it, Hermes' own memory
  and SurrealFS diverge permanently, and recall never sees what the built-in
  memory tool wrote.
- **`on_pre_compress(messages)`** — called with the messages compression is about
  to discard, returns text folded into the compression summary prompt. A
  filesystem-shaped memory is the ideal sink for this: write the doomed messages
  verbatim rather than summarising them, which is all a cloud provider can do.

Both write turns, so both should go through `path_for` and inherit the profile
subtree.

## 3. `queue_prefetch` not implemented

The ABC pairs it with `prefetch`: queue the recall after each turn, serve the
cached result on the next one. The provider does the whole retrieval inline in
`prefetch` instead. See [improvements #1](hermes-memory-review-improvements.md) for
why that costs more than the `ponytail:` note assumes.

## 4. Config schema gaps

`get_config_schema()` covers url, namespace, database, password, memory_dir.
Missing:

- **`SURREALDB_USER`** — read by `_connect.connected()` and documented in the
  README, but setup never prompts for it and `hermes memory`'s env checklist
  (`memory_setup.py:536`) never shows it.
- **`SURREALFS_SEMANTIC`** — same. Arguably belongs in the README-only tier the doc
  recommends for optional settings, but then `memory_dir` is on the wrong side of
  that line too. Pick one rule.
- **`required: True` on `password`**, which currently has neither a default nor a
  required flag.

The env-var-only design itself is valid: `memory_setup.py:399` writes non-secret
fields that carry an `env_var` to `.env` as well as `config.yaml`, so a no-op
`save_config()` loses nothing. `tests/test_memory_provider.py` already pins that
every field names an env var.

## 5. No conformance test against the real ABC

Because `_Base = object` outside Hermes, the suite never checks a single signature
against upstream; a rename or a new required method would pass green. An opt-in test
that imports `agent.memory_provider` when it is importable closes that.

The rest of the suite is well aimed — the `MemoryProvider`-string guard on the tool
plugin especially, and resume, profile isolation and `agent_context` are covered
now.
