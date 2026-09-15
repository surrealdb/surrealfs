# The authenticated browser, on Cloudflare

`surrealfs-browser` has no login. This deploys `surrealfs-browser-sso` instead:
the same page, behind Cloudflare Access, where every request runs as the person
who made it and **SurrealDB** enforces the file permissions.

```
Browser ─▶ Cloudflare Access ─▶ Worker ─▶ Container ─▶ SurrealDB
            SSO, signs the      forwards   verifies,    PERMISSIONS
            assertion           verbatim   mints,       enforce
                                           opens a      owner/mode
                                           session
```

The Worker does nothing but forward. The application is the same Starlette app
you can run on a laptop, which is the point: there is no second implementation
of the API, and no filesystem logic written twice.

## Why the token is minted rather than forwarded

SurrealDB routes a third-party JWT to an access method using its `ns`, `db` and
`ac` claims, and takes the record from `id`. A Cloudflare Access assertion
carries none of those, so it cannot be handed to SurrealDB whatever its
signature says. The container therefore verifies the assertion itself and mints
a 15-minute SurrealDB token from the verified email. That is also where "this
user was never provisioned" becomes a readable 403 instead of an empty result
six layers down.

## Setup

### 1. The database, once

Needs a system credential — a record user has no DDL rights.

```bash
openssl rand -base64 48                  # keep this; it is SURREALFS_SSO_SECRET
export SURREALFS_SSO_SECRET=…
python -m surrealfs.schema --record-auth --sso
python -m surrealfs.users add alice      # one per person, no self-registration
```

There is deliberately no signup. `/home/<name>` belongs to the name it carries,
so letting anyone claim an unused username would hand over that user's home —
see `docs/permissions.md`.

**Which name?** By default an address maps to its full slug, so
`alice@corp.com` is the user `alice-corp-com`. Set `SURREALFS_SSO_DOMAIN=corp.com`
to shorten exactly that one domain to `alice`; anyone from any other domain
keeps the long form, so turning it on cannot make two people collide on one
home.

### 2. The Access application

Zero Trust → Access → Applications → **Self-hosted**, over the hostname this
Worker will serve. Add whichever identity providers and policies you want —
Access normalises all of them to one assertion, so nothing here changes per
provider. Then copy the **Application Audience (AUD) tag**.

### 3. Configure and deploy

Edit the `vars` block in `wrangler.jsonc` (team domain, AUD tag, SurrealDB URL,
namespace, database), then:

```bash
cd deploy/cloudflare && npm install

wrangler secret put SURREALFS_SSO_SECRET   # the same one from step 1
wrangler secret put SURREALDB_PASS         # only needed for embeddings, below
wrangler secret put OPENAI_API_KEY         # optional: semantic search
wrangler secret put ANTHROPIC_API_KEY      # optional: the chat panel

cd ../.. && just ui                        # the page is gitignored build output
cd deploy/cloudflare && wrangler deploy
```

`SURREALDB_USER`/`SURREALDB_PASS` are a **system** credential, and the app opens
a connection with them *only* when `OPENAI_API_KEY` is set — re-embedding has to
read every file in the tree, so it is root-only by design. Leave the embedding
key unset and the process holds no system credential at all. That connection is
never handed to a request; requests only ever get a session on the separate,
unauthenticated socket.

## Running it locally

Against Access you cannot, so there is an escape hatch, and it is loopback-only:

```bash
just db
SURREALFS_SSO_SECRET=$(openssl rand -base64 48) \
  python -m surrealfs.schema --record-auth --sso
python -m surrealfs.users add alice
just browser-sso alice
```

`--dev-identity` turns authentication off and serves every request as that
person. `_parse_args` refuses it on any bind but loopback, so it cannot be
switched on in a deployment by accident.

`wrangler dev` runs the real container locally, which is the way to check the
Dockerfile and the Worker; the assertion still has to come from Access.

## Checking it without deploying

```bash
npm run check     # tsc + `wrangler deploy --dry-run`
```

The dry run builds the container image, so it needs a running Docker daemon.
Without one, `wrangler deploy --dry-run --containers-rollout=none` still
validates the Worker, its bindings and the migrations — everything except the
image itself.

## Things that will bite

**The page is a build artefact and it is gitignored.** `just ui` before
`wrangler deploy`. The Dockerfile fails the build if you forget, which is the
cheapest place to find out.

**`Env` is declared in `env.d.ts`, not generated.** `wrangler types` writes the
`vars` out as *literal* types from whatever placeholder values `wrangler.jsonc`
holds, which in a template is both enormous and wrong. Add a variable there and
in `PASS_THROUGH` in `worker.ts`, or the container will not see it.

**A dropped WebSocket is only retried at the door.** The app reopens its socket
when acquiring a session fails, but not mid-request: reopening destroys the
session, and a half-applied write is worse than a 500 the page can repeat.

**`max_instances` is about concurrency, not isolation.** Each instance holds one
WebSocket and multiplexes a session per request onto it. Users are isolated by
SurrealDB, not by which container they land on.
