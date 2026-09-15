"""The file browser, with a login.

    pip install "surrealfs[browser-sso]"
    surrealfs-browser-sso

Same page as `surrealfs-browser`, but every request runs as the person who made
it, and **SurrealDB** enforces the file permissions rather than this process
being trusted. Built to sit behind Cloudflare Access; `deploy/cloudflare/` is the
wrangler project that puts it there.

The plain browser is not deprecated by this one. It signs in with one credential
and has no login, which is the right shape for a laptop and one agent. This one
costs a provisioned user per person and an identity provider, and is the right
shape for a team.

## How a request becomes an identity

1. Cloudflare Access authenticates the person and adds a signed
   `Cf-Access-Jwt-Assertion` header. We verify it against the team's JWKS.
2. The verified `email` claim becomes a SurrealFS username (see `_username`).
3. We mint a short-lived SurrealDB token for `user:<name>` and open a *database
   session* with it, on a connection that is itself never authenticated.
4. `SurrealFs(session, user=name)` serves the request and the session is closed.

Step 3 exists because SurrealDB routes a third-party JWT to an access method by
its `ns`, `db` and `ac` claims and takes the record from `id`. An Access
assertion has none of those, so it cannot be forwarded as-is -- see
`surrealfs/schema/sso.surql`.

## Setup, once

    openssl rand -base64 48                       # the signing secret
    SURREALFS_SSO_SECRET=... python -m surrealfs.schema --record-auth --sso
    python -m surrealfs.users add alice

    SURREALFS_SSO_JWKS=https://<team>.cloudflareaccess.com/cdn-cgi/access/certs
    SURREALFS_SSO_ISSUER=https://<team>.cloudflareaccess.com
    SURREALFS_SSO_AUD=<the Access application's AUD tag>
    SURREALFS_SSO_SECRET=<the same secret>
    SURREALDB_URL=wss://....surreal.cloud/rpc
    SURREALDB_NAMESPACE=... SURREALDB_DATABASE=...
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any

import uvicorn
from starlette.middleware import Middleware
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request
from starlette.responses import JSONResponse, Response
from starlette.routing import Route
from surrealdb import AsyncSurreal
from surrealdb.errors import ConnectionUnavailableError
from websockets.exceptions import ConnectionClosed

from ..embed import make_embedder
from ..errors import PermissionDenied
from ..fs import ROOT, SurrealFs
from ..integrations._connect import (
    _database,
    _namespace,
    _slug,
    connect,
)
from ..schema import SSO_SECRET_BYTES
from . import INDEX, Browser, assemble
from .agent import build_chat_agent

# The access method in `schema/sso.surql`. Not `account`: that one is password
# signin, which agents and the CLI still use.
SSO_ACCESS = "sso"

# Cloudflare Access sends the assertion in both a header and a `CF_Authorization`
# cookie, and documents that the cookie "is not guaranteed to be passed". The
# header is also the harder of the two to forge from another site, so it is the
# only one read here -- a cookie would be attached to a cross-site request, a
# header would not.
ASSERTION_HEADER = "cf-access-jwt-assertion"

# How long a minted SurrealDB token lives. It is created per request and thrown
# away with it, so this is a ceiling on a leak rather than a session length. The
# `sso` access caps it at the same figure.
TOKEN_TTL = 900

# Methods that change something, and so need the CSRF check.
MUTATING = frozenset({"POST", "PUT", "PATCH", "DELETE"})


class Unauthenticated(PermissionDenied):
    """No usable identity on the request. A 403, via the browser's STATUS map."""


def _env(name: str) -> str:
    if value := os.environ.get(name, "").strip():
        return value
    raise SystemExit(
        f"{name} is unset. `surrealfs-browser-sso` cannot start without it -- "
        "see the setup block in surrealfs/browser/sso.py."
    )


def _username(email: str, domain: str | None) -> str:
    """The SurrealFS user an email address signs in as.

    The safe default is the **whole** address slugged (`alice@corp.com` ->
    `alice-corp-com`), because an Access policy can admit more than one domain
    and the local part alone is not unique across them: `alice@contractor.com`
    would otherwise land in `alice@corp.com`'s home, which is the one mistake
    that cannot be undone -- there is no `chown`, and a home is 0700.

    `SURREALFS_SSO_DOMAIN=corp.com` names one domain whose local part is short
    enough to stand alone, giving `/home/alice`. Anyone from any *other* domain
    still gets the full slug, so turning it on cannot create a collision -- it
    privileges exactly one domain and says which.
    """
    local, _, host = email.rpartition("@")
    if not local:  # no '@' at all: not an address, but slug it rather than guess
        return _slug(email)
    if domain and host.lower() == domain.lower():
        return _slug(local)
    return _slug(email)


class Identity:
    """Verifies an SSO assertion and mints a SurrealDB token from it."""

    def __init__(
        self,
        jwks: str,
        issuer: str,
        audience: str,
        secret: str,
        domain: str | None = None,
    ) -> None:
        import jwt

        if len(secret.encode("utf-8")) < SSO_SECRET_BYTES:
            raise SystemExit(
                f"SURREALFS_SSO_SECRET must be at least {SSO_SECRET_BYTES} bytes "
                "(HMAC-SHA512's block size). Generate one with "
                "`openssl rand -base64 48`."
            )
        self._jwt = jwt
        # PyJWKClient caches keys and picks by `kid`, which is what Cloudflare
        # asks for: pinning a key breaks silently on the next rotation.
        self._keys = jwt.PyJWKClient(jwks)
        self._issuer = issuer
        self._audience = audience
        self._secret = secret
        self._domain = domain

    def username(self, request: Request) -> str:
        """Who made this request, or raise. The only place identity is decided."""
        token = request.headers.get(ASSERTION_HEADER)
        if not token:
            raise Unauthenticated(
                "no SSO assertion on the request. This browser must be reached "
                "through the identity proxy that signs one."
            )
        try:
            key = self._keys.get_signing_key_from_jwt(token).key
            claims = self._jwt.decode(
                token,
                key,
                algorithms=["RS256"],
                audience=self._audience,
                issuer=self._issuer,
                # The defaults, spelled out: an assertion that has expired or
                # names another application is the case this exists to catch.
                options={"require": ["exp", "aud", "iss"]},
            )
        except Exception as exc:  # noqa: BLE001 -- every rejection is the same 403
            raise Unauthenticated(f"the SSO assertion was rejected: {exc}") from None

        email = claims.get("email")
        if not email:
            raise Unauthenticated("the SSO assertion carries no email claim")
        name = _username(email, self._domain)
        if name == ROOT:
            # Unreachable through `_slug` today, but `root` is the permission
            # bypass and `users.add` refuses it by name; so does this.
            raise Unauthenticated(f"{ROOT!r} is not a name anyone may sign in as")
        return name

    def token(self, name: str) -> str:
        """A SurrealDB token for `name`, good for `TOKEN_TTL` seconds.

        Minted per request rather than cached: signing is microseconds, and a
        token that cannot outlive the request it was made for is one less thing
        to store, expire or invalidate.
        """
        now = int(time.time())
        return self._jwt.encode(
            {
                # `ns`, `db` and `ac` are how SurrealDB finds the access method
                # to verify against; `id` is the record it binds `$auth` to.
                "ns": _namespace(),
                "db": _database(),
                "ac": SSO_ACCESS,
                "id": f"user:{name}",
                "iat": now,
                "exp": now + TOKEN_TTL,
            },
            self._secret,
            algorithm="HS512",
        )


class Sessions:
    """One WebSocket, one authenticated database session per request.

    SurrealDB 3.x multiplexes sessions over a single connection (`new_session()`
    -> `attach`), so per-request identity costs three RPCs on an already-open
    socket rather than a new connection each time.

    The connection is **never authenticated**. That is the load-bearing part:
    `new_session()` replays the connection's own token when it has one, so a
    socket signed in as root would hand out root sessions whenever
    `authenticate()` failed. With no token to replay, a session that is not
    authenticated has no identity at all, `fn::sfs_me()` is NONE, and the
    `file` table's PERMISSIONS clause denies everything. It fails closed.
    """

    def __init__(self, url: str, identity: Identity) -> None:
        self.identity = identity
        if not url.startswith(("ws://", "wss://")):
            raise SystemExit(
                f"SURREALDB_URL must be a WebSocket URL for this browser: {url!r}. "
                "Per-request identity uses multiplexed sessions, which the HTTP "
                "transport does not have."
            )
        self._url = url
        self._db: Any = None
        self._lock = asyncio.Lock()

    async def open(self) -> None:
        self._db = AsyncSurreal(self._url)
        await self._db.connect()

    async def close(self) -> None:
        if self._db is not None:
            await self._db.close()

    async def _reopen(self, dead: Any) -> None:
        async with self._lock:
            # Another request that failed on the same socket got here first.
            if self._db.socket is not dead:
                return
            await self._db.close()
            await self._db.connect()

    @asynccontextmanager
    async def session(self, request: Request) -> AsyncIterator[SurrealFs]:
        """The `SurrealFs` this request acts through, released afterwards."""
        name = signed_in_user(request)
        token = self.identity.token(name)
        try:
            session = await self._attach()
        except (ConnectionClosed, ConnectionUnavailableError):
            # Only retried here, at the door. A drop *mid-request* is not retried:
            # reopening the socket destroys the session, and a half-applied write
            # is worse than a 500 the page can repeat.
            await self._reopen(self._db.socket)
            session = await self._attach()
        try:
            await session.authenticate(token)
            await session.use(_namespace(), _database())
            # The same string reaches both layers: `user=` here, and `$auth` in
            # the database via the `id` claim this token was minted with. They
            # cannot disagree about whose permissions apply to whose rows.
            yield SurrealFs(session, user=name)
        finally:
            await session.close_session()

    async def _attach(self) -> Any:
        return await self._db.new_session()


class DevIdentity(Identity):
    """`--dev-identity alice`: skip SSO and be that person. Loopback only.

    There is no way to exercise the page without an identity provider in front
    of it, and standing one up to check a CSS change is not reasonable. This is
    the escape hatch, and `_parse_args` refuses it on any bind but loopback so
    it cannot be switched on in a deployment by accident.
    """

    def __init__(self, name: str, secret: str) -> None:
        import jwt

        self._jwt = jwt
        self._secret = secret
        self._name = _slug(name)

    def username(self, request: Request) -> str:
        return self._name


# Where the verified username is left for the rest of the request to read. In
# the scope rather than on `request.state`, because BaseHTTPMiddleware builds
# its own Request object and only the scope is shared with the endpoint.
USER_SCOPE_KEY = "surrealfs_user"


def signed_in_user(request: Request) -> str:
    """The username `Gate` verified for this request."""
    name = request.scope.get(USER_SCOPE_KEY)
    if name is None:  # unreachable behind Gate; a loud failure if it is ever removed
        raise Unauthenticated("this request was not authenticated")
    return name


class Gate(BaseHTTPMiddleware):
    """Verify the assertion once, before any handler runs.

    Middleware rather than a check inside `Sessions.session` for two reasons.
    `chat` returns a StreamingResponse whose body runs after the handler, and
    catches everything inside it to report errors down the stream -- so a denial
    raised there would arrive as a 200 with an error frame. And verification
    happens exactly once per request instead of once per session opened.

    It also refuses cross-site writes. The identity rides on a cookie Cloudflare
    Access sets, so a page on another origin can cause a request that carries
    it. Every mutating endpoint sends JSON today and is therefore preflighted,
    which CORS already blocks; this keeps that true if one ever stops.
    `Sec-Fetch-Site` is set by the browser and cannot be forged by the page,
    and absent means a client too old to send it, not an attacker.
    """

    def __init__(self, app: Any, identity: Identity) -> None:
        super().__init__(app)
        self.identity = identity

    async def dispatch(self, request: Request, call_next: Any) -> Response:
        site = request.headers.get("sec-fetch-site")
        if request.method in MUTATING and site not in (None, "same-origin"):
            return JSONResponse(
                {"error": f"cross-site {request.method} refused"}, status_code=403
            )
        try:
            request.scope[USER_SCOPE_KEY] = self.identity.username(request)
        except Unauthenticated as exc:
            return JSONResponse({"error": str(exc)}, status_code=403)
        return await call_next(request)


def build_sso_app(
    sessions: Sessions,
    embed: Any = None,
    agent: Any = None,
    indexer: SurrealFs | None = None,
) -> Any:
    b = Browser(sessions.session, embed, agent, indexer=indexer)

    async def whoami(request: Request) -> Response:
        return JSONResponse({"user": signed_in_user(request)})

    return assemble(
        b,
        routes=[Route("/api/whoami", whoami)],
        middleware=[Middleware(Gate, identity=sessions.identity)],
    )


async def serve(
    host: str = "0.0.0.0",  # noqa: S104 -- it lives in a container; see main()
    port: int = 7933,
    dev_identity: str | None = None,
) -> None:
    """Connect and serve on a single event loop.

    Two connections, and the split matters: `sessions` is the unauthenticated
    socket every request gets its identity on, and the root one below exists
    *only* to re-embed. Nothing may hand a request the root handle.
    """
    if not INDEX.exists():
        raise SystemExit(
            f"the browser UI is not built ({INDEX} is missing) -- run `just ui`"
        )

    secret = _env("SURREALFS_SSO_SECRET")
    identity: Identity = (
        DevIdentity(dev_identity, secret)
        if dev_identity
        else Identity(
            jwks=_env("SURREALFS_SSO_JWKS"),
            issuer=_env("SURREALFS_SSO_ISSUER"),
            audience=_env("SURREALFS_SSO_AUD"),
            secret=secret,
            domain=os.environ.get("SURREALFS_SSO_DOMAIN") or None,
        )
    )

    sessions = Sessions(
        os.environ.get("SURREALDB_URL", "ws://localhost:8000/rpc"), identity
    )
    await sessions.open()

    embed = make_embedder() if os.environ.get("OPENAI_API_KEY") else None
    if embed is None:
        print("OPENAI_API_KEY unset -- search will be full-text only.")

    # Opened only when there is something to embed with. `reindex_embeddings`
    # reads every file in the tree, so it needs a system credential -- and a
    # process that does not need one should not be holding one.
    #
    # A failure here downgrades rather than refusing to start, the same way a
    # dead embedding provider does in `_reindex`: semantic search going stale is
    # not a reason to withhold the file browser from everyone. The *user* half
    # needs no credential at all, so this cannot take the login down with it.
    root_db = None
    if embed is not None:
        try:
            root_db = await connect()
        except Exception as exc:  # noqa: BLE001 -- any signin failure, same answer
            print(
                f"no system credential, so new files will not be embedded: {exc}\n"
                "  (SURREALDB_USER/PASS are only needed for that; search still "
                "works against whatever is already indexed.)"
            )
    indexer = SurrealFs(root_db, user=ROOT) if root_db is not None else None

    agent = (
        build_chat_agent(semantic=embed is not None)
        if os.environ.get("ANTHROPIC_API_KEY")
        else None
    )
    if agent is None:
        print("ANTHROPIC_API_KEY unset -- the chat panel will refuse to send.")

    app = build_sso_app(sessions, embed, agent, indexer)
    await app.state.browser._reindex()
    print(f"Browsing SurrealFS on http://{host}:{port}")
    if dev_identity:
        print(
            f"\n  !!  --dev-identity {dev_identity}: SSO is OFF and every request "
            f"is {dev_identity}.\n      Loopback only. Never in a deployment.\n"
        )
    try:
        await uvicorn.Server(uvicorn.Config(app, host=host, port=port)).serve()
    finally:
        await sessions.close()
        if root_db is not None:
            await root_db.close()


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="surrealfs-browser-sso",
        description="Browse a SurrealFS `file` table as the person signed in.",
    )
    # 0.0.0.0 by default, unlike `surrealfs-browser`: this one lives in a
    # container behind an identity proxy, where loopback would answer nothing.
    parser.add_argument("--host", default="0.0.0.0")  # noqa: S104
    parser.add_argument("--port", type=int, default=7933)
    parser.add_argument(
        "--dev-identity",
        metavar="USER",
        help="skip SSO and serve every request as USER. Loopback binds only.",
    )
    args = parser.parse_args(argv)
    if args.dev_identity and args.host not in ("127.0.0.1", "::1", "localhost"):
        parser.error(
            f"--dev-identity cannot be used with --host {args.host}: it turns "
            "authentication off, so it is loopback-only. Use --host 127.0.0.1."
        )
    return args


def main(argv: list[str] | None = None) -> int:
    try:
        from dotenv import find_dotenv, load_dotenv
    except ImportError as exc:
        print(f"{exc.name} is missing. Install the extra:", file=sys.stderr)
        print('    pip install "surrealfs[browser-sso]"', file=sys.stderr)
        return 1

    # usecwd=True for the same reason as `surrealfs-browser`: plain find_dotenv
    # walks up from this module, which inside a checkout is this repo.
    load_dotenv(find_dotenv(usecwd=True))
    args = _parse_args(argv)
    try:
        asyncio.run(
            serve(host=args.host, port=args.port, dev_identity=args.dev_identity)
        )
    except KeyboardInterrupt:
        return 0
    except Exception as exc:  # noqa: BLE001 - a CLI should not show a traceback
        print(f"Browser stopped: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
