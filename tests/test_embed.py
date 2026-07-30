"""The indexer loop, without a database or an API key.

Staleness itself is `test_search.py::test_reindex_embeddings_only_touches_stale_rows`;
what is tested here is only the daemon wrapper around it.
"""

from __future__ import annotations

import pytest

from surrealfs.embed import INDEXER_VERSION, index_forever


class FakeFs:
    """Records each pass, and raises whatever the `outcomes` script says to."""

    def __init__(self, *outcomes: object) -> None:
        self.outcomes = list(outcomes)
        self.calls: list[dict] = []

    async def reindex_embeddings(self, embed, *, version, batch):
        self.calls.append({"version": version, "batch": batch})
        outcome = self.outcomes.pop(0)
        if isinstance(outcome, BaseException):
            raise outcome
        return outcome


async def _embed(text: str) -> list[float]:  # never called; FakeFs stands in
    raise AssertionError("the loop must not embed anything itself")


async def test_once_makes_exactly_one_pass():
    fs = FakeFs(3)
    await index_forever(fs, _embed, once=True, batch=8)
    assert fs.calls == [{"version": INDEXER_VERSION, "batch": 8}]


async def test_a_failed_single_pass_propagates():
    # Unlike the daemon, `--once` has an exit code to be honest in.
    fs = FakeFs(RuntimeError("bad key"))
    with pytest.raises(RuntimeError, match="bad key"):
        await index_forever(fs, _embed, once=True)


async def test_a_failed_pass_does_not_stop_the_daemon():
    # Pass 1 blows up, pass 2 succeeds -- proving the loop survived it -- then
    # KeyboardInterrupt stands in for Ctrl-C to break out.
    fs = FakeFs(RuntimeError("connection reset"), 0, KeyboardInterrupt())
    with pytest.raises(KeyboardInterrupt):
        await index_forever(fs, _embed, interval=0)
    assert len(fs.calls) == 3
