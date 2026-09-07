"""The slice of the Spectron REST API the company brain needs.

Spectron (SurrealDB Agent Memory) is the layer that remembers what SurrealFS no
longer says. A file is current state; Spectron keeps every version that was ever
mirrored into it, extracts entities and relations out of the prose, and answers
questions across the lot.

Two calls, so this is a module of two functions rather than a client class:

    await mirror("/brain/acme/risks/okta-cert.md", text)   # after every write
    print(await recall("what is blocking the SOC2 audit"))  # for `brain_recall`

Configuration is the three variables Spectron Cloud hands you plus one of ours:

    SPECTRON_URL         https://srv1.spectron.aws-usw2.surreal.cloud
    SPECTRON_CONTEXT_ID  the context (tenant) to write into
    SPECTRON_API_KEY     bearer token
    SPECTRON_SCOPE       scope path to file under, default "brain"

With the context id or the key missing, `configured()` is False, `mirror` is a
no-op and `recall` says so. That is deliberate: the MCP server has to stay a
working SurrealFS server for anyone who has not signed up for Spectron.
"""

from __future__ import annotations

import asyncio
import json
import mimetypes
import os
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:  # `httpx` is imported inside the functions that use it, so
    import httpx  # importing this module works without the `claude` extra --
    # otherwise a missing extra is a module-level traceback that an MCP client
    # renders as a bare CONNECTION_CLOSED, with nothing to point at the cause.

__all__ = ["configured", "mirror", "recall", "scope"]

TIMEOUT = 30.0
RECALL_K = 8
# Long enough for a chunk to make its point, short enough that eight of them do
# not crowd out the files the skill has already read.
SNIPPET_CHARS = 500
DEFAULT_URL = "https://srv1.spectron.aws-usw2.surreal.cloud"
DEFAULT_SCOPE = "brain"


def configured() -> bool:
    """Whether there is a Spectron to talk to."""
    return bool(os.environ.get("SPECTRON_CONTEXT_ID") and _key())


def scope() -> str:
    """The scope path documents are filed under, and queries are lensed to.

    No leading slash: the API normalises a scope record to ``brain/`` but a
    ``lens`` of ``brain/`` matches nothing, while ``brain`` matches. Send the
    bare form to both.
    """
    return os.environ.get("SPECTRON_SCOPE", DEFAULT_SCOPE).strip("/") or DEFAULT_SCOPE


async def mirror(path: str, text: str) -> None:
    """Upload one SurrealFS file to Spectron as a document.

    Uploads are deduplicated by content hash, so re-mirroring a file nobody
    changed costs one request and creates nothing. A file that *did* change
    becomes a second document rather than replacing the first -- which is the
    point of having Spectron at all: the superseded version stays recallable
    after the filesystem has moved on.
    """
    if not configured() or not text.strip():
        # An empty file -- a bare `touch` -- is not knowledge, and uploading one
        # leaves a document in the way of every future query.
        return
    metadata = {
        "title": path,
        "scopes": [[scope()]],
        "labels": [f"path={path}", f"dir={path.rsplit('/', 1)[0] or '/'}"],
    }
    # Order is load-bearing. The server streams the multipart body and only
    # applies `metadata` if it arrives before `file`; the other way round the
    # upload still succeeds, silently titled after the filename and filed in the
    # root scope. And the file part must carry an explicit content type -- omit it
    # and the upload is a 500. Hence a list of parts, not a dict.
    import httpx

    parts = [
        ("metadata", (None, json.dumps(metadata), "application/json")),
        ("file", (path.rsplit("/", 1)[-1], text.encode(), _content_type(path))),
    ]
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.post(
            f"{_base()}/documents", files=parts, headers=_auth()
        )
        response.raise_for_status()


async def recall(query: str, k: int = RECALL_K) -> str:
    """Ask Spectron what it knows, rendered for a model to read."""
    if not configured():
        return (
            "Spectron is not configured: set SPECTRON_CONTEXT_ID and "
            "SPECTRON_API_KEY to recall anything beyond the filesystem."
        )
    import httpx

    body = {"query": query, "k": k, "mode": "hybrid", "lens": [[scope()]]}
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.post(f"{_base()}/query", json=body, headers=_auth())
        response.raise_for_status()
        hits = response.json().get("hits") or []
        titles = await _titles(client, hits)
    if not hits:
        return f"Nothing recalled from Spectron for {query!r}."
    return "Recalled from Spectron:\n" + "\n".join(_render(hit, titles) for hit in hits)


async def _titles(
    client: httpx.AsyncClient, hits: list[dict[str, Any]]
) -> dict[str, str]:
    """Document id -> title, for the documents these hits came from.

    A hit cites a document by id, and an id is no use to an agent that wants to
    open the file next. The title is the SurrealFS path `mirror` filed it under,
    so one lookup per distinct document -- a handful at most for one query --
    turns recall into something actionable.
    """
    ids = {
        str(hit.get("resource", {}).get("documentId"))
        for hit in hits
        if (hit.get("resource") or {}).get("documentId")
    }

    import httpx

    async def one(document_id: str) -> tuple[str, str]:
        try:
            response = await client.get(
                f"{_base()}/documents/{document_id}", headers=_auth()
            )
            response.raise_for_status()
            return document_id, str(response.json().get("title") or document_id)
        except httpx.HTTPError:
            # A document deleted between the query and now; the id still reads.
            return document_id, document_id

    return dict(await asyncio.gather(*(one(document_id) for document_id in ids)))


def _render(hit: dict[str, Any], titles: dict[str, str]) -> str:
    """One hit as a labelled line plus its text, indented.

    Same two-line shape the Hermes memory provider uses for a recalled file, so
    an agent reading both surfaces sees one format. The label names where the hit
    came from: a `chunk` or `section` cites the document it was cut from, an
    `entity` or `attribute` is something Spectron extracted that was never
    written down anywhere.
    """
    resource = hit.get("resource") or {}
    if resource.get("kind") == "entity":
        where = f"{resource.get('entityType', '?')}/{resource.get('name', '?')}"
    else:
        document_id = str(resource.get("documentId", "?"))
        where = titles.get(document_id, document_id)
    text = " ".join((hit.get("text") or "").split())[:SNIPPET_CHARS]
    return (
        f"{hit.get('source', 'hit')} · {where} · {hit.get('score', 0):.2f}\n    {text}"
    )


def _base() -> str:
    url = os.environ.get("SPECTRON_URL", DEFAULT_URL).rstrip("/")
    return f"{url}/api/v1/{os.environ['SPECTRON_CONTEXT_ID']}"


def _auth() -> dict[str, str]:
    return {"Authorization": f"Bearer {_key()}"}


def _key() -> str:
    return os.environ.get("SPECTRON_API_KEY", "")


def _content_type(path: str) -> str:
    guess, _ = mimetypes.guess_type(path)
    return guess or "text/plain"
