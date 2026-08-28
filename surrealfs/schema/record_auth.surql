-- Optional: make SurrealDB itself enforce the `file` table's permissions.
--
-- Without this, SurrealFS is secured by `SurrealFs` alone -- which is a real
-- boundary for an agent, because no tool exposes raw SurrealQL, but not one
-- against anyone holding the database credential. Apply this when several
-- agents share a database and you want the store to enforce the split too:
--
--     await apply_schema(db, record_auth=True)
--     python -m surrealfs.users add alice
--     SURREALDB_AUTH_LEVEL=record SURREALDB_USER=alice SURREALDB_PASS=...
--
-- The `file` table's PERMISSIONS clause is defined unconditionally in
-- file.surql; it simply does nothing until someone connects as a record user,
-- because root/namespace/database users bypass table permissions. This file
-- adds only the identity half. See `docs/permissions.md`.

-- The username *is* the record id, so `user:alice` owns `/home/alice`. That
-- keeps `fn::sfs_me()` a pure read of `$auth` with no row fetch, needs no
-- unique index, and means a username cannot drift from the home it names.
--
-- `FOR select WHERE id = $auth` compares against `$auth` itself rather than
-- `$auth.id`: `$auth` is bound to a record *id*, and dereferencing a field on
-- it outside a permission clause would re-enter this very table's permissions.
DEFINE TABLE OVERWRITE user SCHEMAFULL
    PERMISSIONS
        FOR select WHERE id = $auth
        FOR create, update, delete NONE;

DEFINE FIELD OVERWRITE password   ON TABLE user TYPE string;
DEFINE FIELD OVERWRITE created_at ON TABLE user TYPE datetime VALUE time::now() READONLY;

-- No SIGNUP, deliberately. Self-service registration would let anyone claim any
-- unused username -- and with it that user's home, since `/home/<name>` is
-- owned by the name it carries. That is the squatting hole `_guard_home` closes
-- on the Python side, and it would be wide open here. Provisioning is an admin
-- operation: `python -m surrealfs.users add <name>`.
DEFINE ACCESS OVERWRITE account ON DATABASE TYPE RECORD
    SIGNIN (
        SELECT * FROM ONLY type::record('user', $user)
        WHERE crypto::argon2::compare(password, $pass)
    )
    DURATION FOR TOKEN 15m, FOR SESSION 12h;
