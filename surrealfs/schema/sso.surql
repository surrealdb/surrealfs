-- Optional: let an external identity provider sign people in to SurrealFS.
--
-- This is the SSO half of record auth. `record_auth.surql` already gives the
-- `user` table and a password `SIGNIN`; this adds a *second* access method that
-- trusts a JWT instead, for the browser at `surrealfs/browser/sso.py`:
--
--     SURREALFS_SSO_SECRET=... python -m surrealfs.schema --record-auth --sso
--     python -m surrealfs.users add alice
--
-- Applied separately from `apply_schema()` because it needs a secret. The key is
-- bound as `$secret` rather than written here, so nothing in this repo is a file
-- that leaks the deployment's signing key if it is read or committed.
--
-- The password access stays: `account` is what agents and the CLI use, `sso` is
-- what a person's browser uses, and one deployment wants both.

-- ## Why the token is minted, and not the identity provider's own
--
-- SurrealDB routes a third-party JWT to an access method using `ns`, `db` and
-- `ac` claims, and takes the record from `id`. A Cloudflare Access assertion --
-- or a Google, Okta or Auth0 id_token -- carries none of those, so it cannot be
-- handed to `authenticate()` directly whatever the signature says. The browser
-- therefore verifies the provider's assertion itself and mints this token from
-- it, which is also where "the user must already exist" is answered with a
-- readable error rather than a denial six layers down.
--
-- HS512 and a shared secret rather than a keypair: the only signer is the same
-- process that verifies the assertion, so there is no second party to hand a
-- public key to, and a symmetric secret is one environment variable instead of
-- a key pair to provision. `sso.py` refuses a secret under 64 bytes, which is
-- the HMAC-SHA512 block size (RFC 7518 3.2).
DEFINE ACCESS OVERWRITE sso ON DATABASE TYPE RECORD
    WITH JWT ALGORITHM HS512 KEY $secret
    AUTHENTICATE {
        -- `id` in the token means SurrealDB has already bound `$auth` by the
        -- time this runs -- *without* checking that the row exists. Skip this
        -- and a token naming `user:ghost` yields a working session for someone
        -- who was never provisioned: `fn::sfs_me()` returns 'ghost', and
        -- `fn::sfs_home_ok` would then let them create and own `/home/ghost`.
        -- That is exactly the squatting hole `record_auth.surql` refuses a
        -- SIGNUP clause to avoid, so it is refused here too. Provisioning stays
        -- an admin operation: `python -m surrealfs.users add alice`.
        IF record::exists($auth) { RETURN $auth };
        THROW 'no such user';
    }
    -- Same as `account`. The token is minted per request and never stored, so
    -- the 15m is a ceiling on a mistake rather than a session length.
    DURATION FOR TOKEN 15m, FOR SESSION 12h;
