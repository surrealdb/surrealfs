/**
 * The container's configuration, as one contract.
 *
 * Hand-written rather than left to `wrangler types`: that generates the vars as
 * *literal* types from whatever placeholder values wrangler.jsonc happens to
 * hold, which makes a template repo's generated file both enormous and wrong.
 * `Cloudflare.Env` is the ambient interface `Container` and `getContainer` are
 * written against, so declaring it here is what makes them line up.
 */
declare namespace Cloudflare {
    interface Env {
        BROWSER: DurableObjectNamespace<import("./worker").BrowserContainer>;

        // From `vars` in wrangler.jsonc.
        SURREALFS_SSO_ISSUER: string;
        SURREALFS_SSO_JWKS: string;
        SURREALFS_SSO_AUD: string;
        SURREALDB_URL: string;
        SURREALDB_NAMESPACE: string;
        SURREALDB_DATABASE: string;
        /** Optional: shortens one domain's addresses to their local part. */
        SURREALFS_SSO_DOMAIN?: string;

        // From `wrangler secret put`.
        SURREALFS_SSO_SECRET: string;
        /** Only needed to re-embed, which is root-only. Unset, the app holds
         *  no system credential at all. */
        SURREALDB_USER?: string;
        SURREALDB_PASS?: string;
        OPENAI_API_KEY?: string;
        ANTHROPIC_API_KEY?: string;
    }
}

type Env = Cloudflare.Env;
