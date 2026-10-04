/**
 * SurrealFS Core Client in TypeScript.
 */

import { type RecordId, Surreal, Table } from "surrealdb";
import {
  ConflictError,
  mapSurrealError,
  NotFoundError,
  SurrealFsError,
} from "./errors.js";
import {
  type FileEntry,
  type FileVersionEntry,
  type GraphRelation,
  type LockInfo,
  parseFileEntry,
  type SearchHit,
  type SectionHit,
  type WatchEvent,
  type WorkspaceDiff,
  type WorkspaceEntry,
} from "./models.js";
import { globToRegex, normalize, parentPath } from "./paths.js";

export interface SurrealFsOptions {
  user?: string;
}

export class SurrealFs {
  readonly db: Surreal;
  readonly user: string;

  constructor(db: Surreal, options: SurrealFsOptions = {}) {
    this.db = db;
    this.user = options.user ?? "root";
  }

  private async queryRaw<T = unknown>(
    sql: string,
    vars: Record<string, unknown> = {}
  ): Promise<T> {
    try {
      const results = await this.db.query<unknown[]>(sql, vars);
      if (Array.isArray(results) && results.length > 0) {
        return results[0] as T;
      }
      return results as unknown as T;
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async stat(path: string): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_stat($path);",
        { path: clean }
      );
      if (!res || (typeof res === "object" && Object.keys(res).length === 0)) {
        throw new NotFoundError(clean);
      }
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async exists(path: string): Promise<boolean> {
    try {
      await this.stat(path);
      return true;
    } catch (err) {
      if (err instanceof NotFoundError) return false;
      throw err;
    }
  }

  async ls(path: string = "/"): Promise<FileEntry[]> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_ls($path);",
        { path: clean }
      );
      if (!Array.isArray(res)) return [];
      return res.map((r) => parseFileEntry(r as Record<string, unknown>));
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async tree(path: string = "/", maxDepth: number = 10): Promise<string[]> {
    const results: string[] = [];
    const walk = async (current: string, depth: number) => {
      if (depth > maxDepth) return;
      const entries = await this.ls(current);
      for (const entry of entries) {
        results.push(entry.path);
        if (entry.isFolder) {
          await walk(entry.path, depth + 1);
        }
      }
    };
    await walk(normalize(path), 1);
    return results;
  }

  async readText(path: string): Promise<string> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_read($path, $caller);",
        { path: clean, caller: this.user }
      );
      if (typeof res === "string") return res;
      if (res && typeof res === "object" && "content" in res) {
        return String((res as { content: unknown }).content ?? "");
      }
      return String(res ?? "");
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async writeText(
    path: string,
    content: string,
    options: { ifGeneration?: number; createParents?: boolean } = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_write($path, $content, $if_gen, $parents, $caller);",
        {
          path: clean,
          content,
          if_gen: options.ifGeneration ?? null,
          parents: options.createParents ?? true,
          caller: this.user,
        }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async appendText(
    path: string,
    suffix: string,
    options: { ifGeneration?: number } = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_append($path, $suffix, $if_gen, $caller);",
        {
          path: clean,
          suffix,
          if_gen: options.ifGeneration ?? null,
          caller: this.user,
        }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async editText(
    path: string,
    oldText: string,
    newText: string,
    options: { ifGeneration?: number } = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_edit($path, $old, $new, $if_gen, $caller);",
        {
          path: clean,
          old: oldText,
          new: newText,
          if_gen: options.ifGeneration ?? null,
          caller: this.user,
        }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async mkdir(
    path: string,
    options: { parents?: boolean } = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_mkdir($path, $parents, $caller);",
        {
          path: clean,
          parents: options.parents ?? true,
          caller: this.user,
        }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async rm(path: string, options: { recursive?: boolean } = {}): Promise<void> {
    const clean = normalize(path);
    try {
      await this.queryRaw<unknown>(
        "RETURN fn::sfs_rm($path, $recursive, $caller);",
        {
          path: clean,
          recursive: options.recursive ?? false,
          caller: this.user,
        }
      );
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async mv(src: string, dst: string): Promise<FileEntry> {
    const cleanSrc = normalize(src);
    const cleanDst = normalize(dst);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_mv($src, $dst, $caller);",
        { src: cleanSrc, dst: cleanDst, caller: this.user }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, `${cleanSrc} -> ${cleanDst}`);
    }
  }

  async cp(
    src: string,
    dst: string,
    options: { recursive?: boolean } = {}
  ): Promise<FileEntry> {
    const cleanSrc = normalize(src);
    const cleanDst = normalize(dst);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_cp($src, $dst, $recursive, $caller);",
        {
          src: cleanSrc,
          dst: cleanDst,
          recursive: options.recursive ?? false,
          caller: this.user,
        }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, `${cleanSrc} -> ${cleanDst}`);
    }
  }

  async chmod(path: string, mode: number): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_chmod($path, $mode, $caller);",
        { path: clean, mode, caller: this.user }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async chown(path: string, owner: string): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_chown($path, $owner, $caller);",
        { path: clean, owner, caller: this.user }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async history(path: string, limit?: number): Promise<FileVersionEntry[]> {
    const clean = normalize(path);
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_history($path, $limit);",
        { path: clean, limit: limit ?? 20 }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        generation: Number(r.generation ?? 1),
        content: String(r.content ?? ""),
        createdAt: r.created_at ?? null,
        author: String(r.author ?? ""),
        hash: r.hash ?? null,
      }));
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async diff(
    path: string,
    genA?: number,
    genB?: number
  ): Promise<string> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_diff($path, $gen_a, $gen_b, $caller);",
        {
          path: clean,
          gen_a: genA ?? null,
          gen_b: genB ?? null,
          caller: this.user,
        }
      );
      return String(res ?? "");
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async restore(path: string, version: number): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_restore($path, $ver, $caller);",
        { path: clean, ver: version, caller: this.user }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async search(
    query: string,
    options: { limit?: number; match?: "any" | "all" } = {}
  ): Promise<SearchHit[]> {
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_search_text($query, $match, $limit, $caller);",
        {
          query,
          match: options.match ?? "any",
          limit: options.limit ?? 20,
          caller: this.user,
        }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        path: String(r.path ?? ""),
        score: Number(r.score ?? 0),
        snippet: String(r.snippet ?? ""),
        generation: Number(r.generation ?? 1),
        updatedAt: r.updated_at ?? null,
      }));
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async searchSections(
    vector: number[],
    options: { limit?: number; pathPrefix?: string } = {}
  ): Promise<SectionHit[]> {
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_search_sections($vector, $limit, $prefix);",
        {
          vector,
          limit: options.limit ?? 10,
          prefix: options.pathPrefix ?? "",
        }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        path: String(r.path ?? ""),
        heading: String(r.heading ?? ""),
        content: String(r.content ?? ""),
        distance: Number(r.dist ?? 0),
        score: Number(r.score ?? 0),
        tokenCount: Number(r.token_count ?? 0),
        startLine: Number(r.start_line ?? 0),
        endLine: Number(r.end_line ?? 0),
      }));
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async relate(
    sourcePath: string,
    relation: string,
    targetPath: string
  ): Promise<void> {
    const cleanSrc = normalize(sourcePath);
    const cleanDst = normalize(targetPath);
    try {
      await this.queryRaw<unknown>(
        "RETURN fn::sfs_relate($src, $rel, $dst, $caller);",
        {
          src: cleanSrc,
          rel: relation,
          dst: cleanDst,
          caller: this.user,
        }
      );
    } catch (err) {
      throw mapSurrealError(err, `${cleanSrc} -> ${cleanDst}`);
    }
  }

  async backlinks(path: string): Promise<GraphRelation[]> {
    const clean = normalize(path);
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_backlinks($path);",
        { path: clean }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        source: String(r.source ?? ""),
        relation: String(r.relation ?? ""),
        target: String(r.target ?? ""),
        createdAt: r.created_at ?? null,
      }));
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async acquireLock(
    path: string,
    options: { ttlSeconds?: number; reason?: string } = {}
  ): Promise<LockInfo> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<any>(
        "RETURN fn::sfs_acquire_lock($path, $holder, $ttl, $reason);",
        {
          path: clean,
          holder: this.user,
          ttl: options.ttlSeconds ?? 60,
          reason: options.reason ?? "advisory lease",
        }
      );
      return {
        holder: String(res?.holder ?? this.user),
        expiresAt: res?.expires_at ?? new Date(Date.now() + 60000),
        reason: res?.reason,
      };
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async releaseLock(path: string): Promise<boolean> {
    const clean = normalize(path);
    try {
      await this.queryRaw<unknown>(
        "RETURN fn::sfs_release_lock($path, $holder);",
        { path: clean, holder: this.user }
      );
      return true;
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async forkWorkspace(
    dstBranch: string,
    srcBranch: string = "main"
  ): Promise<{ workspace: string; count: number }> {
    try {
      const res = await this.queryRaw<any>(
        "RETURN fn::sfs_fork_workspace($src, $dst, $owner);",
        { src: srcBranch, dst: dstBranch, owner: this.user }
      );
      const obj = Array.isArray(res) ? res[0] : res;
      return {
        workspace: String(obj?.workspace ?? dstBranch),
        count: Number(obj?.count ?? 0),
      };
    } catch (err) {
      throw mapSurrealError(err, dstBranch);
    }
  }

  async listWorkspaces(): Promise<WorkspaceEntry[]> {
    try {
      const rows = await this.queryRaw<any[]>(
        "SELECT id, name, owner, is_public, created_at FROM workspace " +
          "WHERE owner = $owner OR is_public = true ORDER BY created_at DESC;",
        { owner: this.user }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        id: r.id as RecordId<string>,
        name: String(r.name ?? ""),
        owner: String(r.owner ?? ""),
        isPublic: Boolean(r.is_public),
        createdAt: r.created_at ?? null,
      }));
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async diffWorkspace(branch: string): Promise<WorkspaceDiff[]> {
    try {
      const rows = await this.queryRaw<any[]>(
        "RETURN fn::sfs_diff_workspace($branch, $owner);",
        { branch, owner: this.user }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        path: String(r.path ?? ""),
        modified: Boolean(r.modified),
        conflict: Boolean(r.conflict),
        baseGen: Number(r.base_gen ?? 1),
        currentGen: Number(r.current_gen ?? 1),
      }));
    } catch (err) {
      throw mapSurrealError(err, branch);
    }
  }

  async writeWorkspaceText(
    branch: string,
    path: string,
    content: string
  ): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const branchPath = `/.workspaces/${branch}${clean}`;
      const branchFile = await this.writeText(branchPath, content);

      const wsRows = await this.queryRaw<any[]>(
        "SELECT VALUE id FROM ONLY workspace WHERE name = $branch AND owner = $owner LIMIT 1;",
        { branch, owner: this.user }
      );
      const wsId = Array.isArray(wsRows) ? wsRows[0] : wsRows;
      if (!wsId) throw new NotFoundError(branch, `Workspace not found: ${branch}`);

      await this.queryRaw<unknown>(
        `
        LET $existing = (SELECT * FROM ONLY file_branch WHERE workspace = $ws AND path = $path LIMIT 1);
        IF $existing IS NOT NONE {
            UPDATE $existing.id SET current_file = $branch_file;
        } ELSE {
            CREATE file_branch CONTENT {
                workspace: $ws,
                base_file: $branch_file,
                path: $path,
                base_gen: 1,
                current_file: $branch_file
            };
        };
        `,
        { ws: wsId, path: clean, branch_file: branchFile.id }
      );
      return branchFile;
    } catch (err) {
      throw mapSurrealError(err, path);
    }
  }

  async mergeWorkspace(
    branch: string,
    target: string = "main"
  ): Promise<{ status: string }> {
    try {
      const res = await this.queryRaw<any>(
        "RETURN fn::sfs_merge_workspace($branch, $target, $owner);",
        { branch, target, owner: this.user }
      );
      const wsPath = `/.workspaces/${branch}`;
      if (await this.exists(wsPath)) {
        await this.rm(wsPath, { recursive: true });
      }
      const obj = Array.isArray(res) ? res[0] : res;
      return { status: String(obj?.status ?? "ok") };
    } catch (err) {
      throw mapSurrealError(err, branch);
    }
  }

  async discardWorkspace(branch: string): Promise<{ discarded: string }> {
    try {
      const res = await this.queryRaw<any>(
        "RETURN fn::sfs_discard_workspace($branch, $owner);",
        { branch, owner: this.user }
      );
      const wsPath = `/.workspaces/${branch}`;
      if (await this.exists(wsPath)) {
        await this.rm(wsPath, { recursive: true });
      }
      const obj = Array.isArray(res) ? res[0] : res;
      return { discarded: String(obj?.discarded ?? branch) };
    } catch (err) {
      throw mapSurrealError(err, branch);
    }
  }

  async sendMessage(
    agentId: string,
    taskName: string,
    payload: string
  ): Promise<FileEntry> {
    const inboxDir = `/agents/${agentId}/inbox`;
    await this.mkdir(inboxDir, { parents: true });
    return await this.writeText(`${inboxDir}/${taskName}`, payload);
  }

  async receiveMessages(
    agentId: string,
    limit: number = 10
  ): Promise<FileEntry[]> {
    const inboxDir = `/agents/${agentId}/inbox`;
    if (!(await this.exists(inboxDir))) return [];
    const entries = await this.ls(inboxDir);
    const messages = entries.filter(
      (e) => !e.isFolder && !e.filename.startsWith(".")
    );
    messages.sort((a, b) => {
      const tA = a.createdAt ? new Date(a.createdAt).getTime() : 0;
      const tB = b.createdAt ? new Date(b.createdAt).getTime() : 0;
      return tA - tB;
    });
    return messages.slice(0, limit);
  }

  async claimMessage(
    agentId: string,
    taskName: string,
    consumerId: string
  ): Promise<FileEntry> {
    const inboxPath = `/agents/${agentId}/inbox/${taskName}`;
    const claimedDir = `/agents/${agentId}/inbox/.claimed/${consumerId}`;
    await this.mkdir(claimedDir, { parents: true });
    const claimedPath = `${claimedDir}/${taskName}`;
    return await this.mv(inboxPath, claimedPath);
  }

  async completeMessage(
    agentId: string,
    taskName: string,
    consumerId: string
  ): Promise<void> {
    const claimedPath = `/agents/${agentId}/inbox/.claimed/${consumerId}/${taskName}`;
    if (await this.exists(claimedPath)) {
      await this.rm(claimedPath);
    }
  }

  async watch(
    pathPattern: string,
    callback: (event: WatchEvent) => void
  ): Promise<{ unsubscribe: () => Promise<void> }> {
    const regex = globToRegex(pathPattern);
    const sub = await this.db.live(new Table("file"));
    const unsub = sub.subscribe((message) => {
      if (message.action === "KILLED") return;
      const raw = message.value as Record<string, unknown>;
      const p = String(raw?.path ?? "");
      if (regex.test(p)) {
        callback({
          action: message.action as "CREATE" | "UPDATE" | "DELETE",
          path: p,
          entry: raw ? parseFileEntry(raw) : undefined,
        });
      }
    });

    return {
      unsubscribe: async () => {
        unsub();
        await sub.kill();
      },
    };
  }
}
