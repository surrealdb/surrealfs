"""A web file browser for the SurrealFS `file` table.

    pip install "surrealfs[browser]"
    surrealfs-browser        # then open http://127.0.0.1:7933

Run it locally against whatever database the agents are writing to -- the
`SURREALDB_*` variables in a `.env` in the working directory point it there. See
`surrealfs/browser/__main__.py` for the command line.

Tree on the left, file on the right, chat with the note-taking agent on the far
right: text is editable, markdown renders as HTML with a source toggle, HTML
renders in a sandboxed iframe, images display.

Search is hybrid -- full-text and vector results fused by rank. The vector arm
needs OPENAI_API_KEY; without it search quietly falls back to full-text only.
The chat panel needs ANTHROPIC_API_KEY; without it the rest of the page works
and sending a message reports the missing key.
"""

from __future__ import annotations

import json
import os
import re
from collections.abc import AsyncIterator
from datetime import datetime
from pathlib import Path
from typing import Any

import uvicorn
from markdown_it import MarkdownIt
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
from starlette.routing import Route

from ..embed import INDEXER_VERSION, make_embedder
from ..errors import (
    AlreadyExists,
    DirectoryNotEmpty,
    InvalidPath,
    NotFound,
    SurrealFsError,
)
from ..fs import ROOT, SurrealFs
from ..integrations._connect import connect
from ..schema import apply_schema
from ..tools import ToolContext
from .agent import build_chat_agent

PAGE = Path(__file__).with_name("page.html")

# html=False is NOT the default -- the commonmark preset sets html=True, because
# the CommonMark spec allows raw HTML. The rendered fragment is injected into the
# page with innerHTML, so leaving it on is stored XSS: `<script>` would not fire
# from innerHTML but `<img onerror=…>` very much would. Off means raw HTML in a
# markdown file is escaped and shown as text. Do not turn it back on.
MD = MarkdownIt("commonmark", {"html": False}).enable("table")

STATUS = {
    NotFound: 404,
    AlreadyExists: 409,
    DirectoryNotEmpty: 409,
}

# Conversations are files like every other note: the tree is the session list,
# and opening one loads it back into the chat panel.
SESSIONS = "/_sessions"

# chat() prefixes each user message with whatever was open at the time. Strip it
# back out so a reloaded transcript shows what the user actually typed.
OPEN_FILE_NOTE = re.compile(r"^\[The user has .*? open in the file browser\.\]\n\n")


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
    # The reply is rendered markdown, same as at the end of a live turn; the
    # user's own text is not markdown and reaches the page as textContent.
    return [
        {**b, "html": MD.render(b["text"]) if b["who"] == "agent" else ""}
        for b in out
        if b["text"] or b.get("tools")
    ]


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


class Browser:
    """Route handlers over one `SurrealFs`."""

    def __init__(self, fs: SurrealFs, embed: Any = None, agent: Any = None) -> None:
        self.fs = fs
        self.embed = embed
        self.agent = agent

    async def page(self, request: Request) -> Response:
        return FileResponse(PAGE)

    async def tree(self, request: Request) -> Response:
        # ponytail: whole tree in one request; paginate if a DB ever holds
        # thousands of files. ls(recursive=True) is a BFS over the child index.
        entries = await self.fs.ls("/", recursive=True)
        return JSONResponse([_entry_json(e) for e in entries])

    async def raw(self, request: Request) -> Response:
        path = _param(request, "path")
        entry = await self.fs.stat(path)
        data = await self.fs.read_bytes(path)
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

    async def markdown(self, request: Request) -> Response:
        html = MD.render(await self.fs.read_text(_param(request, "path")))
        return Response(html, media_type="text/html; charset=utf-8")

    async def save(self, request: Request) -> Response:
        body = await request.json()
        path = body["path"]
        entry = await self.fs.stat(path)
        saved = await self.fs.write_text(
            path, body["content"], content_type=entry.content_type
        )
        await self._reindex()
        return JSONResponse(_entry_json(saved))

    async def create(self, request: Request) -> Response:
        body = await request.json()
        path = body["path"]
        if await self.fs.exists(path):
            raise AlreadyExists(f"Already exists: {path}")
        if body.get("folder"):
            entry = await self.fs.mkdir(path, parents=True)
        else:
            # Not touch(): it hardcodes text/plain, so a new .md would open in
            # the plain editor with no preview toggle. write_text sniffs the
            # extension instead.
            entry = await self.fs.write_text(path, "")
        return JSONResponse(_entry_json(entry))

    async def move(self, request: Request) -> Response:
        body = await request.json()
        entry = await self.fs.mv(body["src"], body["dst"])
        return JSONResponse(_entry_json(entry))

    async def delete(self, request: Request) -> Response:
        path = _param(request, "path")
        recursive = request.query_params.get("recursive") == "1"
        removed = await self.fs.rm(path, recursive=recursive)
        await self._reindex()
        return JSONResponse({"removed": removed})

    async def search(self, request: Request) -> Response:
        query = request.query_params.get("q", "").strip()
        if not query:
            return JSONResponse({"hybrid": self.embed is not None, "results": []})

        # The fusion lives in the library, so the agent's `search` tool and this
        # box rank identically.
        hits = await self.fs.search(query, vector=await self._embed(query), limit=20)
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
        # ponytail: the whole history goes back to the model every turn. Trim the
        # middle of it if a conversation ever outgrows the context window.
        session = body.get("session")
        history = (
            ModelMessagesTypeAdapter.validate_json(await self.fs.read_bytes(session))
            if session
            else []
        )

        async def stream() -> AsyncIterator[bytes]:
            reply = ""
            messages: list[Any] = []
            try:
                async with self.agent.run_stream_events(
                    message,
                    # Read `self.embed` per turn: it is set to None if the
                    # provider ever fails, and the tool must follow.
                    deps=ToolContext(fs=self.fs, embed=self.embed),
                    message_history=history,
                ) as events:
                    async for event in events:
                        if isinstance(event, AgentRunResultEvent):
                            messages = event.result.all_messages()
                        elif (payload := _stream_event(event)) is not None:
                            # Accumulate what was streamed rather than taking
                            # `result.output`: that holds only the final text
                            # part, so anything said before a tool call would
                            # vanish when the rendered reply replaced it.
                            reply = (reply + payload.get("delta", "")).lstrip()
                            yield _line(payload)
                # write_bytes, not write_text: `search_text` and
                # `reindex_embeddings` both filter on the row's `content`, which
                # this leaves unset -- so transcripts cost no embeddings and stay
                # out of note search, while /raw still serves them to the viewer.
                # `messages` is empty only if the run ended without a result
                # event; writing that would blank an existing transcript.
                stored_at = session or _session_path(body["message"])
                if messages:
                    await self.fs.write_bytes(
                        stored_at,
                        ModelMessagesTypeAdapter.dump_json(messages),
                        content_type="application/json",
                    )
                # The agent writes files, so the search index is now stale.
                await self._reindex()
                yield _line(
                    # Nothing stored means nothing to adopt: leave the page on
                    # the session it already had.
                    {
                        "done": True,
                        "html": MD.render(reply),
                        "session": stored_at if messages else session,
                    }
                )
            except Exception as exc:  # noqa: BLE001 -- any failure, same answer
                # The 200 is already on the wire, so failures have to ride the
                # stream: on_error never sees them.
                yield _line({"error": str(exc)})

        return StreamingResponse(stream(), media_type="application/x-ndjson")

    async def session(self, request: Request) -> Response:
        """One stored conversation, as the bubbles the chat panel draws."""
        raw = await self.fs.read_bytes(_param(request, "path"))
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
        if self.embed is None or not self.fs.is_root:
            return
        try:
            await self.fs.reindex_embeddings(self.embed, version=INDEXER_VERSION)
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
    status = STATUS.get(type(exc), 400 if isinstance(exc, ValueError) else 500)
    return JSONResponse({"error": str(exc)}, status_code=status)


def build_app(fs: SurrealFs, embed: Any = None, agent: Any = None) -> Starlette:
    b = Browser(fs, embed, agent)
    app = Starlette(
        routes=[
            Route("/", b.page),
            Route("/raw", b.raw),
            Route("/api/tree", b.tree),
            Route("/api/markdown", b.markdown),
            Route("/api/search", b.search),
            Route("/api/session", b.session),
            Route("/api/chat", b.chat, methods=["POST"]),
            Route("/api/move", b.move, methods=["POST"]),
            Route("/api/file", b.save, methods=["PUT"]),
            Route("/api/file", b.create, methods=["POST"]),
            Route("/api/file", b.delete, methods=["DELETE"]),
        ],
        exception_handlers={SurrealFsError: on_error, KeyError: on_error},
    )
    app.state.browser = b
    return app


async def serve(host: str = "127.0.0.1", port: int = 7933) -> None:
    """Connect and serve on a single event loop.

    A SurrealDB WebSocket belongs to the loop it was opened on, so the connection
    has to be opened on the loop that will serve requests. `uvicorn.run()` makes
    its own loop; drive `uvicorn.Server` ourselves instead and everything stays
    on one.
    """
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
