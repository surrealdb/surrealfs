import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { type Subprocess, spawn } from "bun";
import { Surreal } from "surrealdb";
import {
  ConflictError,
  NotFoundError,
  PermissionDeniedError,
  SCHEMA_SQL,
  SurrealFs,
  applySchema,
  createSurrealFsTools,
  fileName,
  globToRegex,
  normalize,
  parentPath,
} from "../src/index.js";

describe("SurrealFS TypeScript SDK", () => {
  describe("Schema Parity", () => {
    test("SCHEMA_SQL matches surrealfs/schema/file.surql byte-for-byte", () => {
      const canonicalPath = resolve(
        import.meta.dir,
        "../../../surrealfs/schema/file.surql"
      );
      const canonical = readFileSync(canonicalPath, "utf8");
      expect(SCHEMA_SQL).toBe(canonical);
    });
  });

  describe("Path Utilities", () => {
    test("normalize handles relative, trailing slashes, and traversal", () => {
      expect(normalize("foo/bar")).toBe("/foo/bar");
      expect(normalize("/foo/bar/")).toBe("/foo/bar");
      expect(normalize("/foo/../bar")).toBe("/bar");
      expect(normalize("///")).toBe("/");
      expect(normalize("")).toBe("/");
    });

    test("parentPath and fileName extract path components", () => {
      expect(parentPath("/a/b/c.txt")).toBe("/a/b");
      expect(parentPath("/a")).toBe("/");
      expect(fileName("/a/b/c.txt")).toBe("c.txt");
      expect(fileName("/")).toBe("");
    });

    test("globToRegex matches file patterns", () => {
      const re = globToRegex("/notes/*.md");
      expect(re.test("/notes/today.md")).toBe(true);
      expect(re.test("/notes/sub/today.md")).toBe(false);
      const reRec = globToRegex("/notes/**");
      expect(reRec.test("/notes/a/b/c.txt")).toBe(true);
    });
  });

  describe("Integration Tests against SurrealDB", () => {
    let proc: Subprocess | null = null;
    let db: Surreal;
    let fs: SurrealFs;
    const port = Math.floor(20000 + Math.random() * 10000);
    const url = `ws://127.0.0.1:${port}/rpc`;

    beforeAll(async () => {
      // Spawn throwaway SurrealDB 3.x in memory
      proc = spawn({
        cmd: [
          "surreal",
          "start",
          "--allow-all",
          "-u",
          "root",
          "-p",
          "root",
          "--bind",
          `127.0.0.1:${port}`,
          "memory",
        ],
        stdout: "ignore",
        stderr: "ignore",
      });

      // Poll until server is ready
      let ready = false;
      const start = Date.now();
      while (Date.now() - start < 15000) {
        try {
          const res = await fetch(`http://127.0.0.1:${port}/health`);
          if (res.ok) {
            ready = true;
            break;
          }
        } catch {
          // keep waiting
        }
        await new Promise((r) => setTimeout(r, 100));
      }

      if (!ready) {
        throw new Error(`SurrealDB failed to start on port ${port} within 15s`);
      }

      db = new Surreal();
      await db.connect(url);
      await db.signin({ username: "root", password: "root" });
      await db.use({ namespace: "test", database: "test" });
      await applySchema(db);

      fs = new SurrealFs(db, { user: "root" });
    });

    afterAll(async () => {
      try {
        await db.close();
      } catch {
        // ignore
      }
      if (proc) {
        proc.kill();
      }
    });

    test("writeText and readText roundtrip", async () => {
      const entry = await fs.writeText("/notes/hello.md", "# Hello World\n");
      expect(entry.path).toBe("/notes/hello.md");
      expect(entry.generation).toBe(1);

      const content = await fs.readText("/notes/hello.md");
      expect(content).toBe("# Hello World\n");
    });

    test("stat and exists", async () => {
      expect(await fs.exists("/notes/hello.md")).toBe(true);
      expect(await fs.exists("/notes/ghost.md")).toBe(false);

      const stat = await fs.stat("/notes/hello.md");
      expect(stat.filename).toBe("hello.md");
      expect(stat.isFolder).toBe(false);
    });

    test("appendText and editText", async () => {
      await fs.appendText("/notes/hello.md", "Second line\n");
      let content = await fs.readText("/notes/hello.md");
      expect(content).toContain("Second line");

      await fs.editText("/notes/hello.md", "Second line", "Edited line");
      content = await fs.readText("/notes/hello.md");
      expect(content).toContain("Edited line");
      expect(content).not.toContain("Second line");
    });

    test("mkdir, ls, glob, and tree", async () => {
      await fs.mkdir("/projects/sub", { parents: true });
      await fs.writeText("/projects/sub/a.txt", "A");
      await fs.writeText("/projects/sub/b.txt", "B");

      const list = await fs.ls("/projects/sub");
      expect(list.length).toBe(2);
      expect(list.map((e) => e.filename).sort()).toEqual(["a.txt", "b.txt"]);

      const globbed = await fs.glob("/projects/**/*.txt");
      expect(globbed.map((e) => e.path).sort()).toEqual([
        "/projects/sub/a.txt",
        "/projects/sub/b.txt",
      ]);

      const tree = await fs.tree("/projects");
      expect(tree).toContain("/projects/sub");
      expect(tree).toContain("/projects/sub/a.txt");
    });

    test("mv and cp", async () => {
      await fs.writeText("/tmp/original.txt", "move me");
      await fs.mv("/tmp/original.txt", "/tmp/moved.txt");
      expect(await fs.exists("/tmp/original.txt")).toBe(false);
      expect(await fs.readText("/tmp/moved.txt")).toBe("move me");

      await fs.cp("/tmp/moved.txt", "/tmp/copied.txt");
      expect(await fs.exists("/tmp/moved.txt")).toBe(true);
      expect(await fs.readText("/tmp/copied.txt")).toBe("move me");
    });

    test("rm and undelete", async () => {
      await fs.writeText("/trash/file.txt", "recover me");
      await fs.rm("/trash/file.txt");
      expect(await fs.exists("/trash/file.txt")).toBe(false);

      const recovered = await fs.undelete("/trash/file.txt");
      expect(recovered.path).toBe("/trash/file.txt");
      expect(await fs.readText("/trash/file.txt")).toBe("recover me");

      await fs.mkdir("/trash/dir", { parents: true });
      await fs.writeText("/trash/dir/inner.txt", "inner");
      await fs.rm("/trash/dir", { recursive: true });
      expect(await fs.exists("/trash/dir")).toBe(false);
    });

    test("history, diff, and restore", async () => {
      await fs.writeText("/history/doc.txt", "version 1\n");
      await fs.writeText("/history/doc.txt", "version 2\n");
      await fs.writeText("/history/doc.txt", "version 3\n");

      const hist = await fs.history("/history/doc.txt");
      expect(hist.length).toBeGreaterThanOrEqual(2);
      expect(hist[0].size).toBeGreaterThan(0);
      expect(hist[0].op).toBeDefined();

      const diffText = await fs.diff("/history/doc.txt", 1, 2);
      expect(diffText).toContain("-version 1");
      expect(diffText).toContain("+version 2");

      await fs.restore("/history/doc.txt", 1);
      const restored = await fs.readText("/history/doc.txt");
      expect(restored).toBe("version 1\n");
    });

    test("chmod, chgrp, and immutable chown", async () => {
      await fs.writeText("/perms/secret.txt", "top secret");
      const modEntry = await fs.chmod("/perms/secret.txt", 0o640);
      expect(modEntry.mode).toBe(0o640);

      const grpEntry = await fs.chgrp("/perms/secret.txt", "eng");
      expect(grpEntry.group).toBe("eng");

      let chownThrew = false;
      try {
        await fs.chown("/perms/secret.txt", "bob");
      } catch (err) {
        chownThrew = true;
        expect(err instanceof PermissionDeniedError).toBe(true);
      }
      expect(chownThrew).toBe(true);
    });

    test("writeBytes and readBytes roundtrip", async () => {
      const payload = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
      const entry = await fs.writeBytes("/bin/logo.png", payload, {
        contentType: "image/png",
      });
      expect(entry.path).toBe("/bin/logo.png");
      expect(entry.size).toBe(payload.byteLength);

      const out = await fs.readBytes("/bin/logo.png");
      expect(Array.from(out)).toEqual(Array.from(payload));
    });

    test("relate and backlinks", async () => {
      await fs.writeText("/graph/source.md", "See target");
      await fs.writeText("/graph/target.md", "Target doc");

      await fs.relate("/graph/source.md", "links_to", "/graph/target.md");
      const links = await fs.backlinks("/graph/target.md");
      expect(links.length).toBeGreaterThanOrEqual(1);
      expect(links[0].sourcePath).toBe("/graph/source.md");
      expect(links[0].relation).toBe("links_to");
    });

    test("search and searchSections", async () => {
      // Fill corpus so BM25 IDF is positive
      for (let i = 0; i < 8; i++) {
        await fs.writeText(`/corpus/filler-${i}.md`, `Background document number ${i}`);
      }
      await fs.writeText(
        "/corpus/quantum.md",
        "Quantum entanglement telemetry protocol specifications for orbital links."
      );

      const hits = await fs.search("entanglement telemetry");
      expect(hits.some((h) => h.path === "/corpus/quantum.md")).toBe(true);
      const hit = hits.find((h) => h.path === "/corpus/quantum.md")!;
      expect(hit.snippet).toContain("entanglement");

      // Insert a 1536-dim section and query via searchSections
      const vec = Array.from({ length: 1536 }, (_, i) => (i === 0 ? 1.0 : 0.0));
      const stat = await fs.stat("/corpus/quantum.md");
      await db.query(
        `CREATE file_section CONTENT {
          file_id: $fid,
          section_idx: 0,
          heading: 'Quantum Protocol',
          content: 'Orbital entanglement telemetry details',
          line_start: 1,
          line_end: 5,
          source_hash: 'h1',
          embedding: $vec
        };`,
        { fid: stat.id, vec }
      );

      const secHits = await fs.searchSections(vec, {
        limit: 5,
        pathPrefix: "/corpus",
      });
      expect(secHits.length).toBeGreaterThanOrEqual(1);
      expect(secHits[0].path).toBe("/corpus/quantum.md");
      expect(secHits[0].startLine).toBe(1);
      expect(secHits[0].endLine).toBe(5);
      expect(secHits[0].score).toBeGreaterThan(0.9);
    });

    test("advisory locking and conflict", async () => {
      const fsAlice = new SurrealFs(db, { user: "alice" });
      const fsBob = new SurrealFs(db, { user: "bob" });

      const lockPath = "/shared/spec.md";
      await fsAlice.writeText(lockPath, "# Spec");
      await fsAlice.chmod(lockPath, 0o666);

      const lease = await fsAlice.acquireLock(lockPath, {
        ttlSeconds: 30,
        reason: "Authoring",
      });
      expect(lease.holder).toBe("alice");

      // Bob should receive ConflictError
      let bobFailed = false;
      try {
        await fsBob.acquireLock(lockPath, { ttlSeconds: 30 });
      } catch (err) {
        bobFailed = true;
        expect(err instanceof ConflictError).toBe(true);
      }
      expect(bobFailed).toBe(true);

      // Alice releases lock
      await fsAlice.releaseLock(lockPath);

      // Now Bob can lock
      const bobLease = await fsBob.acquireLock(lockPath, { ttlSeconds: 30 });
      expect(bobLease.holder).toBe("bob");
      await fsBob.releaseLock(lockPath);
    });

    test("zero-copy workspace fork, diff (modified + newly created), and merge", async () => {
      await fs.writeText("/repo/main.ts", "console.log('main');");

      // Fork workspace
      const fork = await fs.forkWorkspace("feature-ts", "main");
      expect(fork.workspace).toBe("feature-ts");

      const workspaces = await fs.listWorkspaces();
      expect(workspaces.some((ws) => ws.name === "feature-ts")).toBe(true);

      // Modify existing file AND create a brand-new file in workspace
      await fs.writeWorkspaceText(
        "feature-ts",
        "/repo/main.ts",
        "console.log('feature');"
      );
      await fs.writeWorkspaceText(
        "feature-ts",
        "/repo/new_helper.ts",
        "export const helper = 42;"
      );

      const diff = await fs.diffWorkspace("feature-ts");
      const modified = diff.filter((d) => d.modified);
      expect(modified.some((d) => d.path === "/repo/main.ts")).toBe(true);
      expect(
        modified.some(
          (d) => d.path === "/repo/new_helper.ts" && d.status === "added"
        )
      ).toBe(true);

      // Merge workspace
      const merge = await fs.mergeWorkspace("feature-ts", "main");
      expect(merge.status).toBe("ok");

      expect(await fs.readText("/repo/main.ts")).toBe("console.log('feature');");
      expect(await fs.readText("/repo/new_helper.ts")).toBe(
        "export const helper = 42;"
      );
    });

    test("agent actor mailbox FIFO claim and complete", async () => {
      const agentId = "agent-42";
      await fs.sendMessage(agentId, "task-1.json", '{"id": 1}');
      await fs.sendMessage(agentId, "task-2.json", '{"id": 2}');

      const pending = await fs.receiveMessages(agentId);
      expect(pending.length).toBe(2);
      expect(pending[0].filename).toBe("task-1.json");

      const claimed = await fs.claimMessage(agentId, "task-1.json", "worker-A");
      expect(claimed.path).toContain(".claimed/worker-A/task-1.json");

      await fs.completeMessage(agentId, "task-1.json", "worker-A");
      expect(await fs.exists(claimed.path)).toBe(false);
    });

    test("AI tools adapter executes correctly", async () => {
      const tools = createSurrealFsTools(fs);
      expect(tools.ls).toBeDefined();
      expect(tools.read_file).toBeDefined();
      expect(tools.write_file).toBeDefined();
      expect(tools.cp).toBeDefined();
      expect(tools.chmod).toBeDefined();
      expect(tools.restore).toBeDefined();
      expect(tools.undelete).toBeDefined();
      expect(tools.relate).toBeDefined();
      expect(tools.backlinks).toBeDefined();

      const writeRes = await tools.write_file.execute({
        path: "/ai/test.txt",
        content: "generated by agent",
      });
      expect(writeRes.path).toBe("/ai/test.txt");

      const readRes = await tools.read_file.execute({ path: "/ai/test.txt" });
      expect(readRes).toBe("generated by agent");

      const lsRes = await tools.ls.execute({ path: "/ai" });
      expect(lsRes.some((e: any) => e.filename === "test.txt")).toBe(true);
    });

    test("grep searches lines by text pattern and regex", async () => {
      await fs.writeText(
        "/code/app.ts",
        "const port = 8080;\nconst host = 'localhost';\nconsole.log(port);"
      );
      await fs.writeText(
        "/code/server.py",
        "port = 8080\nhost = '0.0.0.0'\nprint(port)"
      );

      const matches = await fs.grep("8080", { pathPrefix: "/code" });
      expect(matches.length).toBe(2);
      expect(
        matches.some((m) => m.path === "/code/app.ts" && m.lineNumber === 1)
      ).toBe(true);
      expect(
        matches.some((m) => m.path === "/code/server.py" && m.lineNumber === 1)
      ).toBe(true);

      const tsOnly = await fs.grep("port", { pathPrefix: "/code", glob: "*.ts" });
      expect(tsOnly.length).toBe(2);
      expect(tsOnly.every((m) => m.path.endsWith(".ts"))).toBe(true);

      const tools = createSurrealFsTools(fs);
      expect(tools.grep).toBeDefined();
      const toolHits = await tools.grep.execute({ pattern: "localhost" });
      expect(toolHits.length).toBe(1);
      expect(toolHits[0].path).toBe("/code/app.ts");
    });

    test("transparent CRDT collaboration and compaction", async () => {
      await fs.writeText("/collab/notes.md", "# Meeting Notes\n\n- Point 1\n");
      await fs.enableCrdt("/collab/notes.md");

      const stat1 = await fs.stat("/collab/notes.md");
      expect(stat1.crdt).toBe(true);

      // Append text under CRDT
      await fs.appendText("/collab/notes.md", "- Point 2\n");
      const read1 = await fs.readText("/collab/notes.md");
      expect(read1).toBe("# Meeting Notes\n\n- Point 1\n- Point 2\n");

      // Edit text under CRDT
      await fs.editText("/collab/notes.md", "- Point 1", "- Item 1 (Approved)");
      const read2 = await fs.readText("/collab/notes.md");
      expect(read2).toBe("# Meeting Notes\n\n- Item 1 (Approved)\n- Point 2\n");

      // Compact CRDT
      await fs.compactCrdt("/collab/notes.md");
      const read3 = await fs.readText("/collab/notes.md");
      expect(read3).toBe("# Meeting Notes\n\n- Item 1 (Approved)\n- Point 2\n");
    });
  });
});
