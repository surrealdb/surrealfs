import { Container, getContainer } from "@cloudflare/containers";

/**
 * A doorway, not an application: this forwards every request to the container
 * and does nothing else. The Starlette app inside is the same one
 * `surrealfs-browser-sso` runs locally, so there is no second implementation of
 * the API and no filesystem logic written twice.
 *
 * `Env` is declared in env.d.ts.
 */

const PASS_THROUGH = [
    "SURREALFS_SSO_ISSUER",
    "SURREALFS_SSO_JWKS",
    "SURREALFS_SSO_AUD",
    "SURREALFS_SSO_SECRET",
    "SURREALFS_SSO_DOMAIN",
    "SURREALDB_URL",
    "SURREALDB_NAMESPACE",
    "SURREALDB_DATABASE",
    "SURREALDB_USER",
    "SURREALDB_PASS",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
] as const satisfies readonly (keyof Env)[];

export class BrowserContainer extends Container {
    defaultPort = 7933;
    // Long enough that reading a file does not cost a cold start on the next
    // click, short enough not to idle all night.
    sleepAfter = "10m";

    // A field initializer rather than a constructor: these run after the base
    // constructor, so `this.env` is populated and ours is the assignment that
    // sticks. Nothing is passed down that was not set, so an unset optional
    // stays unset inside the container instead of becoming the string
    // "undefined".
    envVars = Object.fromEntries(
        PASS_THROUGH.filter((name) => this.env[name] !== undefined).map((name) => [
            name,
            String(this.env[name]),
        ]),
    );
}

export default {
    async fetch(request: Request, env: Env): Promise<Response> {
        // Straight through, headers intact. `Cf-Access-Jwt-Assertion` is the one
        // that matters: Access adds it at the edge and the Python app verifies
        // it itself, so reaching the container by another route is not a way
        // past the login.
        return getContainer(env.BROWSER).fetch(request);
    },
} satisfies ExportedHandler<Env>;
