/**
 * SurrealFS Core Client in TypeScript.
 */

import { type RecordId, Surreal, Table } from "surrealdb";
import {
  AlreadyExistsError,
  ConflictError,
  InvalidPathError,
  IsADirectoryError,
  mapSurrealError,
  NotADirectoryError,
  NotFoundError,
  PermissionDeniedError,
  SurrealFsError,
} from "./errors.js";
import {
  type FileEntry,
  type FileVersionEntry,
  type GraphRelation,
  type GrepMatch,
  type LockInfo,
  parseFileEntry,
  type SearchHit,
  type SectionHit,
  type WatchEvent,
  type WorkspaceDiff,
  type WorkspaceEntry,
} from "./models.js";
import { fileName, globToRegex, normalize } from "./paths.js";

export interface SurrealFsOptions {
  user?: string;
  branch?: string;
}

export interface SurrealFsServerOptions {
  url: string;
  token?: string;
  user?: string;
  branch?: string;
}

export interface WriteOptions {
  ifGeneration?: number;
  createParents?: boolean;
  contentType?: string;
}

const INLINE_MAX_BYTES = 262_144;
const CHUNK_SIZE_BYTES = 1_048_576;

function makeSnippet(content: string, query: string, width: number = 240): string {
  if (!content) return "";
  const flat = content.split(/\s+/).join(" ").trim();
  if (flat.length <= width) return flat;

  const terms = (query.toLowerCase().match(/\w+/g) ?? []).filter((t) => t.length > 0);
  const lower = flat.toLowerCase();
  const positions: number[] = [];
  for (const t of terms) {
    const pos = lower.indexOf(t);
    if (pos >= 0) positions.push(pos);
  }
  if (positions.length === 0) {
    return `${flat.slice(0, width)}...`;
  }
  const hit = Math.min(...positions);
  const start = Math.max(0, hit - Math.floor(width / 3));
  const end = Math.min(flat.length, start + width);
  const prefix = start > 0 ? "..." : "";
  const suffix = end < flat.length ? "..." : "";
  return `${prefix}${flat.slice(start, end)}${suffix}`;
}

function formatUnifiedDiff(
  path: string,
  fromGen: number,
  toGen: number,
  fromContent: string,
  toContent: string
): string {
  const fromLines = fromContent ? fromContent.split(/\r?\n/) : [];
  const toLines = toContent ? toContent.split(/\r?\n/) : [];
  const out: string[] = [
    `--- ${path} (gen ${fromGen})`,
    `+++ ${path} (gen ${toGen})`,
    `@@ -1,${fromLines.length} +1,${toLines.length} @@`,
  ];
  const maxLen = Math.max(fromLines.length, toLines.length);
  for (let i = 0; i < maxLen; i++) {
    const a = i < fromLines.length ? fromLines[i] : undefined;
    const b = i < toLines.length ? toLines[i] : undefined;
    if (a === b && a !== undefined) {
      out.push(` ${a}`);
    } else {
      if (a !== undefined) out.push(`-${a}`);
      if (b !== undefined) out.push(`+${b}`);
    }
  }
  return out.join("\n");
}

function toBytes(val: unknown): Uint8Array | null {
  if (val === null || val === undefined) return null;
  if (val instanceof Uint8Array) return val;
  if (val instanceof ArrayBuffer) return new Uint8Array(val);
  if (ArrayBuffer.isView(val)) {
    return new Uint8Array(val.buffer, val.byteOffset, val.byteLength);
  }
  if (Array.isArray(val)) return new Uint8Array(val);
  return null;
}

function validateBranchName(branch: string): void {
  if (!branch || branch.includes("/") || branch === "." || branch === "..") {
    throw new InvalidPathError(branch, `Invalid workspace branch name: ${branch}`);
  }
}

class ServerRpcClient {
  private readonly rpcUrl: string;
  private readonly token?: string;
  private readonly user: string;

  constructor(opts: SurrealFsServerOptions) {
    const base = opts.url
      .trim()
      .replace(/^sfs\+https:\/\//, "https://")
      .replace(/^sfs\+http:\/\//, "http://")
      .replace(/^sfs:\/\//, "http://")
      .replace(/\/+$/, "");
    this.rpcUrl = `${base}/api/rpc`;
    this.token = opts.token;
    this.user = opts.user ?? "root";
  }

  async query<T = unknown[]>(
    sql: string,
    vars: Record<string, unknown> = {}
  ): Promise<T> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      "X-SurrealFS-User": this.user,
    };
    if (this.token) {
      headers["Authorization"] = `Bearer ${this.token}`;
    }
    const serializedVars: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(vars)) {
      serializedVars[k] = v instanceof Uint8Array ? Array.from(v) : v;
    }
    const resp = await fetch(this.rpcUrl, {
      method: "POST",
      headers,
      body: JSON.stringify({
        sql,
        vars: serializedVars,
        caller: this.user,
      }),
    });
    const body = (await resp.json()) as {
      error?: string;
      result?: Array<{ status?: string; result?: unknown }>;
      results?: Array<{ status?: string; result?: unknown }>;
    };
    if (!resp.ok || body.error) {
      throw new Error(body.error || `Server RPC error (${resp.status})`);
    }
    const stmts = body.result ?? body.results ?? [];
    const out: unknown[] = [];
    for (const stmt of stmts) {
      if (stmt.status === "ERR") {
        throw new Error(String(stmt.result ?? "Query error"));
      }
      out.push(stmt.result);
    }
    return out as unknown as T;
  }

  async live(_table: unknown): Promise<never> {
    throw new SurrealFsError(
      "Live queries require a direct WebSocket connection; use polling with connectServer"
    );
  }
}

export class SurrealFs {
  readonly db: Surreal;
  readonly user: string;
  branch: string;

  constructor(db: Surreal, options: SurrealFsOptions = {}) {
    this.db = db;
    this.user = options.user ?? "root";
    this.branch = options.branch ?? "main";
  }

  /**
   * Connect to a Layer 2 SurrealFS Server (Cloudflare Worker or `surrealfs-server`)
   * over HTTPS (`/api/rpc`) without direct SurrealDB credentials.
   */
  static connectServer(options: SurrealFsServerOptions): SurrealFs {
    const rpcClient = new ServerRpcClient(options) as unknown as Surreal;
    return new SurrealFs(rpcClient, {
      user: options.user,
      branch: options.branch,
    });
  }

  setBranch(branch: string): void {
    validateBranchName(branch);
    this.branch = branch;
  }

  private async queryRaw<T = unknown>(
    sql: string,
    vars: Record<string, unknown> = {}
  ): Promise<T> {
    const cleanVars: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(vars)) {
      cleanVars[k] = v === null ? undefined : v;
    }
    try {
      const results = await this.db.query<unknown[]>(sql, cleanVars);
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
      const entry = parseFileEntry(res as Record<string, unknown>);
      return entry;
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async exists(path: string): Promise<boolean> {
    try {
      await this.stat(path);
      return true;
    } catch (err) {
      if (err instanceof NotFoundError || err instanceof PermissionDeniedError) {
        return false;
      }
      throw err;
    }
  }

  async ls(path: string = "/"): Promise<FileEntry[]> {
    const clean = normalize(path);
    if (clean !== "/") {
      const st = await this.stat(clean);
      if (!st.isFolder) {
        throw new NotADirectoryError(clean);
      }
    }
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

  async glob(pattern: string): Promise<FileEntry[]> {
    const cleanPattern = normalize(pattern);
    const rx = globToRegex(cleanPattern);
    try {
      const rows = await this.queryRaw<Record<string, unknown>[]>(
        `
        RETURN {
          LET $who = fn::sfs_caller($caller);
          RETURN (
            SELECT id, filename, path, content_type, is_folder, owner, group, mode, gate,
                   hash, generation, created_at, updated_at, size, meta, crdt
            FROM file
            WHERE deleted_at IS NONE
              AND ($who = 'root' OR fn::sfs_can_read_grp(gate, owner, mode, $who, group))
            ORDER BY path ASC
          );
        };
        `,
        { caller: this.user }
      );
      if (!Array.isArray(rows)) return [];
      return rows
        .map((r) => parseFileEntry(r))
        .filter((entry) => rx.test(entry.path));
    } catch (err) {
      throw mapSurrealError(err, cleanPattern);
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

  async lsRecursive(path: string = "/", maxDepth: number = 20): Promise<FileEntry[]> {
    const results: FileEntry[] = [];
    const walk = async (current: string, depth: number) => {
      if (depth > maxDepth) return;
      const entries = await this.ls(current);
      for (const entry of entries) {
        results.push(entry);
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
    if (this.branch !== "main" && !clean.startsWith("/.workspaces/")) {
      const branchPath = `/.workspaces/${this.branch}${clean}`;
      if (await this.exists(branchPath)) {
        return await this.readText(branchPath);
      }
    }
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

  async readBytes(path: string): Promise<Uint8Array> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_read($path, $caller);",
        { path: clean, caller: this.user }
      );
      const directBytes = toBytes(res);
      if (directBytes !== null) {
        return directBytes;
      }
      if (typeof res === "string" && res.length > 0) {
        return new TextEncoder().encode(res);
      }
      if (res && typeof res === "object") {
        const rec = res as Record<string, unknown>;
        const inlineBytes = toBytes(rec.file);
        if (inlineBytes !== null) {
          return inlineBytes;
        }
        if (typeof rec.content === "string" && rec.content.length > 0) {
          return new TextEncoder().encode(rec.content);
        }
      }
      const entry = await this.stat(clean);
      if (entry.size === 0) {
        return new Uint8Array(0);
      }
      const chunkBytes = await this.queryRaw<unknown>(
        "RETURN fn::sfs_read_bytes_range($path, 0, $length, $caller);",
        { path: clean, length: Math.max(entry.size, 104_857_600), caller: this.user }
      );
      const decoded = toBytes(chunkBytes);
      if (decoded !== null) {
        return decoded;
      }
      return new Uint8Array(0);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async writeBytes(
    path: string,
    data: Uint8Array,
    options: WriteOptions = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    if (clean === "/") {
      throw new IsADirectoryError(clean);
    }
    const name = fileName(clean);
    try {
      if (data.byteLength <= INLINE_MAX_BYTES) {
        const res = await this.queryRaw<Record<string, unknown>>(
          `
          RETURN {
            LET $who = fn::sfs_caller($caller);
            LET $parent_id = IF $parents {
                fn::sfs_ensure_parents($path, $who)
            } ELSE {
                LET $parts = array::filter(string::split($path, '/'), |$s| $s != '');
                LET $dir_parts = array::slice($parts, 0, array::len($parts) - 1);
                IF array::len($dir_parts) == 0 {
                    NONE
                } ELSE {
                    LET $pid = fn::sfs_resolve('/' + array::join($dir_parts, '/'));
                    IF $pid IS NONE { THROW 'sfs:not_found' };
                    $pid
                }
            };
            IF !fn::sfs_home_ok($parent_id, $filename, $who) {
                THROW 'sfs:denied';
            };
            LET $pkey = IF $parent_id IS NONE { 'root' } ELSE { <string>$parent_id };
            LET $existing = (SELECT * FROM ONLY file WHERE parent_key = $pkey AND filename = $filename LIMIT 1);
            IF $existing IS NOT NONE AND $existing.is_folder AND $existing.deleted_at IS NONE {
                THROW 'sfs:is_a_directory';
            };
            IF $if_gen IS NOT NONE {
                IF $existing IS NONE OR $existing.deleted_at IS NOT NONE OR $existing.generation != $if_gen {
                    THROW 'sfs:conflict';
                };
            };
            LET $mime = fn::sfs_guess_mime($filename, $content_type);
            IF $existing IS NONE {
                IF $parent_id IS NOT NONE AND $who != 'root' {
                    LET $prow = (SELECT gate, owner, group, mode FROM ONLY $parent_id);
                    IF !fn::sfs_can_write_grp($prow.gate, $prow.owner, $prow.mode, $who, $prow.group) {
                        THROW 'sfs:denied';
                    };
                };
                CREATE ONLY file CONTENT {
                    filename: $filename,
                    parent: $parent_id,
                    content: NONE,
                    file: <bytes>$data,
                    size: $size,
                    content_type: $mime,
                    owner: $who,
                    generation: 1
                };
            } ELSE {
                IF $existing.deleted_at IS NONE AND $who != 'root' AND !fn::sfs_can_write_grp($existing.gate, $existing.owner, $existing.mode, $who, $existing.group) {
                    THROW 'sfs:denied';
                };
                UPDATE ONLY $existing.id MERGE {
                    content: NONE,
                    file: <bytes>$data,
                    size: $size,
                    content_type: $mime,
                    deleted_at: NONE,
                    updated_at: time::now()
                };
            };
            RETURN fn::sfs_stat($path);
          };
          `,
          {
            path: clean,
            filename: name,
            data,
            size: data.byteLength,
            if_gen: options.ifGeneration ?? null,
            parents: options.createParents ?? true,
            content_type: options.contentType ?? null,
            caller: this.user,
          }
        );
        return parseFileEntry(res);
      }

      const session = await this.queryRaw<Record<string, unknown>>(
        "RETURN fn::sfs_upload_begin($path, $content_type, $total_size, $caller);",
        {
          path: clean,
          content_type: options.contentType ?? "application/octet-stream",
          total_size: data.byteLength,
          caller: this.user,
        }
      );
      const sessionId = session.id;
      let idx = 0;
      for (let offset = 0; offset < data.byteLength; offset += CHUNK_SIZE_BYTES) {
        const chunk = data.subarray(
          offset,
          Math.min(offset + CHUNK_SIZE_BYTES, data.byteLength)
        );
        await this.queryRaw(
          "RETURN fn::sfs_upload_chunk($session_id, $chunk_idx, <bytes>$data);",
          {
            session_id: sessionId,
            chunk_idx: idx,
            data: chunk,
          }
        );
        idx += 1;
      }
      const committed = await this.queryRaw<Record<string, unknown>>(
        "RETURN fn::sfs_upload_commit($session_id, $caller);",
        { session_id: sessionId, caller: this.user }
      );
      return parseFileEntry(committed);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async writeText(
    path: string,
    content: string,
    options: WriteOptions = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    if (this.branch !== "main" && !clean.startsWith("/.workspaces/")) {
      return await this.writeWorkspaceText(this.branch, clean, content);
    }
    const existing = await this.stat(clean).catch(() => null);
    if (existing?.crdt) {
      const { doc, nextSeq, baseGen } = await this.loadCrdtDoc(clean);
      const text = doc.getText("text");
      const current = text.toString();
      let delta = new Uint8Array(0);
      doc.on("update", (update: Uint8Array) => {
        delta = new Uint8Array(update);
      });
      doc.transact(() => {
        text.delete(0, current.length);
        text.insert(0, content);
      });
      return await this.applyCrdtMutation(clean, delta, text.toString(), nextSeq, baseGen);
    }
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_write($path, $content, $if_gen, $parents, $caller, $content_type);",
        {
          path: clean,
          content,
          if_gen: options.ifGeneration ?? null,
          parents: options.createParents ?? true,
          caller: this.user,
          content_type: options.contentType ?? null,
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
    options: { ifGeneration?: number } | number = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    const opts = typeof options === "number" ? { ifGeneration: options } : options;
    const existing = await this.stat(clean).catch(() => null);
    if (existing?.crdt) {
      const { doc, nextSeq, baseGen } = await this.loadCrdtDoc(clean);
      const text = doc.getText("text");
      let delta = new Uint8Array(0);
      doc.on("update", (update: Uint8Array) => {
        delta = new Uint8Array(update);
      });
      doc.transact(() => {
        text.insert(text.length, suffix);
      });
      return await this.applyCrdtMutation(clean, delta, text.toString(), nextSeq, baseGen);
    }
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_append($path, $suffix, $if_gen, $caller);",
        {
          path: clean,
          suffix,
          if_gen: opts.ifGeneration ?? null,
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
    options: { ifGeneration?: number; replaceAll?: boolean } = {}
  ): Promise<FileEntry> {
    const clean = normalize(path);
    if (!oldText) {
      throw new InvalidPathError(clean, "oldText must not be empty");
    }
    const existing = await this.stat(clean);
    if (existing.crdt) {
      const { doc, nextSeq, baseGen } = await this.loadCrdtDoc(clean);
      const text = doc.getText("text");
      const str = text.toString();
      const idx = str.indexOf(oldText);
      if (idx === -1) {
        throw new ConflictError(clean, `Pattern not found in file: ${oldText}`);
      }
      if (!options.replaceAll && str.indexOf(oldText, idx + oldText.length) !== -1) {
        throw new ConflictError(
          clean,
          "oldText matches multiple locations; set replaceAll: true"
        );
      }
      let delta = new Uint8Array(0);
      doc.on("update", (update: Uint8Array) => {
        delta = new Uint8Array(update);
      });
      doc.transact(() => {
        if (options.replaceAll) {
          const updated = str.split(oldText).join(newText);
          text.delete(0, str.length);
          text.insert(0, updated);
        } else {
          text.delete(idx, oldText.length);
          text.insert(idx, newText);
        }
      });
      return await this.applyCrdtMutation(clean, delta, text.toString(), nextSeq, baseGen);
    }

    const currentContent = await this.readText(clean);
    const firstIdx = currentContent.indexOf(oldText);
    if (firstIdx === -1) {
      throw new ConflictError(clean, `Pattern not found in file: ${oldText}`);
    }
    if (
      !options.replaceAll &&
      currentContent.indexOf(oldText, firstIdx + oldText.length) !== -1
    ) {
      throw new ConflictError(
        clean,
        "oldText matches multiple locations; set replaceAll: true"
      );
    }
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
      await this.queryRaw<unknown>(
        "RETURN fn::sfs_chmod($path, $mode, $caller);",
        { path: clean, mode, caller: this.user }
      );
      return await this.stat(clean);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async chgrp(path: string, group: string | null): Promise<FileEntry> {
    const clean = normalize(path);
    try {
      const res = await this.queryRaw<unknown>(
        "RETURN fn::sfs_chgrp($path, $group, $caller);",
        { path: clean, group, caller: this.user }
      );
      return parseFileEntry(res as Record<string, unknown>);
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async chown(path: string, _owner: string): Promise<FileEntry> {
    const clean = normalize(path);
    throw new PermissionDeniedError(
      clean,
      "owner is immutable in SurrealFS; use chgrp to change group"
    );
  }

  async history(path: string, limit?: number): Promise<FileVersionEntry[]> {
    const clean = normalize(path);
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_history($path, $limit, $caller);",
        { path: clean, limit: limit ?? 20, caller: this.user }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        generation: Number(r.generation ?? 1),
        content: String(r.content ?? ""),
        createdAt: r.created_at ?? null,
        author: String(r.author ?? ""),
        hash: r.hash ?? null,
        size: Number(r.size ?? 0),
        op: String(r.op ?? "write"),
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
      const res = await this.queryRaw<any>(
        "RETURN fn::sfs_diff($path, $gen_a, $gen_b, $caller);",
        {
          path: clean,
          gen_a: genA ?? null,
          gen_b: genB ?? null,
          caller: this.user,
        }
      );
      if (typeof res === "string") return res;
      if (res && typeof res === "object") {
        const fromGen = Number(res.from_generation ?? genA ?? 0);
        const toGen = Number(res.to_generation ?? genB ?? 0);
        const fromContent = String(res.from_content ?? "");
        let toContent = String(res.to_content ?? "");
        if (!toContent && genB !== undefined) {
          const stat = await this.stat(clean);
          if (stat.generation === genB) {
            toContent = await this.readText(clean);
          }
        }
        return formatUnifiedDiff(clean, fromGen, toGen, fromContent, toContent);
      }
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

  async undelete(path: string): Promise<FileEntry> {
    const clean = normalize(path);
    if (await this.exists(clean)) {
      throw new AlreadyExistsError(clean);
    }
    try {
      const rows = await this.queryRaw<any[]>(
        "SELECT * FROM file_version WHERE path = $path ORDER BY created_at DESC, generation DESC LIMIT 1;",
        { path: clean }
      );
      const snap = Array.isArray(rows) && rows.length > 0 ? rows[0] : null;
      if (!snap) {
        throw new NotFoundError(clean, `No version snapshot found for ${clean}`);
      }
      const bytes = toBytes(snap.file_bytes ?? snap.file);
      if (bytes !== null) {
        return await this.writeBytes(clean, bytes);
      }
      return await this.writeText(clean, String(snap.content ?? ""));
    } catch (err) {
      throw mapSurrealError(err, clean);
    }
  }

  async search(
    query: string,
    optionsOrLimit: { limit?: number; match?: "any" | "all" } | number = {},
    matchArg?: "any" | "all"
  ): Promise<SearchHit[]> {
    const opts =
      typeof optionsOrLimit === "number"
        ? { limit: optionsOrLimit, match: matchArg }
        : optionsOrLimit;
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_search_text($q, $k, $match, $me);",
        {
          q: query,
          k: opts.limit ?? 20,
          match: opts.match ?? "any",
          me: this.user,
        }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        path: String(r.path ?? ""),
        score: Number(r.score ?? 0),
        snippet: r.snippet
          ? String(r.snippet)
          : makeSnippet(String(r.content ?? ""), query),
        generation: Number(r.generation ?? 1),
        updatedAt: r.updated_at ?? null,
      }));
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async searchSemantic(
    vector: number[],
    optionsOrLimit: { limit?: number } | number = {}
  ): Promise<SearchHit[]> {
    const limit =
      typeof optionsOrLimit === "number"
        ? optionsOrLimit
        : (optionsOrLimit.limit ?? 10);
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_search_semantic($qvec, $k, $me);",
        {
          qvec: vector,
          k: limit,
          me: this.user,
        }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        path: String(r.path ?? ""),
        score: Number(r.score ?? 0),
        snippet: r.snippet
          ? String(r.snippet)
          : makeSnippet(String(r.content ?? ""), ""),
        generation: Number(r.generation ?? 1),
        updatedAt: r.updated_at ?? null,
      }));
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async searchHybrid(
    query: string,
    vector: number[],
    optionsOrLimit: { limit?: number; match?: "any" | "all" } | number = {},
    matchArg?: "any" | "all"
  ): Promise<SearchHit[]> {
    const opts =
      typeof optionsOrLimit === "number"
        ? { limit: optionsOrLimit, match: matchArg }
        : optionsOrLimit;
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_hybrid_search($q, $qvec, $k, $match, $me);",
        {
          q: query,
          qvec: vector,
          k: opts.limit ?? 10,
          match: opts.match ?? "any",
          me: this.user,
        }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        path: String(r.path ?? ""),
        score: Number(r.score ?? 0),
        snippet: r.snippet
          ? String(r.snippet)
          : makeSnippet(String(r.content ?? ""), query),
        generation: Number(r.generation ?? 1),
        updatedAt: r.updated_at ?? null,
      }));
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async grep(
    pattern: string,
    options: {
      pathPrefix?: string;
      glob?: string;
      limit?: number;
      isRegex?: boolean;
      caseSensitive?: boolean;
    } = {}
  ): Promise<GrepMatch[]> {
    if (!pattern) return [];
    const limit = options.limit ?? 100;
    if (limit <= 0) return [];

    const isRegex = options.isRegex ?? false;
    const caseSensitive = options.caseSensitive ?? true;

    const rawRegex = isRegex ? pattern : pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const compiled = new RegExp(rawRegex, caseSensitive ? "" : "i");

    const normPrefix = options.pathPrefix ? normalize(options.pathPrefix) : undefined;
    const cleanPrefix = normPrefix === "/" ? undefined : normPrefix;

    const globMatcher = options.glob
      ? globToRegex(options.glob.startsWith("/") ? options.glob : `/**/${options.glob}`)
      : null;

    try {
      const rows = await this.queryRaw<any[]>(
        "RETURN fn::sfs_grep($pattern, $prefix, $me);",
        {
          pattern: rawRegex,
          prefix: cleanPrefix,
          me: this.user,
        }
      );
      if (!Array.isArray(rows)) return [];

      const matches: GrepMatch[] = [];
      for (const row of rows) {
        const entryPath = String(row.path ?? "");
        if (globMatcher && !globMatcher.test(entryPath)) continue;
        const content = String(row.content ?? "");
        const lines = content.split(/\r?\n/);
        for (let idx = 0; idx < lines.length; idx++) {
          const line = lines[idx];
          if (compiled.test(line)) {
            matches.push({
              path: entryPath,
              lineNumber: idx + 1,
              line,
            });
            if (matches.length >= limit) return matches;
          }
        }
      }
      return matches;
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async searchSections(
    vector: number[],
    optionsOrLimit: { limit?: number; pathPrefix?: string } | number = {},
    pathPrefixArg?: string
  ): Promise<SectionHit[]> {
    const opts =
      typeof optionsOrLimit === "number"
        ? { limit: optionsOrLimit, pathPrefix: pathPrefixArg }
        : optionsOrLimit;
    const normPrefix = opts.pathPrefix ? normalize(opts.pathPrefix) : null;
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_search_sections($qvec, $k, $me, $prefix);",
        {
          qvec: vector,
          k: opts.limit ?? 10,
          me: this.user,
          prefix: normPrefix,
        }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => {
        const dist = Number(r.distance ?? r.dist ?? 0);
        const score =
          r.score !== undefined ? Number(r.score) : Math.max(0, 1.0 - dist);
        return {
          path: String(r.path ?? ""),
          heading: String(r.heading ?? ""),
          content: String(r.content ?? ""),
          distance: dist,
          score,
          tokenCount: Number(r.token_count ?? 0),
          startLine: Number(r.line_start ?? r.start_line ?? 0),
          endLine: Number(r.line_end ?? r.end_line ?? 0),
        };
      });
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

  async backlinks(path: string, relation?: string): Promise<GraphRelation[]> {
    const clean = normalize(path);
    try {
      const rows = await this.queryRaw<unknown[]>(
        "RETURN fn::sfs_backlinks($path, $caller);",
        { path: clean, caller: this.user }
      );
      if (!Array.isArray(rows)) return [];
      const mapped = rows.map((r: any) => {
        const srcPath = String(r.source_path ?? r.source ?? r.path ?? "");
        return {
          source: srcPath,
          sourcePath: srcPath,
          sourceName: String(r.source_name ?? fileName(srcPath)),
          relation: String(r.relation ?? ""),
          target: clean,
          createdAt: r.created_at ?? null,
        };
      });
      return relation ? mapped.filter((m) => m.relation === relation) : mapped;
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

  async listLocks(): Promise<Array<LockInfo & { path: string }>> {
    try {
      const res = await this.queryRaw<any[]>(
        "SELECT * FROM file_lock WHERE expires_at > time::now();"
      );
      if (!Array.isArray(res)) return [];
      return res.map((r) => ({
        path: String(r.path ?? ""),
        holder: String(r.holder ?? ""),
        expiresAt: r.expires_at ?? new Date(),
        reason: r.reason ? String(r.reason) : undefined,
      }));
    } catch (err) {
      throw mapSurrealError(err);
    }
  }

  async forkWorkspace(
    dstBranch: string,
    srcBranch: string = "main"
  ): Promise<{ workspace: string; count: number }> {
    validateBranchName(dstBranch);
    validateBranchName(srcBranch);
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
    validateBranchName(branch);
    try {
      const rows = await this.queryRaw<any[]>(
        "RETURN fn::sfs_diff_workspace($branch, $owner);",
        { branch, owner: this.user }
      );
      if (!Array.isArray(rows)) return [];
      return rows.map((r: any) => ({
        path: String(r.path ?? ""),
        status: r.status
          ? String(r.status)
          : r.base_file == null
          ? "added"
          : r.modified
          ? "modified"
          : "unchanged",
        modified: Boolean(r.modified),
        conflict: Boolean(r.conflict),
        baseGen: Number(r.base_gen ?? 0),
        currentGen: Number(r.branch_gen ?? r.current_gen ?? 1),
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
    validateBranchName(branch);
    const clean = normalize(path);
    try {
      const wsRows = await this.queryRaw<any[]>(
        "SELECT VALUE id FROM ONLY workspace WHERE name = $branch AND owner = $owner LIMIT 1;",
        { branch, owner: this.user }
      );
      const wsId = Array.isArray(wsRows) ? wsRows[0] : wsRows;
      if (!wsId) throw new NotFoundError(branch, `Workspace not found: ${branch}`);

      const branchPath = `/.workspaces/${branch}${clean}`;
      const branchFile = await this.writeText(branchPath, content);

      await this.queryRaw<unknown>(
        `
        RETURN {
          LET $existing = (SELECT * FROM ONLY file_branch WHERE workspace = $ws AND path = $path LIMIT 1);
          IF $existing IS NOT NONE {
              UPDATE $existing.id SET current_file = $branch_file;
          } ELSE {
              CREATE file_branch CONTENT {
                  workspace: $ws,
                  base_file: NONE,
                  path: $path,
                  base_gen: 0,
                  current_file: $branch_file
              };
          };
          RETURN true;
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
    validateBranchName(branch);
    validateBranchName(target);
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
    validateBranchName(branch);
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
    const regex = globToRegex(normalize(pathPattern));
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

  async enableCrdt(path: string): Promise<void> {
    const clean = normalize(path);
    const existing = await this.stat(clean);
    if (!existing) {
      throw new NotFoundError(clean);
    }
    if (existing.crdt) return;
    const content = (await this.readText(clean)) ?? "";
    const Y = await import("yjs");
    const doc = new Y.Doc();
    const text = doc.getText("text");
    if (content.length > 0) {
      text.insert(0, content);
    }
    const update = Y.encodeStateAsUpdate(doc);

    const sql = `
      RETURN {
        LET $file_id = fn::sfs_resolve($path);
        IF $file_id IS NONE { THROW 'sfs:not_found' };
        CREATE file_crdt_snapshot CONTENT {
          file_id: $file_id,
          upto: 1,
          state: <bytes>$state,
          author: $author
        };
        CREATE file_crdt_update CONTENT {
          file_id: $file_id,
          seq: 1,
          update_bytes: <bytes>$state,
          author: $author,
          base_gen: $base_gen
        };
        UPDATE $file_id SET crdt = true;
        RETURN true;
      };
    `;
    await this.queryRaw(sql, {
      path: clean,
      state: update,
      author: this.user,
      base_gen: existing.generation ?? 1,
    });
  }

  async compactCrdt(path: string): Promise<void> {
    const clean = normalize(path);
    const existing = await this.stat(clean);
    if (!existing) {
      throw new NotFoundError(clean);
    }
    if (!existing.crdt) return;

    const Y = await import("yjs");
    const { doc, nextSeq } = await this.loadCrdtDoc(clean);
    const upto = nextSeq - 1;
    const snapshot = Y.encodeStateAsUpdate(doc);

    const sqlCompact = `
      RETURN {
        LET $file_id = fn::sfs_resolve($path);
        IF $file_id IS NONE { THROW 'sfs:not_found' };
        UPSERT file_crdt_snapshot CONTENT {
          file_id: $file_id,
          upto: $upto,
          state: <bytes>$snapshot,
          author: $author
        } WHERE file_id = $file_id;
        DELETE file_crdt_update WHERE file_id = $file_id AND seq <= $upto;
        RETURN true;
      };
    `;
    await this.queryRaw(sqlCompact, {
      path: clean,
      upto,
      snapshot,
      author: this.user,
    });
  }

  private async loadCrdtDoc(
    path: string
  ): Promise<{ doc: any; nextSeq: number; baseGen: number }> {
    const clean = normalize(path);
    const sql = `
      RETURN {
        LET $file_id = fn::sfs_resolve($path);
        IF $file_id IS NONE { THROW 'sfs:not_found' };
        LET $stat = (SELECT generation, content FROM ONLY $file_id);
        LET $snap = (SELECT upto, state FROM ONLY file_crdt_snapshot WHERE file_id = $file_id LIMIT 1);
        LET $upto = IF $snap.upto IS NOT NONE { $snap.upto } ELSE { 0 };
        LET $updates = (
          SELECT seq, update_bytes FROM file_crdt_update 
          WHERE file_id = $file_id AND seq > $upto 
          ORDER BY seq ASC
        );
        RETURN {
          gen: $stat.generation,
          content: $stat.content,
          upto: $upto,
          snap_state: $snap.state,
          updates: $updates
        };
      };
    `;
    const res = await this.queryRaw<any>(sql, { path: clean });
    const Y = await import("yjs");
    const doc = new Y.Doc();

    const snapBytes = toBytes(res.snap_state);
    if (snapBytes && snapBytes.byteLength > 0) {
      Y.applyUpdate(doc, snapBytes);
    }
    let maxSeq = Number(res.upto ?? 0);
    if (Array.isArray(res.updates)) {
      for (const u of res.updates) {
        if (Number(u.seq) > maxSeq) {
          maxSeq = Number(u.seq);
        }
        const uBytes = toBytes(u.update_bytes);
        if (uBytes && uBytes.byteLength > 0) {
          Y.applyUpdate(doc, uBytes);
        }
      }
    }
    const baseGen = Number(res.gen ?? 1);
    return { doc, nextSeq: maxSeq + 1, baseGen };
  }

  private async applyCrdtMutation(
    path: string,
    delta: Uint8Array,
    newContent: string,
    seq: number,
    baseGen: number
  ): Promise<FileEntry> {
    const clean = normalize(path);
    const sql = `
      RETURN {
        LET $file_id = fn::sfs_resolve($path);
        IF $file_id IS NONE { THROW 'sfs:not_found' };
        CREATE file_crdt_update CONTENT {
          file_id: $file_id,
          seq: $seq,
          update_bytes: <bytes>$delta,
          author: $author,
          base_gen: $base_gen
        };
        UPDATE $file_id SET content = $content, updated_at = time::now();
        RETURN fn::sfs_stat($path);
      };
    `;
    const res = await this.queryRaw<Record<string, unknown>>(sql, {
      path: clean,
      delta,
      seq,
      author: this.user,
      base_gen: baseGen,
      content: newContent,
    });
    return parseFileEntry(res);
  }
}
