#!/usr/bin/env bun
/**
 * SurrealFS Native Bun Browser Server
 * Ultra-fast native TypeScript server for the SurrealFS Web UI & Spatial Brain Studio 2.0.
 */

import { resolve, join } from "node:path";
import { existsSync } from "node:fs";
import { Surreal } from "surrealdb";
import { SurrealFs, type FileEntry, normalize } from "./index.js";

// CLI options & Environment configuration
const args = process.argv.slice(2);
function getArg(flag: string, envName: string, def: string): string {
  const idx = args.indexOf(flag);
  if (idx !== -1 && args[idx + 1]) return args[idx + 1];
  return process.env[envName] || def;
}

const PORT = parseInt(getArg("--port", "PORT", "7933"), 10);
const HOST = getArg("--host", "HOST", "127.0.0.1");
const DB_URL = getArg("--url", "SURREALDB_URL", "ws://127.0.0.1:8000/rpc");
const NS = getArg("--ns", "SURREALDB_NAMESPACE", "surrealfs");
const DB = getArg("--db", "SURREALDB_DATABASE", "surrealfs");
const USER = getArg("--user", "SURREALDB_USER", "root");
const PASS = getArg("--pass", "SURREALDB_PASS", "root");

// Static assets directory
const STATIC_DIR = resolve(import.meta.dir, "../../../surrealfs/browser/static");

function entryJson(e: FileEntry) {
  return {
    path: e.path,
    filename: e.filename,
    is_folder: e.isFolder,
    content_type: e.contentType,
    size: e.size,
    updated_at: e.updatedAt ? new Date(e.updatedAt).toISOString() : "",
  };
}

async function startServer() {
  if (!existsSync(join(STATIC_DIR, "index.html"))) {
    console.error(`Browser UI assets not found at: ${STATIC_DIR}`);
    console.error("Please run: cd surrealfs/browser/ui && bun run build");
    process.exit(1);
  }

  const db = new Surreal();
  try {
    await db.connect(DB_URL);
    await db.signin({ username: USER, password: PASS });
    await db.use({ namespace: NS, database: DB });
  } catch (err) {
    console.error(`Failed to connect to SurrealDB at ${DB_URL}:`, err);
    process.exit(1);
  }

  const fs = new SurrealFs(db, { user: USER });

  const server = Bun.serve({
    port: PORT,
    hostname: HOST,
    async fetch(req) {
      const url = new URL(req.url);
      const pathname = url.pathname;
      const method = req.method;

      // Handle CORS
      if (method === "OPTIONS") {
        return new Response(null, {
          headers: {
            "Access-Control-Allow-Origin": "*",
            "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
            "Access-Control-Allow-Headers": "Content-Type",
          },
        });
      }

      try {
        // API: Tree
        if (pathname === "/api/tree" && method === "GET") {
          const entries = await fs.lsRecursive("/");
          return Response.json(entries.map(entryJson));
        }

        // API: Raw content preview
        if (pathname === "/raw" && method === "GET") {
          const path = url.searchParams.get("path");
          if (!path) return Response.json({ error: "missing 'path' parameter" }, { status: 400 });
          const entry = await fs.stat(path);
          const content = await fs.readText(path);
          return new Response(content, {
            headers: {
              "content-type": entry.contentType || "text/plain",
              "X-Content-Type-Options": "nosniff",
              "Content-Security-Policy": "sandbox",
            },
          });
        }

        // API: File operations
        if (pathname === "/api/file") {
          if (method === "PUT") {
            const body = await req.json() as { path: string; content: string };
            const saved = await fs.writeText(body.path, body.content);
            return Response.json(entryJson(saved));
          }

          if (method === "POST") {
            const body = await req.json() as { path: string; folder?: boolean };
            if (await fs.exists(body.path)) {
              return Response.json({ error: `Already exists: ${body.path}` }, { status: 409 });
            }
            const entry = body.folder
              ? await fs.mkdir(body.path, { parents: true })
              : await fs.writeText(body.path, "");
            return Response.json(entryJson(entry));
          }

          if (method === "DELETE") {
            const path = url.searchParams.get("path");
            if (!path) return Response.json({ error: "missing 'path' parameter" }, { status: 400 });
            const recursive = url.searchParams.get("recursive") === "1";
            await fs.rm(path, { recursive });
            return Response.json({ removed: 1 });
          }
        }

        // API: Move / Rename
        if (pathname === "/api/move" && method === "POST") {
          const body = await req.json() as { src: string; dst: string };
          const entry = await fs.mv(body.src, body.dst);
          return Response.json(entryJson(entry));
        }

        // API: Search
        if (pathname === "/api/search" && method === "GET") {
          const q = url.searchParams.get("q")?.trim() || "";
          if (!q) return Response.json({ hybrid: false, results: [] });
          const hits = await fs.search(q, { limit: 20 });
          const results = hits.map((h) => ({
            path: h.path,
            filename: h.path.split("/").pop() || "",
            is_folder: false,
            content_type: "text/markdown",
            size: 0,
            updated_at: h.updatedAt ? new Date(h.updatedAt).toISOString() : "",
            snippet: h.snippet,
            score: h.score,
          }));
          return Response.json({ hybrid: false, results });
        }

        // API: Graph (Spatial Brain Studio 2.0 Canvas)
        if (pathname === "/api/graph" && method === "GET") {
          const entries = await fs.lsRecursive("/");
          const nodes = entries.map((e) => ({
            id: e.path,
            label: e.filename || e.path,
            path: e.path,
            is_folder: e.isFolder,
            size: e.size,
            updated_at: e.updatedAt ? new Date(e.updatedAt).toISOString() : "",
            content_type: e.contentType,
          }));

          const edges: Array<{ source: string; target: string; relation: string }> = [];

          // Relational graph edges
          try {
            const relQuery = `
              LET $ref = (SELECT in.path AS source, out.path AS target, 'references' AS relation FROM references WHERE in.path != NONE AND out.path != NONE);
              LET $sup = (SELECT in.path AS source, out.path AS target, 'supersedes' AS relation FROM supersedes WHERE in.path != NONE AND out.path != NONE);
              LET $der = (SELECT in.path AS source, out.path AS target, 'derives_from' AS relation FROM derives_from WHERE in.path != NONE AND out.path != NONE);
              LET $imp = (SELECT in.path AS source, out.path AS target, 'implements' AS relation FROM implements WHERE in.path != NONE AND out.path != NONE);
              LET $lnk = (SELECT in.path AS source, out.path AS target, 'links_to' AS relation FROM links_to WHERE in.path != NONE AND out.path != NONE);
              RETURN array::flatten([$ref, $sup, $der, $imp, $lnk]);
            `;
            const rels = await (fs as any).queryRaw(relQuery);
            if (Array.isArray(rels)) {
              edges.push(...rels);
            }
          } catch {
            // Optional relations query
          }

          // Containment edges
          const nodeIds = new Set(nodes.map((n) => n.id));
          for (const e of entries) {
            const parts = e.path.replace(/\/$/, "").split("/");
            parts.pop();
            const parent = parts.join("/") || "/";
            if (nodeIds.has(parent) && parent !== e.path) {
              edges.push({ source: parent, target: e.path, relation: "contains" });
            }
          }

          return Response.json({ nodes, edges });
        }

        // API: Swarm Activity & Lock Status
        if (pathname === "/api/activity" && method === "GET") {
          let locks: unknown[] = [];
          let recent_versions: unknown[] = [];
          try {
            const l = await (fs as any).queryRaw(
              "SELECT path, holder, expires_at, reason FROM file_lock WHERE expires_at > time::now();"
            );
            if (Array.isArray(l)) locks = l;
          } catch {}

          try {
            const v = await (fs as any).queryRaw(
              "SELECT path, generation, author, op, created_at, reason FROM file_version ORDER BY created_at DESC LIMIT 30;"
            );
            if (Array.isArray(v)) recent_versions = v;
          } catch {}

          return Response.json({ locks, recent_versions });
        }

        // API: Version History
        if (pathname === "/api/history" && method === "GET") {
          const path = url.searchParams.get("path");
          if (!path) return Response.json([]);
          const hist = await fs.history(path, 50);
          return Response.json(
            hist.map((h: any) => ({
              generation: h.generation,
              author: h.author,
              created_at: h.createdAt ? new Date(h.createdAt).toISOString() : "",
              size: h.size ?? 0,
              op: h.op ?? "write",
              reason: h.reason ?? "",
            }))
          );
        }

        // API: Time Travel Restore
        if (pathname === "/api/restore" && method === "POST") {
          const body = await req.json() as { path: string; generation: number };
          const restored = await fs.restore(body.path, Number(body.generation));
          return Response.json(entryJson(restored));
        }

        // API: Sessions list
        if (pathname === "/api/session" && method === "GET") {
          try {
            const sessions = await fs.ls("/_sessions");
            return Response.json(sessions.map(entryJson));
          } catch {
            return Response.json([]);
          }
        }

        // API: Chat
        if (pathname === "/api/chat" && method === "POST") {
          return Response.json(
            { error: "chat needs ANTHROPIC_API_KEY environment variable" },
            { status: 400 }
          );
        }

        // Static files: Assets or SPA fallback
        if (pathname.startsWith("/assets/")) {
          const assetFile = Bun.file(join(STATIC_DIR, pathname));
          if (await assetFile.exists()) {
            return new Response(assetFile);
          }
        }

        // Check if an exact static file exists
        const maybeFile = Bun.file(join(STATIC_DIR, pathname));
        if (pathname !== "/" && (await maybeFile.exists())) {
          return new Response(maybeFile);
        }

        // Default to index.html for SPA
        return new Response(Bun.file(join(STATIC_DIR, "index.html")), {
          headers: { "content-type": "text/html; charset=utf-8" },
        });
      } catch (err: any) {
        console.error(`Error handling ${method} ${pathname}:`, err);
        return Response.json(
          { error: err?.message || String(err) },
          { status: 500 }
        );
      }
    },
  });

  console.log(`🚀 SurrealFS Browser running on Bun at: http://${HOST}:${PORT}`);
  console.log(`📦 Connected to SurrealDB: ${DB_URL} (${NS}/${DB})`);
}

startServer().catch((err) => {
  console.error("Fatal server error:", err);
  process.exit(1);
});
