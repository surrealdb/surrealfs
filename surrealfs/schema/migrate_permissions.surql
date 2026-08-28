-- Migration: bring an existing database up to the permissions schema.
--
-- Run this ONCE per database that holds rows written before `owner` and `mode`
-- existed, right after applying the new `file.surql`:
--
--     python -m surrealfs.schema --migrate
--
-- It is required, not optional, and the database says so loudly: `gate` is
-- computed from `$parent.mode % 2`, so until every row has a mode, *every* read
-- fails with "Cannot perform remainder with 'none' and 'int'". A fresh database
-- needs none of this -- `DEFAULT` covers rows created from now on, and the
-- statements below match nothing.
--
-- Safe to re-run. Each step is guarded so it only touches rows it has not
-- already migrated; re-running will not reset a mode somebody has since
-- changed with `chmod`.
--
-- `RETURN NONE` on every UPDATE is load-bearing for step 1: projecting a row
-- would evaluate `gate`, which is the very thing that cannot be computed until
-- the backfill is done.

-- 1. Give every legacy row an owner and a mode.
--
-- These are the same defaults `default_mode` applies to new rows: shared
-- read-write, because that is what the tree was before permissions existed and
-- silently tightening somebody's existing files would break their agents.
-- Folders are detected from the stored columns rather than the computed
-- `is_folder`, for the same reason `RETURN NONE` is here.
UPDATE file SET
    owner = 'root',
    mode  = IF content IS NONE AND file IS NONE AND symlink IS NONE
            THEN 511   -- 0o777, a folder
            ELSE 438   -- 0o666, a file
            END
WHERE mode IS NONE
RETURN NONE;

-- 2. Close the homes.
--
-- This is the step that actually secures an existing tree. Every folder
-- directly under `/home` becomes 0700 and is owned by the user it names --
-- `default_owner`'s rule, applied retroactively.
--
-- The `IF $home IS NOT NONE` guard matters: `parent = NONE` matches every
-- top-level row, so without it a database that happens to have no `/home`
-- would make each of its root folders private to a user named after it.
LET $home = (SELECT VALUE id FROM ONLY file
             WHERE parent IS NONE AND filename = 'home' LIMIT 1);

IF $home IS NOT NONE THEN {
    UPDATE file SET owner = filename, mode = 448   -- 0o700
    WHERE parent = $home
      AND content IS NONE AND file IS NONE AND symlink IS NONE
      AND owner = 'root'
    RETURN NONE;

    -- 3. Hand each home's contents to its owner.
    --
    -- Step 1 gave them `root`, which would leave people unable to `chmod` their
    -- own files. Safe to use the computed `path` here: step 1 has run, so every
    -- row has a mode and `gate` resolves.
    --
    -- The names go through a LET first: `FOR $x IN (SELECT ...)` is rejected
    -- with "Cannot execute statement using value: <first row>".
    LET $names = (SELECT VALUE filename FROM file WHERE parent = $home);
    FOR $name IN $names {
        UPDATE file SET owner = $name
        WHERE string::starts_with(path, '/home/' + $name + '/')
          AND owner = 'root'
        RETURN NONE;
    };
} END;

-- 4. Retire the previous optional `user` schema.
--
-- Only present if the old `apply_schema(include_user=True)` was ever run. That
-- table keyed users on `email`; the new one in `record_auth.surql` keys them on
-- the record id. `DEFINE TABLE OVERWRITE` does *not* drop field or index
-- definitions, so without this the leftover required `email` makes
-- `python -m surrealfs.users add` fail on a coercion error.
--
-- Any old `user` rows are left in place rather than deleted: they cannot sign
-- in under the new access method, which keys on the record id, and removing
-- somebody's data is not a migration's job. Drop them yourself if you want to.
REMOVE INDEX IF EXISTS user_email_unique ON TABLE user;
REMOVE FIELD IF EXISTS email ON TABLE user;
