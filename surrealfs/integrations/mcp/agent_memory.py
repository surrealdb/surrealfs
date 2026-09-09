"""The slice of the SurrealDB agent memory REST API the company brain needs.

Agent memory is the layer that remembers what SurrealFS no longer says. A file
is current state; agent memory keeps every version that was ever mirrored into
it, extracts entities and relations out of the prose, and answers questions
across the lot.

Two calls, so this is a module of two functions rather than a client class:

    await mirror("/brain/acme/risks/okta-cert.md", text)   # after every write
    print(await recall("what is blocking the SOC2 audit"))  # for `brain_recall`

Agent memory is optional, and this module is the whole of it. It needs the
`agent-memory` extra (`pip install 'surrealfs[mcp,agent-memory]'`, for httpx)
plus the three variables SurrealDB Cloud hands you and one of ours:

    AGENT_MEMORY_URL         https://srv1.spectron.aws-usw2.surreal.cloud
    AGENT_MEMORY_CONTEXT_ID  the context (tenant) to write into
    AGENT_MEMORY_API_KEY     bearer token
    AGENT_MEMORY_SCOPE       scope path to file under, default "brain"

With the context id or the key missing, `configured()` is False, `mirror` is a
no-op and `recall` says so, and the server does not advertise `brain_recall` at
all. That is deliberate: the MCP server has to be a complete SurrealFS server for
anyone who has not signed up for agent memory.
"""

from __future__ import annotations

import asyncio
import json
import mimetypes
import os
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:  # `httpx` is imported by `_httpx()`, inside the functions that
    import httpx  # use it, so importing this module works without the `agent-memory`
    # extra -- otherwise a missing extra is a module-level traceback that an MCP
    # client renders as a bare CONNECTION_CLOSED, with nothing to point at the
    # cause.

__all__ = ["DEFAULT_URL", "configured", "mirror", "recall", "scope"]

TIMEOUT = 30.0
RECALL_K = 8
# Long enough for a chunk to make its point, short enough that eight of them do
# not crowd out the files the skill has already read.
SNIPPET_CHARS = 500
DEFAULT_URL = "https://srv1.spectron.aws-usw2.surreal.cloud"
DEFAULT_SCOPE = "brain"


def configured() -> bool:
    """Whether there is an agent memory to talk to."""
    return bool(os.environ.get("AGENT_MEMORY_CONTEXT_ID") and _key())


def scope() -> str:
    """The scope path documents are filed under, and queries are lensed to.

    No leading slash: the API normalises a scope record to ``brain/`` but a
    ``lens`` of ``brain/`` matches nothing, while ``brain`` matches. Send the
    bare form to both.
    """
    return (
        os.environ.get("AGENT_MEMORY_SCOPE", DEFAULT_SCOPE).strip("/") or DEFAULT_SCOPE
    )


async def mirror(path: str, text: str) -> None:
    """Upload one SurrealFS file to agent memory as a document.

    Uploads are deduplicated by content hash, so re-mirroring a file nobody
    changed costs one request and creates nothing. A file that *did* change
    becomes a second document rather than replacing the first -- which is the
    point of having agent memory at all: the superseded version stays recallable
    after the filesystem has moved on.

    A scope has to exist before anything can be filed under it, so the first
    write into a fresh context registers `scope()` and posts again.
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
    parts = [
        ("metadata", (None, json.dumps(metadata), "application/json")),
        ("file", (path.rsplit("/", 1)[-1], text.encode(), _content_type(path))),
    ]
    async with _httpx().AsyncClient(timeout=TIMEOUT) as client:
        response = await client.post(
            f"{_base()}/documents", files=parts, headers=_auth()
        )
        if response.status_code == 400 and "scope" in response.text.lower():
            # A fresh context has only the root scope registered, and an upload
            # into an unregistered one is a 400 whose *body* is the only thing
            # that names the reason. Register and retry: the first write into a
            # new context is exactly when nobody is watching for a failure.
            # `parts` is bytes, not a stream, so it survives being posted twice.
            await client.post(
                f"{_base()}/scopes", json={"path": scope()}, headers=_auth()
            )
            response = await client.post(
                f"{_base()}/documents", files=parts, headers=_auth()
            )
        if response.is_error:
            # Not `raise_for_status`: its message carries the status and the URL
            # but not the body, and every message this API sends is in the body.
            raise RuntimeError(
                f"agent memory rejected {path}: "
                f"{response.status_code} {response.text[:200]}"
            )


async def recall(query: str, k: int = RECALL_K) -> str:
    """Ask agent memory what it knows, rendered for a model to read."""
    if not configured():
        return (
            "Agent memory is not configured: set AGENT_MEMORY_CONTEXT_ID and "
            "AGENT_MEMORY_API_KEY to recall anything beyond the filesystem."
        )
    body = {"query": query, "k": k, "mode": "hybrid", "lens": [[scope()]]}
    async with _httpx().AsyncClient(timeout=TIMEOUT) as client:
        response = await client.post(f"{_base()}/query", json=body, headers=_auth())
        response.raise_for_status()
        hits = response.json().get("hits") or []
        titles = await _titles(client, hits)
    if not hits:
        return f"Nothing recalled from agent memory for {query!r}."
    return "Recalled from agent memory:\n" + "\n".join(
        _render(hit, titles) for hit in hits
    )


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

    httpx = _httpx()

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
    `entity` or `attribute` is something agent memory extracted that was never
    written down anywhere.
    """
    resource = hit.get("resource") or {}
    if resource.get("kind") == "entity":
        where = f"{resource.get('entityType', '?')}/{resource.get('name', '?')}"
    else:
        document_id = str(resource.get("documentId", "?"))
        where = titles.get(document_id, document_id)
    text = " ".join((hit.get("text") or "").split())[:SNIPPET_CHARS]
    # `or 0` and not a `.get` default: a hit that carries `"score": null` has the
    # key, so the default never applies and `:.2f` raises -- which surfaces as
    # "could not reach agent memory" for a recall that in fact came back fine.
    score = float(hit.get("score") or 0)
    return f"{hit.get('source', 'hit')} · {where} · {score:.2f}\n    {text}"


def _httpx() -> Any:
    """The httpx module, or an error that names the extra it is missing from.

    Configured-but-not-installed has to be loud. Folding this into `configured()`
    would turn a typo in an install command into a memory layer that silently
    files nothing for someone who did sign up for one.
    """
    try:
        import httpx
    except ImportError as exc:  # pragma: no cover -- see tests/test_mcp.py
        raise RuntimeError(
            "AGENT_MEMORY_CONTEXT_ID and AGENT_MEMORY_API_KEY are set but httpx is not "
            "installed. Install: pip install 'surrealfs[mcp,agent-memory]'"
        ) from exc
    return httpx


def _base() -> str:
    url = os.environ.get("AGENT_MEMORY_URL", DEFAULT_URL).rstrip("/")
    return f"{url}/api/v1/{os.environ['AGENT_MEMORY_CONTEXT_ID']}"


def _auth() -> dict[str, str]:
    return {"Authorization": f"Bearer {_key()}"}


def _key() -> str:
    return os.environ.get("AGENT_MEMORY_API_KEY", "")


def _content_type(path: str) -> str:
    guess, _ = mimetypes.guess_type(path)
    return guess or "text/plain"
