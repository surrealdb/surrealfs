// Cloudflare Worker entrypoint for SurrealFS Layer 2 Wasm Gateway.
// Dispatches `/api/*`, `/raw`, `/mcp`, `/dav/*`, and `/s3/*` to the compiled
// `surrealfs-server` WebAssembly module (`handle_worker_request`) and serves
// the React Web UI static assets via Cloudflare Workers Assets (`env.ASSETS`).

import initWasm, { handle_worker_request } from "../pkg/surrealfs_server.js";
import wasmModule from "../pkg/surrealfs_server_bg.wasm";

let wasmReady = null;

function ensureWasm() {
  if (!wasmReady) {
    wasmReady = initWasm(wasmModule);
  }
  return wasmReady;
}

function isGatewayRoute(pathname, headers) {
  if (
    pathname === "/api" ||
    pathname.startsWith("/api/") ||
    pathname === "/raw" ||
    pathname === "/mcp" ||
    pathname.startsWith("/mcp/") ||
    pathname === "/dav" ||
    pathname.startsWith("/dav/") ||
    pathname === "/s3" ||
    pathname.startsWith("/s3/")
  ) {
    return true;
  }
  const auth = headers.get("authorization") || "";
  if (
    headers.has("x-amz-content-sha256") ||
    headers.has("x-amz-date") ||
    auth.startsWith("AWS4-HMAC-SHA256")
  ) {
    return true;
  }
  return false;
}

export default {
  async fetch(request, env, _ctx) {
    const url = new URL(request.url);

    if (!isGatewayRoute(url.pathname, request.headers) && env.ASSETS) {
      return env.ASSETS.fetch(request);
    }

    await ensureWasm();

    const headersObj = {};
    for (const [k, v] of request.headers.entries()) {
      headersObj[k.toLowerCase()] = v;
    }

    const bodyBuffer =
      request.method === "GET" || request.method === "HEAD"
        ? new Uint8Array(0)
        : new Uint8Array(await request.arrayBuffer());

    const envObj = {
      SURREALDB_URL: env.SURREALDB_URL || "",
      SURREALDB_NS: env.SURREALDB_NS || "surrealfs",
      SURREALDB_DB: env.SURREALDB_DB || "main",
      SURREALDB_USER: env.SURREALDB_USER || undefined,
      SURREALDB_PASS: env.SURREALDB_PASS || undefined,
      SURREALDB_TOKEN: env.SURREALDB_TOKEN || undefined,
      OPENAI_API_KEY: env.OPENAI_API_KEY || undefined,
      OPENAI_BASE_URL: env.OPENAI_BASE_URL || undefined,
      SURREALFS_EMBED_MODEL: env.SURREALFS_EMBED_MODEL || undefined,
      ANTHROPIC_API_KEY: env.ANTHROPIC_API_KEY || undefined,
      SURREALFS_CHAT_MODEL: env.SURREALFS_CHAT_MODEL || undefined,
      SURREALFS_SERVER_TOKEN: env.SURREALFS_SERVER_TOKEN || undefined,
    };

    const pathAndQuery = url.pathname + (url.search || "");
    const rawResp = await handle_worker_request(
      request.method,
      pathAndQuery,
      JSON.stringify(headersObj),
      bodyBuffer,
      JSON.stringify(envObj)
    );

    const parsed = JSON.parse(rawResp);
    const respHeaders = new Headers(parsed.headers || {});
    const bodyBytes = new Uint8Array(parsed.body || []);

    return new Response(bodyBytes, {
      status: parsed.status || 200,
      headers: respHeaders,
    });
  },
};
