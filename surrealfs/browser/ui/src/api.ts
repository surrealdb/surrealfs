/** Fetch helpers and the content-type predicates the viewer branches on. */

export class ApiError extends Error {
    constructor(
        message: string,
        readonly status: number,
    ) {
        super(message);
    }
}

export async function api(url: string, options?: RequestInit): Promise<Response> {
    const res = await fetch(url, options);
    if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        throw new ApiError(body.error || res.statusText, res.status);
    }
    return res;
}

export const json = <T,>(url: string, options?: RequestInit): Promise<T> =>
    api(url, options).then((r) => r.json() as Promise<T>);

/** POST/PUT a JSON body. */
export const send = <T,>(url: string, method: string, body: unknown): Promise<T> =>
    json<T>(url, {
        method,
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
    });

export const raw = (path: string) => `/raw?path=${encodeURIComponent(path)}`;

export const isText = (ct: string) =>
    ct.startsWith("text/") || /(json|javascript|xml|yaml|toml|x-sh)/.test(ct);
// application/json, and application/ld+json and friends.
export const isJson = (ct: string) => ct.endsWith("json");
export const hasPreview = (ct: string) =>
    ct === "text/markdown" || ct === "text/html" || isJson(ct);

export const formatSize = (n: number) =>
    n < 1024
        ? `${n} B`
        : n < 1048576
          ? `${(n / 1024).toFixed(1)} KB`
          : `${(n / 1048576).toFixed(1)} MB`;

/** Conversations are files under here, so the tree doubles as the session list. */
export const SESSIONS = "/_sessions";
