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
        },
        required: ["path", "content"],
      },
      execute: async ({ path, content }: { path: string; content: string }) => {
        const entry = await fs.writeText(path, content);
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
        },
        required: ["path", "old_text", "new_text"],
      },
      execute: async ({
        path,
        old_text,
        new_text,
      }: {
        path: string;
        old_text: string;
        new_text: string;
      }) => {
        const entry = await fs.editText(path, old_text, new_text);
        return { path: entry.path, generation: entry.generation };
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
