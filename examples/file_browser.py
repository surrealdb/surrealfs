"""A web file browser for the SurrealFS `file` table.

    just db          # start SurrealDB in one terminal
    just browser     # then run this, and open http://127.0.0.1:7933

Tree on the left, file on the right, chat with the note-taking agent on the far
right: text is editable, markdown renders as HTML with a source toggle, HTML
renders in a sandboxed iframe, images display.

Search is hybrid -- full-text and vector results fused by rank. The vector arm
needs OPENAI_API_KEY; without it search quietly falls back to full-text only.
The chat panel needs ANTHROPIC_API_KEY; without it the rest of the page works
and sending a message reports the missing key.
"""

from __future__ import annotations

import asyncio
import json
import os
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import uvicorn

# chat_agent is the sibling example, resolved because `examples/` is sys.path[0]
# when this file is run as a script. Reusing it keeps the DB config in one place.
from chat_agent import build_chat_agent, connect
from markdown_it import MarkdownIt
from pydantic_ai import AgentRunResultEvent
from pydantic_ai.messages import (
    FunctionToolCallEvent,
    PartDeltaEvent,
    PartStartEvent,
    TextPart,
    TextPartDelta,
    ToolCallPart,
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

from surrealfs import SurrealFs
from surrealfs.embed import INDEXER_VERSION, make_embedder
from surrealfs.errors import (
    AlreadyExists,
    DirectoryNotEmpty,
    InvalidPath,
    NotFound,
    SurrealFsError,
)
from surrealfs.tools import ToolContext

PAGE = Path(__file__).with_name("file_browser.html")

# html=False is NOT the default -- the commonmark preset sets html=True, because
# the CommonMark spec allows raw HTML. The rendered fragment is injected into the
# page with innerHTML, so leaving it on is stored XSS: `<script>` would not fire
# from innerHTML but `<img onerror=…>` very much would. Off means raw HTML in a
# markdown file is escaped and shown as text. Do not turn it back on.
MD = MarkdownIt("commonmark", {"html": False})

STATUS = {
    NotFound: 404,
    AlreadyExists: 409,
    DirectoryNotEmpty: 409,
}


def rrf(*ranked: list[str], k: int = 60) -> list[str]:
    """Reciprocal rank fusion: merge ranked path lists into one ranking.

    The two search arms produce incomparable numbers -- full-text scores are term
    counts (higher is better), semantic scores are cosine distances (lower is
    better). Fusing on rank instead of score sidesteps normalising them.
    """
    scores: dict[str, float] = {}
    for lst in ranked:
        for rank, path in enumerate(lst):
            scores[path] = scores.get(path, 0.0) + 1 / (k + rank)
    return sorted(scores, key=lambda p: -scores[p])


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


def demo() -> None:
    """Self-check: a path ranked mid-list by both arms beats one that tops only one."""
    both = rrf(["a", "both", "b"], ["c", "both", "d"])
    assert both[0] == "both", both
    assert rrf(["x"], []) == ["x"]
    assert rrf() == []

    delta = PartDeltaEvent(index=0, delta=TextPartDelta(content_delta="hi"))
    assert _stream_event(delta) == {"delta": "hi"}
    start = PartStartEvent(index=0, part=TextPart(content="hi"))
    assert _stream_event(start) == {"delta": "\n\nhi"}
    call = FunctionToolCallEvent(ToolCallPart(tool_name="write_file"))
    assert _stream_event(call) == {"tool": "write_file"}
    assert _stream_event(object()) is None


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
        # ponytail: one conversation shared by the whole process, growing without
        # bound. Key it by a session cookie and trim it if this ever serves two
        # people, or runs long enough to outgrow the context window.
        self.history: list[Any] = []

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

        text_hits = await self.fs.search_text(query, limit=20)
        by_path = {h.path: h for h in text_hits}
        ranked = [[h.path for h in text_hits]]

        vector = await self._embed(query)
        if vector is not None:
            vector_hits = await self.fs.search_semantic(vector, k=10)
            ranked.append([h.path for h in vector_hits])
            for hit in vector_hits:
                by_path.setdefault(hit.path, hit)

        results = [
            {
                **_entry_json(by_path[path].entry),
                "snippet": by_path[path].snippet,
            }
            for path in rrf(*ranked)
        ]
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
        message = (await request.json())["message"]

        async def stream() -> AsyncIterator[bytes]:
            reply = ""
            try:
                async with self.agent.run_stream_events(
                    message,
                    deps=ToolContext(fs=self.fs),
                    message_history=self.history,
                ) as events:
                    async for event in events:
                        if isinstance(event, AgentRunResultEvent):
                            self.history = event.result.all_messages()
                        elif (payload := _stream_event(event)) is not None:
                            # Accumulate what was streamed rather than taking
                            # `result.output`: that holds only the final text
                            # part, so anything said before a tool call would
                            # vanish when the rendered reply replaced it.
                            reply = (reply + payload.get("delta", "")).lstrip()
                            yield _line(payload)
                # The agent writes files, so the search index is now stale.
                await self._reindex()
                yield _line({"done": True, "html": MD.render(reply)})
            except Exception as exc:  # noqa: BLE001 -- any failure, same answer
                # The 200 is already on the wire, so failures have to ride the
                # stream: on_error never sees them.
                yield _line({"error": str(exc)})

        return StreamingResponse(stream(), media_type="application/x-ndjson")

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
        """Re-embed what just changed. Incremental: unchanged rows are skipped."""
        if self.embed is None:
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

    Same constraint as the chat agent: the DB connection has to be opened on the
    loop that serves requests, so drive `uvicorn.Server` ourselves rather than
    calling `uvicorn.run()`, which would make its own.
    """
    db = await connect()
    fs = SurrealFs(db)

    embed = make_embedder() if os.environ.get("OPENAI_API_KEY") else None
    if embed is None:
        print("OPENAI_API_KEY unset -- search will be full-text only.")

    agent = build_chat_agent() if os.environ.get("ANTHROPIC_API_KEY") else None
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


if __name__ == "__main__":
    demo()
    asyncio.run(serve())
