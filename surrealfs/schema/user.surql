-- Optional companion schema for the `file` table's ownership model.
--
-- `file.owner` is `record<user>` and the table's PERMISSIONS clause scopes every
-- operation to `owner = $auth.id`. That clause only does anything under *record*
-- authentication -- root/namespace/database users bypass table permissions
-- entirely, and `$auth.id` is NONE for them (so `owner` defaults to NONE).
--
-- Apply this only if you want per-user isolation:
--     await apply_schema(db, include_user=True)

DEFINE TABLE OVERWRITE user SCHEMAFULL
    PERMISSIONS
        FOR select, update, delete WHERE id = $auth.id
        FOR create NONE;

DEFINE FIELD OVERWRITE email      ON TABLE user TYPE string
    ASSERT string::is_email($value);
DEFINE FIELD OVERWRITE password   ON TABLE user TYPE string;
DEFINE FIELD OVERWRITE created_at ON TABLE user TYPE datetime VALUE time::now() READONLY;

DEFINE INDEX OVERWRITE user_email_unique ON TABLE user FIELDS email UNIQUE;

DEFINE ACCESS OVERWRITE account ON DATABASE TYPE RECORD
    SIGNUP ( CREATE user SET email = $email, password = crypto::argon2::generate($password) )
    SIGNIN ( SELECT * FROM user WHERE email = $email AND crypto::argon2::compare(password, $password) )
    DURATION FOR TOKEN 15m, FOR SESSION 12h;
