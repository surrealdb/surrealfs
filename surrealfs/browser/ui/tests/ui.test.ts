import { describe, expect, test } from "bun:test";
import {
    ApiError,
    SESSIONS,
    api,
    formatSize,
    hasPreview,
    isJson,
    isText,
    json,
    raw,
    send,
} from "../src/api";
import { isDir, toTreeData } from "../src/tree";
import type { Entry } from "../src/types";

describe("Browser UI API & Content-Type Helpers", () => {
    test("raw URL encodes path query parameter", () => {
        expect(raw("/notes/hello world.md")).toBe("/raw?path=%2Fnotes%2Fhello%20world.md");
        expect(raw("/a+b&c=d.txt")).toBe("/raw?path=%2Fa%2Bb%26c%3Dd.txt");
    });

    test("isText, isJson, and hasPreview predicates", () => {
        expect(isText("text/plain")).toBe(true);
        expect(isText("text/markdown")).toBe(true);
        expect(isText("application/json")).toBe(true);
        expect(isText("application/ld+json")).toBe(true);
        expect(isText("application/javascript")).toBe(true);
        expect(isText("application/x-yaml")).toBe(true);
        expect(isText("application/toml")).toBe(true);
        expect(isText("application/x-sh")).toBe(true);
        expect(isText("image/png")).toBe(false);
        expect(isText("application/octet-stream")).toBe(false);

        expect(isJson("application/json")).toBe(true);
        expect(isJson("application/ld+json")).toBe(true);
        expect(isJson("text/markdown")).toBe(false);

        expect(hasPreview("text/markdown")).toBe(true);
        expect(hasPreview("text/html")).toBe(true);
        expect(hasPreview("application/json")).toBe(true);
        expect(hasPreview("text/plain")).toBe(false);
        expect(hasPreview("image/png")).toBe(false);
    });

    test("formatSize formats bytes, KB, and MB", () => {
        expect(formatSize(0)).toBe("0 B");
        expect(formatSize(512)).toBe("512 B");
        expect(formatSize(1023)).toBe("1023 B");
        expect(formatSize(1024)).toBe("1.0 KB");
        expect(formatSize(1536)).toBe("1.5 KB");
        expect(formatSize(1048576)).toBe("1.0 MB");
        expect(formatSize(2621440)).toBe("2.5 MB");
    });

    test("SESSIONS constant points to /_sessions", () => {
        expect(SESSIONS).toBe("/_sessions");
    });

    test("api, json, and send handle success and ApiError responses", async () => {
        const origFetch = globalThis.fetch;
        try {
            let capturedInit: RequestInit | undefined;
            globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
                capturedInit = init;
                const url = String(input);
                if (url === "/api/ok") {
                    return new Response(JSON.stringify({ ok: true, count: 3 }), {
                        status: 200,
                        headers: { "content-type": "application/json" },
                    });
                }
                if (url === "/api/fail-json") {
                    return new Response(JSON.stringify({ error: "Permission denied: /secret" }), {
                        status: 403,
                        statusText: "Forbidden",
                        headers: { "content-type": "application/json" },
                    });
                }
                return new Response("Internal Server Error", {
                    status: 500,
                    statusText: "Internal Server Error",
                });
            }) as typeof fetch;

            const data = await json<{ ok: boolean; count: number }>("/api/ok");
            expect(data).toEqual({ ok: true, count: 3 });

            const sent = await send<{ ok: boolean }>("/api/ok", "PUT", {
                path: "/notes/a.md",
                content: "hi",
            });
            expect(sent.ok).toBe(true);
            expect(capturedInit?.method).toBe("PUT");
            expect(capturedInit?.body).toBe(JSON.stringify({ path: "/notes/a.md", content: "hi" }));

            try {
                await api("/api/fail-json");
                throw new Error("Expected ApiError");
            } catch (err) {
                expect(err).toBeInstanceOf(ApiError);
                const apiErr = err as ApiError;
                expect(apiErr.status).toBe(403);
                expect(apiErr.message).toBe("Permission denied: /secret");
            }

            try {
                await api("/api/fail-plain");
                throw new Error("Expected ApiError");
            } catch (err) {
                expect(err).toBeInstanceOf(ApiError);
                const apiErr = err as ApiError;
                expect(apiErr.status).toBe(500);
                expect(apiErr.message).toBe("Internal Server Error");
            }
        } finally {
            globalThis.fetch = origFetch;
        }
    });
});

describe("FileTree Hierarchy & Sorting (toTreeData)", () => {
    const makeEntry = (
        path: string,
        is_folder = false,
        content_type = is_folder ? "inode/directory" : "text/markdown",
    ): Entry => ({
        path,
        filename: path.split("/").filter(Boolean).pop() ?? "",
        is_folder,
        content_type,
        size: is_folder ? 0 : 42,
        updated_at: "2026-10-17T00:00:00Z",
    });

    test("builds nested tree, synthesises implicit folders, and sorts folders before files", () => {
        const entries: Entry[] = [
            makeEntry("/zeta.md", false),
            makeEntry("/alpha.md", false),
            makeEntry("/docs/guides/getting-started.md", false),
            makeEntry("/docs/api.md", false),
            makeEntry("/empty-dir", true),
        ];

        const tree = toTreeData(entries);
        // Root level should have folders first (/docs, /empty-dir), then files (/alpha.md, /zeta.md)
        expect(tree.map((n) => n.value)).toEqual([
            "/docs",
            "/empty-dir",
            "/alpha.md",
            "/zeta.md",
        ]);

        const docsNode = tree[0]!;
        expect(isDir(docsNode)).toBe(true);
        // /docs was synthesised implicitly (no explicit /docs row in entries)
        expect(docsNode.entry).toBeUndefined();
        // Inside /docs: /docs/guides (folder) comes before /docs/api.md (file)
        expect(docsNode.children.map((n) => n.value)).toEqual([
            "/docs/guides",
            "/docs/api.md",
        ]);

        const guidesNode = docsNode.children[0]!;
        expect(isDir(guidesNode)).toBe(true);
        expect(guidesNode.children.map((n) => n.value)).toEqual([
            "/docs/guides/getting-started.md",
        ]);
        expect(guidesNode.children[0]!.entry?.filename).toBe("getting-started.md");

        const emptyDirNode = tree[1]!;
        expect(isDir(emptyDirNode)).toBe(true);
        expect(emptyDirNode.entry?.is_folder).toBe(true);
        expect(emptyDirNode.children).toEqual([]);
    });
});
