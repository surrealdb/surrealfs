import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { type Subprocess, spawn } from "bun";
import { Surreal } from "surrealdb";
import {
  AlreadyExistsError,
  ConflictError,
  DirectoryNotEmptyError,
  InvalidPathError,
  IsADirectoryError,
  NotADirectoryError,
  NotFoundError,
  PermissionDeniedError,
  RECORD_AUTH_SCHEMA,
  SCHEMA_SQL,
  SurrealFs,
  SurrealFsError,
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

    test("RECORD_AUTH_SCHEMA matches surrealfs/schema/record_auth.surql byte-for-byte", () => {
      const canonicalPath = resolve(
        import.meta.dir,
        "../../../surrealfs/schema/record_auth.surql"
      );
      const canonical = readFileSync(canonicalPath, "utf8");
      expect(RECORD_AUTH_SCHEMA).toBe(canonical);
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
      await db.connect(url, { versionCheck: false });
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

    test("POSIX negative paths and error hierarchy", async () => {
      // NotFoundError on missing paths
      await expect(fs.stat("/does/not/exist.txt")).rejects.toBeInstanceOf(
        NotFoundError
      );
      await expect(fs.readText("/does/not/exist.txt")).rejects.toBeInstanceOf(
        NotFoundError
      );
      await expect(fs.readBytes("/does/not/exist.txt")).rejects.toBeInstanceOf(
        NotFoundError
      );
      await expect(fs.rm("/does/not/exist.txt")).rejects.toBeInstanceOf(
        NotFoundError
      );
      await expect(
        fs.mv("/does/not/exist.txt", "/does/not/dst.txt")
      ).rejects.toBeInstanceOf(NotFoundError);
      await expect(
        fs.cp("/does/not/exist.txt", "/does/not/dst.txt")
      ).rejects.toBeInstanceOf(NotFoundError);
      await expect(
        fs.chmod("/does/not/exist.txt", 0o600)
      ).rejects.toBeInstanceOf(NotFoundError);
      await expect(
        fs.chgrp("/does/not/exist.txt", "eng")
      ).rejects.toBeInstanceOf(NotFoundError);
      await expect(
        fs.undelete("/does/not/exist.txt")
      ).rejects.toBeInstanceOf(NotFoundError);

      // Setup directory and file for type mismatch errors
      await fs.mkdir("/posix/dir", { parents: true });
      await fs.writeText("/posix/dir/child.txt", "child content");

      // IsADirectoryError when reading/writing a directory as a file
      await expect(fs.readText("/posix/dir")).rejects.toBeInstanceOf(
        IsADirectoryError
      );
      await expect(fs.writeText("/posix/dir", "overwrite")).rejects.toBeInstanceOf(
        IsADirectoryError
      );
      await expect(fs.appendText("/posix/dir", "more")).rejects.toBeInstanceOf(
        IsADirectoryError
      );
      await expect(
        fs.writeBytes("/posix/dir", new Uint8Array([1, 2, 3]))
      ).rejects.toBeInstanceOf(IsADirectoryError);

      // NotADirectoryError when listing a regular file
      await expect(fs.ls("/posix/dir/child.txt")).rejects.toBeInstanceOf(
        NotADirectoryError
      );

      // DirectoryNotEmptyError when removing non-empty folder without recursive: true
      await expect(
        fs.rm("/posix/dir", { recursive: false })
      ).rejects.toBeInstanceOf(DirectoryNotEmptyError);

      // mkdir without parents: true when intermediate parent does not exist
      await expect(
        fs.mkdir("/posix/missing_parent/leaf", { parents: false })
      ).rejects.toBeInstanceOf(NotFoundError);

      // mkdir on a path that already exists as a regular file
      await expect(
        fs.mkdir("/posix/dir/child.txt", { parents: false })
      ).rejects.toBeInstanceOf(AlreadyExistsError);

      // Moving a folder into its own subtree is rejected
      await expect(
        fs.mv("/posix/dir", "/posix/dir/sub/into_self")
      ).rejects.toBeInstanceOf(InvalidPathError);

      // editText negative cases: empty oldText, missing pattern, ambiguous multi-match
      await expect(
        fs.editText("/posix/dir/child.txt", "", "x")
      ).rejects.toBeInstanceOf(InvalidPathError);
      await expect(
        fs.editText("/posix/dir/child.txt", "nonexistent_substring", "x")
      ).rejects.toBeInstanceOf(ConflictError);

      await fs.writeText("/posix/dir/dup.txt", "repeat repeat");
      await expect(
        fs.editText("/posix/dir/dup.txt", "repeat", "once", {
          replaceAll: false,
        })
      ).rejects.toBeInstanceOf(ConflictError);
      const replacedAll = await fs.editText(
        "/posix/dir/dup.txt",
        "repeat",
        "once",
        { replaceAll: true }
      );
      expect(replacedAll.generation).toBe(2);
      expect(await fs.readText("/posix/dir/dup.txt")).toBe("once once");
    });

    test("lsRecursive lists all nested entries", async () => {
      await fs.mkdir("/tree_rec/a/b", { parents: true });
      await fs.writeText("/tree_rec/a/one.txt", "1");
      await fs.writeText("/tree_rec/a/b/two.txt", "2");

      const all = await fs.lsRecursive("/tree_rec");
      const paths = all.map((e) => e.path);
      expect(paths).toContain("/tree_rec/a");
      expect(paths).toContain("/tree_rec/a/one.txt");
      expect(paths).toContain("/tree_rec/a/b");
      expect(paths).toContain("/tree_rec/a/b/two.txt");
    });

    test("optimistic concurrency (ifGeneration) across writeText, appendText, editText, and writeBytes", async () => {
      // writeText with ifGeneration on non-existent file throws ConflictError
      await expect(
        fs.writeText("/occ/doc.txt", "gen1", { ifGeneration: 1 })
      ).rejects.toBeInstanceOf(ConflictError);

      const created = await fs.writeText("/occ/doc.txt", "gen1");
      expect(created.generation).toBe(1);

      // writeText with wrong generation fails
      await expect(
        fs.writeText("/occ/doc.txt", "bad", { ifGeneration: 99 })
      ).rejects.toBeInstanceOf(ConflictError);
      expect(await fs.readText("/occ/doc.txt")).toBe("gen1");

      // writeText with matching generation succeeds
      const updated = await fs.writeText("/occ/doc.txt", "gen2", {
        ifGeneration: 1,
      });
      expect(updated.generation).toBe(2);

      // appendText with wrong generation fails, matching succeeds
      await expect(
        fs.appendText("/occ/doc.txt", "!", { ifGeneration: 1 })
      ).rejects.toBeInstanceOf(ConflictError);
      const appended = await fs.appendText("/occ/doc.txt", "!", {
        ifGeneration: 2,
      });
      expect(appended.generation).toBe(3);
      expect(await fs.readText("/occ/doc.txt")).toBe("gen2!");

      // editText with wrong generation fails, matching succeeds
      await expect(
        fs.editText("/occ/doc.txt", "gen2!", "gen4", { ifGeneration: 2 })
      ).rejects.toBeInstanceOf(ConflictError);
      const edited = await fs.editText("/occ/doc.txt", "gen2!", "gen4", {
        ifGeneration: 3,
      });
      expect(edited.generation).toBe(4);
      expect(await fs.readText("/occ/doc.txt")).toBe("gen4");

      // writeBytes with ifGeneration on non-existent file fails, then succeeds on existing
      await expect(
        fs.writeBytes("/occ/bin.dat", new Uint8Array([1, 2]), {
          ifGeneration: 1,
        })
      ).rejects.toBeInstanceOf(ConflictError);
      const b1 = await fs.writeBytes("/occ/bin.dat", new Uint8Array([1, 2]));
      expect(b1.generation).toBe(1);
      await expect(
        fs.writeBytes("/occ/bin.dat", new Uint8Array([3, 4]), {
          ifGeneration: 9,
        })
      ).rejects.toBeInstanceOf(ConflictError);
      const b2 = await fs.writeBytes(
        "/occ/bin.dat",
        new Uint8Array([3, 4]),
        { ifGeneration: 1 }
      );
      expect(b2.generation).toBe(2);
    });

    test("multi-user POSIX permissions and group ACL enforcement", async () => {
      const fsAlice = new SurrealFs(db, { user: "alice" });
      const fsBob = new SurrealFs(db, { user: "bob" });
      const fsCarol = new SurrealFs(db, { user: "carol" });

      // Root creates /acl shared directory (0o777) so alice can create files inside
      await fs.mkdir("/acl", { parents: true });
      await fs.chmod("/acl", 0o777);

      // Alice creates a private file (0o600)
      await fsAlice.writeText("/acl/secret.txt", "alice eyes only");
      await fsAlice.chmod("/acl/secret.txt", 0o600);

      // Bob cannot read, write, append, edit, chmod, or rm Alice's 0o600 file
      await expect(fsBob.readText("/acl/secret.txt")).rejects.toBeInstanceOf(
        PermissionDeniedError
      );
      await expect(
        fsBob.writeText("/acl/secret.txt", "hacked")
      ).rejects.toBeInstanceOf(PermissionDeniedError);
      await expect(
        fsBob.appendText("/acl/secret.txt", "hacked")
      ).rejects.toBeInstanceOf(PermissionDeniedError);
      await expect(
        fsBob.chmod("/acl/secret.txt", 0o666)
      ).rejects.toBeInstanceOf(PermissionDeniedError);
      await expect(fsBob.rm("/acl/secret.txt")).rejects.toBeInstanceOf(
        PermissionDeniedError
      );

      // Alice sets group to 'research' and mode to 0o640 (group read)
      await db.query(
        `CREATE user_group CONTENT { name: 'research', description: 'Research team' };
         CREATE group_member CONTENT { group_name: 'research', user_id: 'bob' };`
      );
      await fsAlice.chgrp("/acl/secret.txt", "research");
      await fsAlice.chmod("/acl/secret.txt", 0o640);

      // Bob (in group 'research') can now read, but still cannot write; Carol (not in group) cannot read
      expect(await fsBob.readText("/acl/secret.txt")).toBe("alice eyes only");
      await expect(
        fsBob.writeText("/acl/secret.txt", "bob edit")
      ).rejects.toBeInstanceOf(PermissionDeniedError);
      await expect(fsCarol.readText("/acl/secret.txt")).rejects.toBeInstanceOf(
        PermissionDeniedError
      );

      // Alice grants group write (0o660) -> Bob can now write and append, Carol still denied
      await fsAlice.chmod("/acl/secret.txt", 0o660);
      await fsBob.appendText("/acl/secret.txt", " + bob contribution");
      expect(await fsAlice.readText("/acl/secret.txt")).toBe(
        "alice eyes only + bob contribution"
      );
      await expect(
        fsCarol.writeText("/acl/secret.txt", "carol edit")
      ).rejects.toBeInstanceOf(PermissionDeniedError);

      // Home directory anti-squatting: Bob cannot write into /home/alice
      await fsAlice.writeText("/home/alice/notes.md", "my home note");
      await expect(
        fsBob.writeText("/home/alice/intrude.md", "nope")
      ).rejects.toBeInstanceOf(PermissionDeniedError);
    });

    test("record-auth database user login and table-level permission isolation", async () => {
      // Apply record_auth schema
      await applySchema(db, { recordAuth: true });

      // Provision record user 'dave' and 'erin' as admin
      await db.query(
        `UPSERT type::record('user', 'dave') CONTENT { password: crypto::argon2::generate('dave-pass-123') };
         UPSERT type::record('user', 'erin') CONTENT { password: crypto::argon2::generate('erin-pass-123') };`
      );

      // Connect a fresh client signed in as record user 'dave'
      const daveDb = new Surreal();
      await daveDb.connect(url, { versionCheck: false });
      await daveDb.use({ namespace: "test", database: "test" });
      await daveDb.signin({
        namespace: "test",
        database: "test",
        access: "account",
        variables: { user: "dave", pass: "dave-pass-123" },
      });

      const erinDb = new Surreal();
      await erinDb.connect(url, { versionCheck: false });
      await erinDb.use({ namespace: "test", database: "test" });
      await erinDb.signin({
        namespace: "test",
        database: "test",
        access: "account",
        variables: { user: "erin", pass: "erin-pass-123" },
      });

      try {
        const fsDave = new SurrealFs(daveDb, { user: "dave" });
        const fsErin = new SurrealFs(erinDb, { user: "erin" });

        await fsDave.writeText("/home/dave/diary.md", "Dear diary");
        await fsDave.chmod("/home/dave/diary.md", 0o600);
        expect(await fsDave.readText("/home/dave/diary.md")).toBe("Dear diary");

        // Erin is blocked even if she tries to read /home/dave/diary.md
        await expect(
          fsErin.readText("/home/dave/diary.md")
        ).rejects.toBeInstanceOf(SurrealFsError);

        // Even if Erin spoofs caller='dave' in SurrealFs options, fn::sfs_caller($caller) uses $auth ('erin')!
        const fsErinSpoof = new SurrealFs(erinDb, { user: "dave" });
        await expect(
          fsErinSpoof.readText("/home/dave/diary.md")
        ).rejects.toBeInstanceOf(SurrealFsError);
      } finally {
        await daveDb.close();
        await erinDb.close();
      }
    });

    test("search modes (any vs all), searchSemantic, and searchHybrid", async () => {
      await fs.writeText(
        "/search_modes/alpha.md",
        "neural network architecture with transformer attention layers"
      );
      await fs.writeText(
        "/search_modes/beta.md",
        "neural compiler optimization pass for gpu kernels"
      );

      const anyHits = await fs.search("neural transformer", { match: "any" });
      expect(anyHits.some((h) => h.path === "/search_modes/alpha.md")).toBe(
        true
      );
      expect(anyHits.some((h) => h.path === "/search_modes/beta.md")).toBe(
        true
      );

      const allHits = await fs.search("neural transformer", { match: "all" });
      expect(allHits.some((h) => h.path === "/search_modes/alpha.md")).toBe(
        true
      );
      expect(allHits.some((h) => h.path === "/search_modes/beta.md")).toBe(
        false
      );

      // Attach 1536-dim embeddings to alpha.md and beta.md
      const vecAlpha = Array.from({ length: 1536 }, (_, i) =>
        i === 1 ? 1.0 : 0.0
      );
      const vecBeta = Array.from({ length: 1536 }, (_, i) =>
        i === 2 ? 1.0 : 0.0
      );
      const statAlpha = await fs.stat("/search_modes/alpha.md");
      const statBeta = await fs.stat("/search_modes/beta.md");
      await db.query("UPDATE $id SET embedding = $vec;", {
        id: statAlpha.id,
        vec: vecAlpha,
      });
      await db.query("UPDATE $id SET embedding = $vec;", {
        id: statBeta.id,
        vec: vecBeta,
      });

      const semHits = await fs.searchSemantic(vecAlpha, { limit: 5 });
      expect(semHits.length).toBeGreaterThanOrEqual(1);
      expect(semHits[0].path).toBe("/search_modes/alpha.md");

      const hybHits = await fs.searchHybrid("transformer", vecAlpha, {
        limit: 5,
        match: "any",
      });
      expect(hybHits.length).toBeGreaterThanOrEqual(1);
      expect(hybHits[0].path).toBe("/search_modes/alpha.md");
    });

    test("discardWorkspace and listLocks", async () => {
      await fs.writeText("/ws_discard/base.txt", "original");
      await fs.forkWorkspace("temp-branch", "main");
      await fs.writeWorkspaceText("temp-branch", "/ws_discard/base.txt", "changed");

      const discarded = await fs.discardWorkspace("temp-branch");
      expect(discarded.discarded).toBe("temp-branch");
      expect(await fs.readText("/ws_discard/base.txt")).toBe("original");

      const workspaces = await fs.listWorkspaces();
      expect(workspaces.some((w) => w.name === "temp-branch")).toBe(false);

      // listLocks returns active locks
      await fs.acquireLock("/ws_discard/base.txt", {
        ttlSeconds: 60,
        reason: "testing listLocks",
      });
      const locks = await fs.listLocks();
      expect(locks.some((l) => l.path === "/ws_discard/base.txt")).toBe(true);
      await fs.releaseLock("/ws_discard/base.txt");
    });

    test("watch delivers live events and unsubscribes cleanly", async () => {
      // surrealdb.js 2.x gates live queries on a server version below 4.0.
      const { version } = await db.version();
      if (/^surrealdb-([4-9]|\d{2,})\./.test(version)) {
        console.warn(`skipping live query test: surrealdb.js does not support ${version}`);
        return;
      }
      const events: Array<{ action: string; path: string }> = [];
      const sub = await fs.watch("/watched/**", (ev) => {
        events.push({ action: ev.action, path: ev.path });
      });

      try {
        await fs.writeText("/watched/live.txt", "hello live");
        // Allow async live notification to arrive
        const deadline = Date.now() + 2000;
        while (events.length === 0 && Date.now() < deadline) {
          await new Promise((r) => setTimeout(r, 50));
        }
        expect(events.some((e) => e.path === "/watched/live.txt")).toBe(true);
      } finally {
        await sub.unsubscribe();
      }
    });

    test("complete AI toolset execution coverage", async () => {
      const tools = createSurrealFsTools(fs);

      // mkdir + write_file + append_file + edit_file + stat
      await tools.mkdir.execute({ path: "/aitools/sub", parents: true });
      await tools.write_file.execute({
        path: "/aitools/sub/note.md",
        content: "Line 1\n",
      });
      await tools.append_file.execute({
        path: "/aitools/sub/note.md",
        suffix: "Line 2\n",
      });
      await tools.edit_file.execute({
        path: "/aitools/sub/note.md",
        old_text: "Line 2",
        new_text: "Line Two",
      });
      const st = await tools.stat.execute({ path: "/aitools/sub/note.md" });
      expect(st.generation).toBe(3);

      // glob + tree
      const globRes = await tools.glob.execute({ pattern: "/aitools/**/*.md" });
      expect(globRes.map((e: any) => e.path)).toEqual(["/aitools/sub/note.md"]);
      const treeRes = await tools.tree.execute({ path: "/aitools" });
      expect(treeRes).toContain("/aitools/sub/note.md");

      // history + diff + restore
      const hist = await tools.history.execute({ path: "/aitools/sub/note.md" });
      expect(hist.length).toBeGreaterThanOrEqual(2);
      const diffTxt = await tools.diff.execute({
        path: "/aitools/sub/note.md",
        from_gen: 1,
        to_gen: 2,
      });
      expect(diffTxt).toContain("+Line 2");
      await tools.restore.execute({ path: "/aitools/sub/note.md", version: 1 });
      expect(await tools.read_file.execute({ path: "/aitools/sub/note.md" })).toBe(
        "Line 1\n"
      );

      // cp + mv + chmod + rm + undelete
      await tools.cp.execute({
        src: "/aitools/sub/note.md",
        dst: "/aitools/sub/copy.md",
      });
      await tools.mv.execute({
        src: "/aitools/sub/copy.md",
        dst: "/aitools/sub/moved.md",
      });
      const mod = await tools.chmod.execute({
        path: "/aitools/sub/moved.md",
        mode: 0o640,
      });
      expect(mod.mode).toBe(0o640);
      await tools.rm.execute({ path: "/aitools/sub/moved.md" });
      expect(await fs.exists("/aitools/sub/moved.md")).toBe(false);
      await tools.undelete.execute({ path: "/aitools/sub/moved.md" });
      expect(await fs.exists("/aitools/sub/moved.md")).toBe(true);

      // relate + backlinks
      await tools.relate.execute({
        src: "/aitools/sub/moved.md",
        relation: "references",
        dst: "/aitools/sub/note.md",
      });
      const bks = await tools.backlinks.execute({
        path: "/aitools/sub/note.md",
        relation: "references",
      });
      expect(bks.length).toBe(1);

      // search_text + search_semantic + search_hybrid + search_sections
      const stHits = await tools.search_text.execute({
        query: "entanglement",
        limit: 5,
      });
      expect(stHits.length).toBeGreaterThanOrEqual(1);
      const vec = Array.from({ length: 1536 }, (_, i) => (i === 0 ? 1.0 : 0.0));
      const semHits = await tools.search_semantic.execute({
        vector: vec,
        limit: 5,
      });
      expect(Array.isArray(semHits)).toBe(true);
      const hybHits = await tools.search_hybrid.execute({
        query: "entanglement",
        vector: vec,
        limit: 5,
      });
      expect(hybHits.length).toBeGreaterThanOrEqual(1);
      const secHits = await tools.search_sections.execute({
        vector: vec,
        path_prefix: "/corpus",
        limit: 5,
      });
      expect(secHits.length).toBeGreaterThanOrEqual(1);

      // acquire_lease + release_lease
      const lease = await tools.acquire_lease.execute({
        path: "/aitools/sub/note.md",
        ttl_seconds: 30,
        reason: "tool test",
      });
      expect(lease.holder).toBe("root");
      const rel = await tools.release_lease.execute({
        path: "/aitools/sub/note.md",
      });
      expect(rel).toBe(true);

      // fork_workspace + merge_workspace
      const forked = await tools.fork_workspace.execute({
        dst_branch: "ai-branch",
        src_branch: "main",
      });
      expect(forked.workspace).toBe("ai-branch");
      const merged = await tools.merge_workspace.execute({
        branch: "ai-branch",
        target: "main",
      });
      expect(merged.status).toBe("ok");
    });
  });
});
