# Hermes memory provider — what's wrong

Review of `surrealfs/integrations/hermes_memory/` against
[Building a Memory Provider Plugin](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/developer-guide/memory-provider-plugin.md)
and the sources it describes: `agent/memory_provider.py`, `agent/memory_manager.py`,
`plugins/memory/__init__.py`, `hermes_cli/memory_setup.py`, `hermes_cli/plugins.py`.

Open defects. Companion files: [missing](hermes-memory-review-missing.md),
[improvements](hermes-memory-review-improvements.md), and
[decisions](hermes-memory-decisions.md) for what has already been settled.

## 1. Bootstrap is manual and breaks on every venv rebuild

`plugin.yaml` can declare `pip_dependencies: [surrealfs]`. `hermes memory setup`
installs those into Hermes' venv (`hermes_cli/memory_setup.py:_install_dependencies`),
and `hermes update` re-installs them with `force=True` after a venv sync strips or
downgrades them (#53272, #70636) — which is precisely the "surrealfs also has to be
importable by Hermes' own interpreter" step the README asks the user to do by hand.

It cannot bootstrap as written: `hermes_memory/__init__.py` imports `surrealfs` at
module top, and `_get_available_providers()` drops any provider whose module fails
to import, so the wizard never lists it on a machine that doesn't already have the
package. Move the three `surrealfs` imports into the methods that use them and the
manual `uv pip install --python ~/.hermes/hermes-agent/venv/bin/python` step goes
away.

## 2. `is_available()` is unconditionally `True`

Correct with respect to "no network calls", but the outcome is `hermes memory`
reporting the provider healthy while every turn fails into a `logger.warning`
nobody reads (`memory_manager.py` catches everything from `sync_turn` and
`prefetch`). At minimum check `importlib.util.find_spec("surrealdb")`. The
reachability probe that `is_available()` is not allowed to perform belongs in a
`cli.py` subcommand — see [missing #2](hermes-memory-review-missing.md).
