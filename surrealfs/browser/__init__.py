"""A web file browser for the SurrealFS `file` table.

    pip install "surrealfs[browser]"
    surrealfs-browser        # then open http://127.0.0.1:7933

Run it locally against whatever database the agents are writing to -- the
`SURREALDB_*` variables in a `.env` in the working directory point it there. See
`surrealfs/browser/__main__.py` for the command line.

Tree on the left, file on the right, chat with the note-taking agent on the far
right: text is editable, markdown renders with a source toggle, HTML renders in
a sandboxed iframe, images display.

This module is the JSON API and nothing else. The page is a React SPA built from
`surrealfs/browser/ui/` into `surrealfs/browser/static/` -- run `just ui`.

Search is hybrid -- full-text and vector results fused by rank. The vector arm
needs OPENAI_API_KEY; without it search quietly falls back to full-text only.
The chat panel needs ANTHROPIC_API_KEY; without it the rest of the page works
and sending a message reports the missing key.
"""

from __future__ import annotations

import json
import os
import re
from collections.abc import AsyncIterator, Callable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from datetime import datetime
from pathlib import Path
from typing import Any

import uvicorn
from pydantic_ai import AgentRunResultEvent
from pydantic_ai.messages import (
    FunctionToolCallEvent,
    ModelMessagesTypeAdapter,
    PartDeltaEvent,
    PartStartEvent,
    TextPart,
    TextPartDelta,
    ToolCallPart,
    UserPromptPart,
)
from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import (
    FileResponse,
    JSONResponse,
    Response,
    StreamingResponse,
)
from starlette.routing import Mount, Route
from starlette.staticfiles import StaticFiles

from ..embed import INDEXER_VERSION, make_embedder
from ..errors import (
    AlreadyExists,
    DirectoryNotEmpty,
    InvalidPath,
    NotFound,
    PermissionDenied,
    SurrealFsError,
)
from ..fs import ROOT, SurrealFs
from ..integrations._connect import connect
from ..schema import apply_schema
from ..tools import ToolContext
from .agent import build_chat_agent

# The vite build output. Gitignored: `just ui` (or `just browser`) fills it.
STATIC = Path(__file__).with_name("static")
INDEX = STATIC / "index.html"

STATUS = {
    NotFound: 404,
    # `PermissionDenied` is an `OSError`, not a `ValueError`, so without this
    # `on_error`'s fallback made every denial a 500 -- reachable from the UI
    # since `--user` let the browser run as somebody other than root.
    PermissionDenied: 403,
    AlreadyExists: 409,
    DirectoryNotEmpty: 409,
}

# Conversations are files like every other note: the tree is the session list,
# and opening one loads it back into the chat panel.
SESSIONS = "/_sessions"

# chat() prefixes each user message with whatever was open at the time. Strip it
# back out so a reloaded transcript shows what the user actually typed.
OPEN_FILE_NOTE = re.compile(r"^\[The user has .*? open in the file browser\.\]\n\n")

Session = Callable[[Request], AbstractAsyncContextManager["SurrealFs"]]
"""Resolves the `SurrealFs` a request acts through, and releases it afterwards.

`one_user` is the trivial one. `browser/sso.py` returns a different identity per
request, which is the whole of the difference between the two browsers.
"""


def _session_path(first_message: str) -> str:
    """Where a new conversation lands.

    The slug only exists to make the tree readable -- the file is always found by
    path, never by name, so a collision within the same second is the only thing
    the timestamp has to rule out.
    """
    now = datetime.now()
    slug = re.sub(r"[^a-z0-9]+", "-", first_message.lower())[:40].strip("-")
    return f"{SESSIONS}/{now:%Y-%m-%d}/{now:%H%M%S}-{slug or 'chat'}.json"


def _transcript(messages: list[Any]) -> list[dict[str, Any]]:
    """Model messages as the bubbles the chat panel draws.

    Mirrors a live turn: one `you` bubble, then one `agent` bubble accumulating
    text and tool names across the whole run -- a run that stops to call tools
    resumes in a new `ModelResponse`, and all of it belongs to the same bubble.
    """
    out: list[dict[str, Any]] = []
    for message in messages:
        for part in message.parts:
            # Content is a list for a multimodal prompt; this UI only sends text.
            if isinstance(part, UserPromptPart) and isinstance(part.content, str):
                out.append({"who": "you", "text": OPEN_FILE_NOTE.sub("", part.content)})
                out.append({"who": "agent", "text": "", "tools": []})
            elif not out:
                continue  # instructions, or a response with no prompt before it
            elif isinstance(part, TextPart):
                out[-1]["text"] = f"{out[-1]['text']}\n\n{part.content}".strip()
            elif isinstance(part, ToolCallPart):
                out[-1]["tools"].append(part.tool_name)
    # Text, not HTML: the page renders the agent's markdown itself, so there is
    # one markdown implementation and no server HTML to sanitise on the way in.
    return [b for b in out if b["text"] or b.get("tools")]


def _line(payload: dict[str, Any]) -> bytes:
    """One NDJSON frame for the chat stream."""
    return (json.dumps(payload) + "\n").encode()


def _stream_event(event: Any) -> dict[str, Any] | None:
    """What the page needs from one agent run event, or None to skip it."""
    if isinstance(event, PartDeltaEvent) and isinstance(event.delta, TextPartDelta):
        return {"delta": event.delta.content_delta}
    if isinstance(event, PartStartEvent) and isinstance(event.part, TextPart):
        # The opening text of a part arrives here, not as a delta -- skip this
        # and every sentence loses its first word or two. A run that stops to
        # call tools resumes in a *new* part, so keep the paragraph break.
        return {"delta": f"\n\n{event.part.content}"}
    if isinstance(event, FunctionToolCallEvent):
        return {"tool": event.part.tool_name}
    return None


def _param(request: Request, name: str) -> str:
    value = request.query_params.get(name)
    if not value:
        raise InvalidPath(f"missing '{name}' parameter")
    return value


def _entry_json(entry: Any) -> dict[str, Any]:
    return {
        "path": entry.path,
        "filename": entry.filename,
        "is_folder": entry.is_folder,
        "content_type": entry.content_type,
        "size": entry.size,
        "updated_at": str(entry.updated_at or ""),
    }


def one_user(fs: SurrealFs) -> Session:
    """A session factory that hands every request the same `SurrealFs`.

    What the plain `surrealfs-browser` runs on: one credential, one identity,
    no login. `browser/sso.py` passes a factory that opens a database session
    per request instead, authenticated as whoever signed in.
    """

    @asynccontextmanager
    async def session(request: Request) -> AsyncIterator[SurrealFs]:
        yield fs

    return session


class Browser:
    """Route handlers over a `SurrealFs` resolved per request.

    The indirection is what lets one set of handlers serve both browsers. It is
    a factory rather than an argument because an authenticated request has a
    database session to close afterwards, which only a context manager can do.
    """

    def __init__(
        self,
        session: Session,
        embed: Any = None,
        agent: Any = None,
        indexer: SurrealFs | None = None,
    ) -> None:
        self.session = session
        self.embed = embed
        self.agent = agent
        # Re-embedding reads every text file, so it is root-only by design and
        # cannot run as the person who happens to have made the request. Held
        # apart from `session` for that reason: it is never a request's handle.
        self.indexer = indexer

    async def page(self, request: Request) -> Response:
        return FileResponse(INDEX)

    async def tree(self, request: Request) -> Response:
        # ponytail: whole tree in one request; paginate if a DB ever holds
        # thousands of files. ls(recursive=True) is a BFS over the child index.
        async with self.session(request) as fs:
            entries = await fs.ls("/", recursive=True)
        return JSONResponse([_entry_json(e) for e in entries])

    async def raw(self, request: Request) -> Response:
        path = _param(request, "path")
        async with self.session(request) as fs:
            entry = await fs.stat(path)
            data = await fs.read_bytes(path)
        # This serves agent-authored HTML from our own origin. nosniff stops a
        # text/plain file being re-read as script; `CSP: sandbox` drops the
        # response into an opaque origin so it cannot touch the browser UI even
        # if opened directly rather than through the iframe.
        return Response(
            data,
            media_type=entry.content_type,
            headers={
                "X-Content-Type-Options": "nosniff",
                "Content-Security-Policy": "sandbox",
            },
        )

    async def save(self, request: Request) -> Response:
        body = await request.json()
        path = body["path"]
        async with self.session(request) as fs:
            entry = await fs.stat(path)
            saved = await fs.write_text(
                path, body["content"], content_type=entry.content_type
            )
        await self._reindex()
        return JSONResponse(_entry_json(saved))

    async def create(self, request: Request) -> Response:
        body = await request.json()
        path = body["path"]
        async with self.session(request) as fs:
            if await fs.exists(path):
                raise AlreadyExists(f"Already exists: {path}")
            if body.get("folder"):
                entry = await fs.mkdir(path, parents=True)
            else:
                # Not touch(): it hardcodes text/plain, so a new .md would open
                # in the plain editor with no preview toggle. write_text sniffs
                # the extension instead.
                entry = await fs.write_text(path, "")
        return JSONResponse(_entry_json(entry))

    async def move(self, request: Request) -> Response:
        body = await request.json()
        async with self.session(request) as fs:
            entry = await fs.mv(body["src"], body["dst"])
        return JSONResponse(_entry_json(entry))

    async def delete(self, request: Request) -> Response:
        path = _param(request, "path")
        recursive = request.query_params.get("recursive") == "1"
        async with self.session(request) as fs:
            removed = await fs.rm(path, recursive=recursive)
        await self._reindex()
        return JSONResponse({"removed": removed})

    async def search(self, request: Request) -> Response:
        query = request.query_params.get("q", "").strip()
        if not query:
            return JSONResponse({"hybrid": self.embed is not None, "results": []})

        # The fusion lives in the library, so the agent's `search` tool and this
        # box rank identically.
        async with self.session(request) as fs:
            hits = await fs.search(query, vector=await self._embed(query), limit=20)
        results = [{**_entry_json(hit.entry), "snippet": hit.snippet} for hit in hits]
        return JSONResponse({"hybrid": self.embed is not None, "results": results})

    async def chat(self, request: Request) -> Response:
        """Stream one agent turn as NDJSON: tool names, text deltas, then done.

        Not SSE: the client hand-rolls the reader either way, so `data:` framing
        buys nothing, and `EventSource` is GET-only and reconnects on its own --
        which would silently re-run, and re-bill, an agent turn.
        """
        if self.agent is None:
            return JSONResponse(
                {"error": "chat needs ANTHROPIC_API_KEY"}, status_code=400
            )
        body = await request.json()
        message = body["message"]
        # The question is nearly always about what is on screen. It rides on the
        # user message rather than the instructions so it is stored with the
        # conversation: each turn then records what was open at the time, instead
        # of the whole transcript being retconned to whatever is open now.
        if path := body.get("path"):
            message = f"[The user has {path} open in the file browser.]\n\n{message}"

        # The conversation lives in the file, not in this process: an absent
        # `session` starts a new one, and the page adopts the path we return.
        transcript = body.get("session")

        async def stream() -> AsyncIterator[bytes]:
            messages: list[Any] = []
            try:
                # Inside the generator, not around it: a StreamingResponse body
                # runs *after* the handler returns, so a database session opened
                # out here would already be closed by the time the agent ran.
                async with self.session(request) as fs:
                    # ponytail: the whole history goes back to the model every
                    # turn. Trim the middle if a conversation ever outgrows the
                    # context window.
                    history = (
                        ModelMessagesTypeAdapter.validate_json(
                            await fs.read_bytes(transcript)
                        )
                        if transcript
                        else []
                    )
                    async with self.agent.run_stream_events(
                        message,
                        # Read `self.embed` per turn: it is set to None if the
                        # provider ever fails, and the tool must follow. `fs` is
                        # this request's, so the agent's tools act as whoever
                        # asked -- and are refused what they may not touch.
                        deps=ToolContext(fs=fs, embed=self.embed),
                        message_history=history,
                    ) as events:
                        async for event in events:
                            if isinstance(event, AgentRunResultEvent):
                                messages = event.result.all_messages()
                            elif (payload := _stream_event(event)) is not None:
                                yield _line(payload)
                    # write_bytes, not write_text: `search_text` and
                    # `reindex_embeddings` both filter on the row's `content`,
                    # which this leaves unset -- so transcripts cost no
                    # embeddings and stay out of note search, while /raw still
                    # serves them to the viewer. `messages` is empty only if the
                    # run ended without a result event; writing that would blank
                    # an existing transcript.
                    stored_at = transcript or _session_path(body["message"])
                    if messages:
                        await fs.write_bytes(
                            stored_at,
                            ModelMessagesTypeAdapter.dump_json(messages),
                            content_type="application/json",
                        )
                # The agent writes files, so the search index is now stale.
                await self._reindex()
                yield _line(
                    # Nothing stored means nothing to adopt: leave the page on
                    # the session it already had. The page re-renders what it
                    # accumulated from the deltas as markdown; the reply does not
                    # come back a second time.
                    {"done": True, "session": stored_at if messages else transcript}
                )
            except Exception as exc:  # noqa: BLE001 -- any failure, same answer
                # The 200 is already on the wire, so failures have to ride the
                # stream: on_error never sees them.
                yield _line({"error": str(exc)})

        return StreamingResponse(stream(), media_type="application/x-ndjson")

    async def transcript(self, request: Request) -> Response:
        """One stored conversation, as the bubbles the chat panel draws."""
        async with self.session(request) as fs:
            raw = await fs.read_bytes(_param(request, "path"))
        return JSONResponse(_transcript(ModelMessagesTypeAdapter.validate_json(raw)))

    async def _embed(self, text: str) -> Any:
        """Embed one string, or None if there is no working embedder."""
        if self.embed is None:
            return None
        try:
            return await self.embed(text)
        except Exception as exc:  # noqa: BLE001 -- any provider failure, same answer
            self._disable(exc)
            return None

    async def _reindex(self) -> None:
        """Re-embed what just changed. Incremental: unchanged rows are skipped.

        Skipped entirely unless this browser is root: `reindex_embeddings` reads
        every text file to embed it, which is not a thing `--user alice` may do.
        Semantic *search* still works for them against what root has indexed.
        """
        if self.embed is None or self.indexer is None or not self.indexer.is_root:
            return
        try:
            await self.indexer.reindex_embeddings(self.embed, version=INDEXER_VERSION)
        except Exception as exc:  # noqa: BLE001
            # A dead embedding provider must not fail the write that just
            # succeeded, nor take the server down.
            self._disable(exc)

    def _disable(self, exc: Exception) -> None:
        self.embed = None
        print(f"embedding failed, falling back to full-text search only: {exc}")


def on_error(request: Request, exc: Exception) -> Response:
    # Starlette walks the MRO when looking up a handler, so registering the base
    # class catches every SurrealFs error subclass.
    if isinstance(exc, KeyError):  # a request body missing a required field
        return JSONResponse({"error": f"missing field: {exc}"}, status_code=400)
    # isinstance, not `STATUS[type(exc)]`: an exact-type lookup gives a subclass
    # of a mapped error the 500 fallback instead of its parent's status, which
    # is how `sso.Unauthenticated` -- a `PermissionDenied` -- first reported as a
    # server fault rather than a denial. The hierarchy in `errors.py` is flat, so
    # at most one entry ever matches.
    status = next(
        (code for kind, code in STATUS.items() if isinstance(exc, kind)),
        400 if isinstance(exc, ValueError) else 500,
    )
    return JSONResponse({"error": str(exc)}, status_code=status)


def assemble(
    b: Browser,
    routes: list[Any] | None = None,
    middleware: list[Any] | None = None,
) -> Starlette:
    """The Starlette app around a `Browser`, with one route table.

    Both browsers come through here so the page cannot be served a different API
    depending on which one is running: `sso.py` adds routes and middleware, it
    does not restate these.
    """
    app = Starlette(
        routes=[
            *(routes or []),
            Route("/", b.page),
            Route("/raw", b.raw),
            Route("/api/tree", b.tree),
            Route("/api/search", b.search),
            Route("/api/session", b.transcript),
            Route("/api/chat", b.chat, methods=["POST"]),
            Route("/api/move", b.move, methods=["POST"]),
            Route("/api/file", b.save, methods=["PUT"]),
            Route("/api/file", b.create, methods=["POST"]),
            Route("/api/file", b.delete, methods=["DELETE"]),
            # Everything vite emits alongside index.html: hashed js, css, fonts.
            # check_dir=False: `build_app` must work with no build output,
            # for tests and for the "run `just ui`" message in `serve`.
            Mount(
                "/assets",
                StaticFiles(directory=STATIC / "assets", check_dir=False),
            ),
        ],
        middleware=middleware or [],
        exception_handlers={SurrealFsError: on_error, KeyError: on_error},
    )
    app.state.browser = b
    return app


def build_app(fs: SurrealFs, embed: Any = None, agent: Any = None) -> Starlette:
    """The single-identity browser: every request acts as `fs`."""
    return assemble(Browser(one_user(fs), embed, agent, indexer=fs))


async def serve(host: str = "127.0.0.1", port: int = 7933) -> None:
    """Connect and serve on a single event loop.

    A SurrealDB WebSocket belongs to the loop it was opened on, so the connection
    has to be opened on the loop that will serve requests. `uvicorn.run()` makes
    its own loop; drive `uvicorn.Server` ourselves instead and everything stays
    on one.
    """
    # Gitignored build output, so "I just cloned this" is the common case. A
    # 404 on every asset is a bad way to learn that; say it once, up front.
    if not INDEX.exists():
        raise SystemExit(
            f"the browser UI is not built ({INDEX} is missing) -- run `just ui`"
        )

    db = await connect()
    try:
        # Usually a no-op: on a shared database the table is already defined, and
        # the credential may hold no right to define one. Either way an
        # un-applied schema is no reason to refuse to show the files.
        await apply_schema(db)
    except Exception as exc:  # noqa: BLE001 -- any DDL failure, same answer
        print(f"could not apply the schema (continuing): {exc}")
    fs = SurrealFs(db, user=os.environ.get("SURREALFS_USER") or ROOT)

    embed = make_embedder() if os.environ.get("OPENAI_API_KEY") else None
    if embed is None:
        print("OPENAI_API_KEY unset -- search will be full-text only.")

    # This process has an embedder, so hand the agent the hybrid `search` too.
    agent = (
        build_chat_agent(semantic=embed is not None)
        if os.environ.get("ANTHROPIC_API_KEY")
        else None
    )
    if agent is None:
        print("ANTHROPIC_API_KEY unset -- the chat panel will refuse to send.")

    app = build_app(fs, embed, agent)
    # Embed anything the agent wrote while this was not running. Goes through
    # the app so that a rejected key downgrades search instead of refusing to
    # start -- an unusable embedding provider is not a reason to withhold the
    # file browser.
    await app.state.browser._reindex()
    print(f"Browsing SurrealFS on http://{host}:{port}")
    try:
        await uvicorn.Server(uvicorn.Config(app, host=host, port=port)).serve()
    finally:
        await db.close()
