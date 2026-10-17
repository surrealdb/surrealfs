/**
 * AI SDK & Tool adapters for SurrealFS.
 * Compatible with Vercel AI SDK, LangChain, and standard agentic tool callers.
 */

import type { SurrealFs } from "./fs.js";

export interface ToolDefinition<TArgs = any, TResult = any> {
  description: string;
  parameters: {
    type: "object";
    properties: Record<string, unknown>;
    required?: string[];
  };
  execute: (args: TArgs) => Promise<TResult>;
}

export function createSurrealFsTools(fs: SurrealFs): Record<string, ToolDefinition> {
  return {
    ls: {
      description: "List directory contents or examine file metadata",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Absolute path to list" },
        },
        required: ["path"],
      },
      execute: async ({ path }: { path: string }) => {
        const entries = await fs.ls(path);
        return entries.map((e) => ({
          filename: e.filename,
          path: e.path,
          isFolder: e.isFolder,
          size: e.size,
          updatedAt: e.updatedAt,
        }));
      },
    },

    read_file: {
      description: "Read the UTF-8 text content of a file",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Absolute path of file to read" },
        },
        required: ["path"],
      },
      execute: async ({ path }: { path: string }) => {
        return await fs.readText(path);
      },
    },

    write_file: {
      description: "Write content to a file, creating parents automatically",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Absolute path to write" },
          content: { type: "string", description: "UTF-8 text content" },
          branch: { type: "string", description: "Optional workspace branch" },
        },
        required: ["path", "content"],
      },
      execute: async ({
        path,
        content,
        branch,
      }: {
        path: string;
        content: string;
        branch?: string;
      }) => {
        const entry =
          branch && branch !== "main"
            ? await fs.writeWorkspaceText(branch, path, content)
            : await fs.writeText(path, content);
        return { path: entry.path, generation: entry.generation, size: entry.size };
      },
    },

    append_file: {
      description: "Append content to an existing file",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Absolute path to append" },
          suffix: { type: "string", description: "Content to append" },
        },
        required: ["path", "suffix"],
      },
      execute: async ({ path, suffix }: { path: string; suffix: string }) => {
        const entry = await fs.appendText(path, suffix);
        return { path: entry.path, generation: entry.generation, size: entry.size };
      },
    },

    edit_file: {
      description: "Replace a unique string in a file with new text",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Absolute path to edit" },
          old_text: { type: "string", description: "Exact text to find and replace" },
          new_text: { type: "string", description: "Replacement text" },
          replace_all: { type: "boolean", description: "Replace all occurrences" },
        },
        required: ["path", "old_text", "new_text"],
      },
      execute: async ({
        path,
        old_text,
        new_text,
        replace_all,
      }: {
        path: string;
        old_text: string;
        new_text: string;
        replace_all?: boolean;
      }) => {
        const entry = await fs.editText(path, old_text, new_text, {
          replaceAll: replace_all,
        });
        return { path: entry.path, generation: entry.generation };
      },
    },

    cp: {
      description: "Copy a file or directory to a destination path",
      parameters: {
        type: "object",
        properties: {
          src: { type: "string", description: "Source path" },
          dst: { type: "string", description: "Destination path" },
          recursive: { type: "boolean", description: "Copy directories recursively" },
        },
        required: ["src", "dst"],
      },
      execute: async ({
        src,
        dst,
        recursive,
      }: {
        src: string;
        dst: string;
        recursive?: boolean;
      }) => {
        const entry = await fs.cp(src, dst, { recursive });
        return { path: entry.path, generation: entry.generation };
      },
    },

    chmod: {
      description: "Change POSIX permission mode on a file or directory",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Target path" },
          mode: { type: "number", description: "POSIX mode bits (e.g. 0o644)" },
        },
        required: ["path", "mode"],
      },
      execute: async ({ path, mode }: { path: string; mode: number }) => {
        const entry = await fs.chmod(path, mode);
        return { path: entry.path, mode: entry.mode };
      },
    },

    restore: {
      description: "Restore a file to a previous version generation",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Path to restore" },
          generation: { type: "number", description: "Target generation number" },
          version: { type: "number", description: "Alias for generation" },
        },
        required: ["path", "generation"],
      },
      execute: async ({
        path,
        generation,
        version,
      }: {
        path: string;
        generation?: number;
        version?: number;
      }) => {
        const entry = await fs.restore(path, (generation ?? version)!);
        return { path: entry.path, generation: entry.generation };
      },
    },

    undelete: {
      description: "Recover a deleted file from its most recent version snapshot",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Path to undelete" },
        },
        required: ["path"],
      },
      execute: async ({ path }: { path: string }) => {
        const entry = await fs.undelete(path);
        return { path: entry.path, generation: entry.generation };
      },
    },

    relate: {
      description: "Create a semantic knowledge graph edge between two files",
      parameters: {
        type: "object",
        properties: {
          source_path: { type: "string", description: "Source file path" },
          relation: {
            type: "string",
            description: "Relation type (links_to, summarizes, derived_from, superseding)",
          },
          target_path: { type: "string", description: "Target file path" },
        },
        required: ["source_path", "relation", "target_path"],
      },
      execute: async ({
        source_path,
        source,
        src,
        relation,
        target_path,
        target,
        dst,
      }: {
        source_path?: string;
        source?: string;
        src?: string;
        relation: string;
        target_path?: string;
        target?: string;
        dst?: string;
      }) => {
        const s = (source_path ?? source ?? src)!;
        const t = (target_path ?? target ?? dst)!;
        await fs.relate(s, relation, t);
        return { source: s, relation, target: t };
      },
    },

    backlinks: {
      description: "List incoming knowledge graph relations pointing to a file",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Target file path" },
          relation: { type: "string", description: "Optional relation filter" },
        },
        required: ["path"],
      },
      execute: async ({ path, relation }: { path: string; relation?: string }) => {
        return await fs.backlinks(path, relation);
      },
    },

    search_text: {
      description: "Perform BM25 full-text search across documents",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "Text query terms" },
          limit: { type: "number", description: "Maximum number of hits" },
        },
        required: ["query"],
      },
      execute: async ({ query, limit }: { query: string; limit?: number }) => {
        return await fs.search(query, { limit });
      },
    },

    grep: {
      description: "Search text files for lines matching pattern or regex (path:line:text)",
      parameters: {
        type: "object",
        properties: {
          pattern: { type: "string", description: "Search pattern or regular expression" },
          path_prefix: { type: "string", description: "Optional path prefix filter" },
          glob: { type: "string", description: "Optional glob file pattern filter (e.g. *.md)" },
          limit: { type: "number", description: "Maximum number of matching lines" },
          is_regex: { type: "boolean", description: "Whether pattern is a regular expression" },
          case_sensitive: { type: "boolean", description: "Whether match is case-sensitive" },
        },
        required: ["pattern"],
      },
      execute: async ({
        pattern,
        path_prefix,
        glob,
        limit,
        is_regex,
        case_sensitive,
      }: {
        pattern: string;
        path_prefix?: string;
        glob?: string;
        limit?: number;
        is_regex?: boolean;
        case_sensitive?: boolean;
      }) => {
        return await fs.grep(pattern, {
          pathPrefix: path_prefix,
          glob,
          limit,
          isRegex: is_regex,
          caseSensitive: case_sensitive,
        });
      },
    },

    search_sections: {
      description: "Perform semantic vector search over document sections",
      parameters: {
        type: "object",
        properties: {
          vector: {
            type: "array",
            items: { type: "number" },
            description: "Dense embedding vector",
          },
          path_prefix: {
            type: "string",
            description: "Optional path prefix filter (e.g. /docs)",
          },
          limit: { type: "number", description: "Maximum sections to return" },
        },
        required: ["vector"],
      },
      execute: async ({
        vector,
        path_prefix,
        limit,
      }: {
        vector: number[];
        path_prefix?: string;
        limit?: number;
      }) => {
        return await fs.searchSections(vector, {
          pathPrefix: path_prefix,
          limit,
        });
      },
    },

    acquire_lease: {
      description: "Acquire an advisory lease lock on a path",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Path to lock" },
          ttl_seconds: { type: "number", description: "Lease duration in seconds" },
          reason: { type: "string", description: "Reason for the lease" },
        },
        required: ["path"],
      },
      execute: async ({
        path,
        ttl_seconds,
        reason,
      }: {
        path: string;
        ttl_seconds?: number;
        reason?: string;
      }) => {
        return await fs.acquireLock(path, { ttlSeconds: ttl_seconds, reason });
      },
    },

    release_lease: {
      description: "Release an advisory lease lock",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Path to unlock" },
        },
        required: ["path"],
      },
      execute: async ({ path }: { path: string }) => {
        return await fs.releaseLock(path);
      },
    },

    fork_workspace: {
      description: "Fork a workspace into an isolated copy-on-write branch",
      parameters: {
        type: "object",
        properties: {
          dst_branch: { type: "string", description: "Name of target branch" },
          src_branch: { type: "string", description: "Base branch (default: main)" },
        },
        required: ["dst_branch"],
      },
      execute: async ({
        dst_branch,
        src_branch,
      }: {
        dst_branch: string;
        src_branch?: string;
      }) => {
        return await fs.forkWorkspace(dst_branch, src_branch);
      },
    },

    merge_workspace: {
      description: "Merge a workspace branch back into target",
      parameters: {
        type: "object",
        properties: {
          branch: { type: "string", description: "Branch to merge" },
          target: { type: "string", description: "Target branch (default: main)" },
        },
        required: ["branch"],
      },
      execute: async ({
        branch,
        target,
      }: {
        branch: string;
        target?: string;
      }) => {
        return await fs.mergeWorkspace(branch, target);
      },
    },

    glob: {
      description: "Find files matching a glob pattern",
      parameters: {
        type: "object",
        properties: {
          pattern: { type: "string", description: "Glob pattern (e.g. /notes/**/*.md)" },
        },
        required: ["pattern"],
      },
      execute: async ({ pattern }: { pattern: string }) => {
        return await fs.glob(pattern);
      },
    },

    tree: {
      description: "Render a directory tree starting at path",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Root path (default: /)" },
        },
        required: [],
      },
      execute: async ({ path = "/" }: { path?: string }) => {
        return await fs.tree(path);
      },
    },

    mkdir: {
      description: "Create a directory",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Directory path" },
          parents: { type: "boolean", description: "Create missing parent directories" },
        },
        required: ["path"],
      },
      execute: async ({ path, parents = true }: { path: string; parents?: boolean }) => {
        return await fs.mkdir(path, { parents });
      },
    },

    rm: {
      description: "Soft-delete a file or directory",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Path to delete" },
          recursive: { type: "boolean", description: "Delete directory contents recursively" },
        },
        required: ["path"],
      },
      execute: async ({ path, recursive = false }: { path: string; recursive?: boolean }) => {
        await fs.rm(path, { recursive });
        return { deleted: path };
      },
    },

    mv: {
      description: "Move or rename a file or directory",
      parameters: {
        type: "object",
        properties: {
          src: { type: "string", description: "Source path" },
          dst: { type: "string", description: "Destination path" },
        },
        required: ["src", "dst"],
      },
      execute: async ({ src, dst }: { src: string; dst: string }) => {
        return await fs.mv(src, dst);
      },
    },

    stat: {
      description: "Inspect metadata of a file or directory",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "Path to inspect" },
        },
        required: ["path"],
      },
      execute: async ({ path }: { path: string }) => {
        return await fs.stat(path);
      },
    },

    history: {
      description: "List version history for a file",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "File path" },
          limit: { type: "number", description: "Maximum versions to return" },
        },
        required: ["path"],
      },
      execute: async ({ path, limit }: { path: string; limit?: number }) => {
        return await fs.history(path, limit);
      },
    },

    diff: {
      description: "Compute unified diff between two generations of a file",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "File path" },
          from_gen: { type: "number", description: "Base generation" },
          to_gen: { type: "number", description: "Target generation (optional, defaults to current)" },
        },
        required: ["path", "from_gen"],
      },
      execute: async ({
        path,
        from_gen,
        to_gen,
      }: {
        path: string;
        from_gen: number;
        to_gen?: number;
      }) => {
        return await fs.diff(path, from_gen, to_gen);
      },
    },

    search_semantic: {
      description: "Vector semantic search over file embeddings",
      parameters: {
        type: "object",
        properties: {
          vector: {
            type: "array",
            items: { type: "number" },
            description: "Query embedding vector",
          },
          limit: { type: "number", description: "Max results" },
        },
        required: ["vector"],
      },
      execute: async ({ vector, limit = 10 }: { vector: number[]; limit?: number }) => {
        return await fs.searchSemantic(vector, { limit });
      },
    },

    search_hybrid: {
      description: "Hybrid BM25 + vector search using Reciprocal Rank Fusion",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "Full-text search query" },
          vector: {
            type: "array",
            items: { type: "number" },
            description: "Query embedding vector",
          },
          limit: { type: "number", description: "Max results" },
          match: { type: "string", enum: ["any", "all"], description: "Match mode" },
        },
        required: ["query", "vector"],
      },
      execute: async ({
        query,
        vector,
        limit = 10,
        match = "any",
      }: {
        query: string;
        vector: number[];
        limit?: number;
        match?: "any" | "all";
      }) => {
        return await fs.searchHybrid(query, vector, { limit, match });
      },
    },
  };
}
