// Cloudflare Worker entrypoint for SurrealFS Layer 2 Wasm Gateway.
// Dispatches `/api/*`, `/raw`, `/mcp`, `/dav/*`, and `/s3/*` to the compiled
// `surrealfs-server` WebAssembly module (`handle_worker_request`) and serves
// the React Web UI static assets via Cloudflare Workers Assets (`env.ASSETS`). File content larger
// than INLINE_MAX_BYTES is streamed between the client and the R2 bucket `env.BLOBS`; SurrealDB is
// asked only whether the caller may.

import initWasm, { handle_worker_request } from "../pkg/surrealfs_server.js";
import wasmModule from "../pkg/surrealfs_server_bg.wasm";

// Mirrors `surrealfs_core::INLINE_MAX_BYTES`: a PUT body longer than this is streamed.
const INLINE_MAX_BYTES = 1024 * 1024;

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

async function workerEnv(env) {
  return {
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
}

export default {
  // Collects the blobs no file names, daily (see `[triggers]` in wrangler.toml).
  async scheduled(_controller, env, _ctx) {
    await ensureWasm();
    const grace = env.SURREALFS_GC_GRACE_SECS || 2 * 24 * 3600;
    const headers = env.SURREALFS_SERVER_TOKEN
      ? { authorization: `Bearer ${env.SURREALFS_SERVER_TOKEN}` }
      : {};
    const result = await handle_worker_request(
      "POST",
      `/api/gc?max_age_secs=${grace}`,
      JSON.stringify(headers),
      new Uint8Array(0),
      null,
      JSON.stringify(await workerEnv(env)),
      env.BLOBS ?? null
    );
    if (result.status !== 200) {
      throw new Error(`blob gc failed with status ${result.status}`);
    }
  },

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

    const length = Number(request.headers.get("content-length") ?? -1);
    const streamed =
      request.method === "PUT" &&
      request.body !== null &&
      (length < 0 || length > INLINE_MAX_BYTES);
    const bodyStream = streamed ? request.body : null;
    const bodyBuffer =
      request.method === "GET" || request.method === "HEAD" || streamed
        ? new Uint8Array(0)
        : new Uint8Array(await request.arrayBuffer());

    const envObj = await workerEnv(env);

    const pathAndQuery = url.pathname + (url.search || "");
    const result = await handle_worker_request(
      request.method,
      pathAndQuery,
      JSON.stringify(headersObj),
      bodyBuffer,
      bodyStream,
      JSON.stringify(envObj),
      env.BLOBS ?? null
    );

    const status = result.status || 200;
    const headers = new Headers(result.headers || {});
    const source = result.stream;
    if (!source) {
      const nullBody = status === 204 || status === 304 || request.method === "HEAD";
      return new Response(nullBody ? null : result.body, { status, headers });
    }

    const body = new ReadableStream({
      async pull(controller) {
        try {
          const piece = await source.next();
          if (piece === undefined) {
            controller.close();
            source.free();
          } else {
            controller.enqueue(piece);
          }
        } catch (error) {
          controller.error(error);
          source.free();
        }
      },
      cancel() {
        source.free();
      },
    });
    return new Response(body, { status, headers });
  },
};
