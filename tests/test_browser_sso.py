"""The authenticated browser: who a request is, and what that lets it do.

Two halves, tested apart and then together:

* the JWT half -- an SSO assertion becomes a username, or is refused. No
  database; a generated RS256 keypair stands in for the identity provider's.
* the database half -- that username becomes a session that SurrealDB enforces.
  No JWT; `DevIdentity` supplies the name directly.

Then one test that the two compose, through the real ASGI app.
"""

from __future__ import annotations

import time

import httpx
import jwt
import pytest
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from surrealdb import AsyncSurreal

from surrealfs import users
from surrealfs.browser import sso
from surrealfs.errors import NotFound, PermissionDenied
from surrealfs.schema import apply_schema, apply_sso_access

# 64 bytes: `apply_sso_access` and `Identity` both refuse anything shorter.
SECRET = "s" * 64
ISSUER = "https://example.cloudflareaccess.com"
AUD = "0123456789abcdef"


@pytest.fixture(scope="module")
def idp():
    """A stand-in identity provider: a keypair and a matching signer."""
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    pem = key.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption(),
    )

    def assertion(**claims):
        now = int(time.time())
        payload = {
            "iss": ISSUER,
            "aud": AUD,
            "iat": now,
            "exp": now + 600,
            "email": "alice@corp.com",
            **claims,
        }
        return jwt.encode(payload, pem, algorithm="RS256")

    return type(
        "Idp", (), {"public": key.public_key(), "assertion": staticmethod(assertion)}
    )


@pytest.fixture
def identity(idp, monkeypatch):
    """A real `Identity`, with only the JWKS fetch stubbed out."""
    monkeypatch.setattr(
        sso.jwt if hasattr(sso, "jwt") else jwt,
        "PyJWKClient",
        lambda url: type(
            "Keys",
            (),
            {
                "get_signing_key_from_jwt": lambda self, t: type(
                    "K", (), {"key": idp.public}
                )()
            },
        )(),
    )
    return sso.Identity(
        jwks="https://example.invalid/certs",
        issuer=ISSUER,
        audience=AUD,
        secret=SECRET,
        domain="corp.com",
    )


def request_with(token: str | None):
    """The least Request that `Identity.username` reads: just headers."""
    from starlette.requests import Request

    headers = [(sso.ASSERTION_HEADER.encode(), token.encode())] if token else []
    return Request({"type": "http", "method": "GET", "headers": headers})


def already_verified(name: str):
    """A request as `Gate` leaves it: the username verified and in the scope."""
    from starlette.requests import Request

    return Request(
        {"type": "http", "method": "GET", "headers": [], sso.USER_SCOPE_KEY: name}
    )


# --- the JWT half ---------------------------------------------------------


def test_an_email_becomes_a_username(identity, idp):
    assert identity.username(request_with(idp.assertion())) == "alice"


def test_a_domain_that_is_not_the_configured_one_keeps_its_full_address(identity, idp):
    """The collision this exists to prevent: two alices, two homes.

    An Access policy can admit more than one domain, and a home is 0700 with no
    `chown` -- so handing `alice@contractor.com` the home of `alice@corp.com`
    would be permanent.
    """
    token = idp.assertion(email="alice@contractor.com")
    assert identity.username(request_with(token)) == "alice-contractor-com"


@pytest.mark.parametrize(
    "case, claims",
    [
        ("expired", {"exp": int(time.time()) - 60}),
        ("another application", {"aud": "someone-elses-app"}),
        ("another issuer", {"iss": "https://evil.example.com"}),
        ("no email", {"email": None}),
    ],
)
def test_a_bad_assertion_is_refused(identity, idp, case, claims):
    claims = {k: v for k, v in claims.items() if v is not None} or {"email": ""}
    with pytest.raises(PermissionDenied):
        identity.username(request_with(idp.assertion(**claims)))


def test_an_assertion_signed_by_someone_else_is_refused(identity):
    other = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    pem = other.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption(),
    )
    forged = jwt.encode(
        {
            "iss": ISSUER,
            "aud": AUD,
            "exp": int(time.time()) + 600,
            "email": "a@corp.com",
        },
        pem,
        algorithm="RS256",
    )
    with pytest.raises(PermissionDenied):
        identity.username(request_with(forged))


def test_no_assertion_at_all_is_refused(identity):
    with pytest.raises(PermissionDenied):
        identity.username(request_with(None))


def test_the_minted_token_carries_what_surrealdb_routes_on(identity, monkeypatch):
    monkeypatch.setenv("SURREALDB_NAMESPACE", "ns1")
    monkeypatch.setenv("SURREALDB_DATABASE", "db1")
    claims = jwt.decode(identity.token("alice"), SECRET, algorithms=["HS512"])
    # Without all four SurrealDB cannot find the access method or the record.
    assert claims["ns"] == "ns1"
    assert claims["db"] == "db1"
    assert claims["ac"] == sso.SSO_ACCESS
    assert claims["id"] == "user:alice"
    assert claims["exp"] - claims["iat"] == sso.TOKEN_TTL


def test_a_short_signing_secret_is_refused():
    with pytest.raises(SystemExit):
        sso.Identity("j", ISSUER, AUD, "too-short")


def test_a_handler_reached_without_the_gate_refuses_rather_than_guesses():
    """`signed_in_user` is the one read of the identity; it must not default."""
    with pytest.raises(PermissionDenied):
        sso.signed_in_user(request_with(None))


def test_dev_identity_is_refused_off_loopback():
    with pytest.raises(SystemExit):
        sso._parse_args(["--dev-identity", "alice", "--host", "0.0.0.0"])
    assert sso._parse_args(["--dev-identity", "alice", "--host", "127.0.0.1"])


# --- the database half ----------------------------------------------------


@pytest.fixture
async def provisioned(db, surreal_url, namespace, monkeypatch):
    """Record auth + the `sso` access + two users, on this test's namespace."""
    monkeypatch.setenv("SURREALDB_NAMESPACE", namespace)
    monkeypatch.setenv("SURREALDB_DATABASE", "test")
    await apply_schema(db, record_auth=True)
    await apply_sso_access(db, SECRET)
    await users.add(db, "alice", "pw-alice")
    await users.add(db, "bob", "pw-bob")
    return surreal_url


async def sessions_for(url, name):
    s = sso.Sessions(url, sso.DevIdentity(name, SECRET))
    await s.open()
    return s


async def test_each_session_acts_as_its_own_user(provisioned):
    alice = await sessions_for(provisioned, "alice")
    bob = await sessions_for(provisioned, "bob")
    try:
        async with alice.session(already_verified("alice")) as fs:
            await fs.write_text("/home/alice/secret.md", "alice's")
            assert fs.user == "alice"
        async with bob.session(already_verified("bob")) as fs:
            await fs.write_text("/home/bob/secret.md", "bob's")
            # Record auth filters per row, so another home is not merely
            # unreadable, it is absent.
            assert [e.filename for e in await fs.ls("/home")] == ["bob"]
            with pytest.raises((NotFound, PermissionDenied)):
                await fs.read_text("/home/alice/secret.md")
    finally:
        await alice.close()
        await bob.close()


async def test_a_session_that_was_never_authenticated_sees_nothing(provisioned):
    """The fail-closed property the two-connection split is built on.

    `new_session()` replays the connection's token when it has one, so the
    socket requests are served on must never be signed in. If this ever returns
    rows, an `authenticate()` that raised would leave a usable session behind.
    """
    connection = AsyncSurreal(provisioned)
    await connection.connect()
    try:
        bare = await connection.new_session()
        await bare.use(*_ns_db())
        result = await bare.query_raw("SELECT * FROM file")
        assert result["result"][-1]["result"] == []
    finally:
        await connection.close()


async def test_an_unprovisioned_user_cannot_sign_in(provisioned):
    """`AUTHENTICATE` THROWs, so a name nobody provisioned gets no session.

    Without that check `$auth` is bound to a record that does not exist, and
    `fn::sfs_home_ok` would let `user:ghost` create and own `/home/ghost`.
    """
    ghost = await sessions_for(provisioned, "ghost")
    try:
        with pytest.raises(Exception) as caught:
            async with ghost.session(already_verified("ghost")):
                pass
        assert "no such user" in str(caught.value)
    finally:
        await ghost.close()


def _ns_db():
    import os

    return os.environ["SURREALDB_NAMESPACE"], os.environ["SURREALDB_DATABASE"]


# --- both halves, through the app ----------------------------------------


async def test_the_app_serves_the_signed_in_user_and_refuses_everyone_else(
    provisioned, identity, idp
):
    sessions = sso.Sessions(provisioned, identity)
    await sessions.open()
    app = sso.build_sso_app(sessions)
    try:
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(app=app), base_url="http://t"
        ) as client:
            headers = {sso.ASSERTION_HEADER: idp.assertion()}
            assert (await client.get("/api/whoami", headers=headers)).json() == {
                "user": "alice"
            }

            created = await client.post(
                "/api/file",
                json={"path": "/home/alice/n.md", "folder": False},
                headers=headers,
            )
            assert created.status_code == 200

            # No assertion: a 403, not a tree.
            assert (await client.get("/api/tree")).status_code == 403

            # A cross-site write, carrying a perfectly valid assertion.
            refused = await client.post(
                "/api/file",
                json={"path": "/home/alice/x.md", "folder": False},
                headers={**headers, "sec-fetch-site": "cross-site"},
            )
            assert refused.status_code == 403
    finally:
        await sessions.close()
