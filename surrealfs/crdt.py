"""Yjs-backed CRDT support for collaborative multi-agent file editing."""

from __future__ import annotations

from typing import Any

try:
    import pycrdt

    _HAS_PYCRDT = True
except ImportError:
    pycrdt = None  # type: ignore[assignment]
    _HAS_PYCRDT = False


def is_crdt_available() -> bool:
    """Return True if pycrdt is installed and CRDT mode can be used."""
    return _HAS_PYCRDT


def _require_pycrdt() -> None:
    if not _HAS_PYCRDT:
        raise RuntimeError(
            "CRDT support requires pycrdt. Install surrealfs with the 'crdt' extra: "
            "pip install 'surrealfs[crdt]'"
        )


def init_doc(content: str = "") -> tuple[Any, bytes]:
    """Create a new CRDT document initialized with text content."""
    _require_pycrdt()
    doc = pycrdt.Doc()
    t = doc.get("text", type=pycrdt.Text)
    if content:
        t += content
    return doc, doc.get_update()


def load_doc(snapshot_state: bytes | None, updates: list[bytes]) -> Any:
    """Reconstruct a CRDT document from an optional snapshot and update stream."""
    _require_pycrdt()
    doc = pycrdt.Doc()
    if snapshot_state:
        doc.apply_update(snapshot_state)
    for update in updates:
        if update:
            doc.apply_update(update)
    return doc


def apply_edit(doc: Any, old: str, new: str) -> tuple[bytes, str]:
    """Apply a targeted find-and-replace edit to the document."""
    _require_pycrdt()
    t = doc.get("text", type=pycrdt.Text)
    current = str(t)
    pos = current.find(old)
    if pos < 0:
        raise ValueError(f"Target content {old!r} not found in document")
    sv = doc.get_state()
    del t[pos : pos + len(old)]
    if new:
        t[pos:pos] = new
    delta = doc.get_update(sv)
    return delta, str(t)


def apply_append(doc: Any, suffix: str) -> tuple[bytes, str]:
    """Append text to the end of the document."""
    _require_pycrdt()
    t = doc.get("text", type=pycrdt.Text)
    sv = doc.get_state()
    t += suffix
    delta = doc.get_update(sv)
    return delta, str(t)


def apply_replace(doc: Any, new_content: str) -> tuple[bytes, str]:
    """Replace the entire content of the document."""
    _require_pycrdt()
    t = doc.get("text", type=pycrdt.Text)
    sv = doc.get_state()
    del t[:]
    if new_content:
        t += new_content
    delta = doc.get_update(sv)
    return delta, str(t)


def materialize(doc: Any) -> str:
    """Render the document as plain string."""
    _require_pycrdt()
    t = doc.get("text", type=pycrdt.Text)
    return str(t)


def get_snapshot(doc: Any) -> bytes:
    """Return a full snapshot update of the document state."""
    _require_pycrdt()
    return doc.get_update()
