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
        },
        required: ["path", "generation"],
      },
      execute: async ({ path, generation }: { path: string; generation: number }) => {
        const entry = await fs.restore(path, generation);
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
        relation,
        target_path,
      }: {
        source_path: string;
        relation: string;
        target_path: string;
      }) => {
        await fs.relate(source_path, relation, target_path);
        return { source: source_path, relation, target: target_path };
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
  };
}
