"""The one bit of non-obvious logic in the file browser: rank fusion.

Needs no database. Skipped when the demo extra is missing, since importing the
example drags in starlette, uvicorn and the agent modules.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "examples"))

pytest.importorskip("uvicorn", reason="needs the demo extra")
pytest.importorskip("markdown_it", reason="needs the demo extra")

from file_browser import rrf  # noqa: E402


def test_agreement_beats_a_single_top_hit():
    fused = rrf(["text-only", "both", "x"], ["vector-only", "both", "y"])
    assert fused[0] == "both"
    assert set(fused) == {"text-only", "vector-only", "both", "x", "y"}


def test_degrades_to_the_single_arm():
    assert rrf(["a", "b"], []) == ["a", "b"]
    assert rrf() == []
