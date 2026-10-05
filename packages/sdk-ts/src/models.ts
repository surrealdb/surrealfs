/**
 * Data models for SurrealFS TypeScript SDK.
 */

import type { RecordId } from "surrealdb";

export interface FileEntry {
  id: RecordId<string>;
  filename: string;
  path: string;
  contentType: string;
  isFolder: boolean;
  owner: string;
  mode: number;
  gate: string;
  hash: string | null;
  generation: number;
  createdAt: string | Date | null;
  updatedAt: string | Date | null;
  size: number;
  meta: Record<string, unknown>;
  crdt: boolean;
  content?: string | null;
  file?: string | null;
}

export interface FileVersionEntry {
  generation: number;
  content: string;
  createdAt: string | Date | null;
  author: string;
  hash: string | null;
}

export interface SearchHit {
  path: string;
  score: number;
  snippet: string;
  generation: number;
  updatedAt: string | Date | null;
}

export interface GrepMatch {
  path: string;
  lineNumber: number;
  line: string;
}

export interface SectionHit {
  path: string;
  heading: string;
  content: string;
  distance: number;
  score: number;
  tokenCount: number;
  startLine: number;
  endLine: number;
}

export interface WatchEvent {
  action: "CREATE" | "UPDATE" | "DELETE";
  path: string;
  entry?: FileEntry;
}

export interface WorkspaceEntry {
  id: RecordId<string>;
  name: string;
  owner: string;
  isPublic: boolean;
  createdAt: string | Date | null;
}

export interface WorkspaceDiff {
  path: string;
  modified: boolean;
  conflict: boolean;
  baseGen: number;
  currentGen: number;
}

export interface LockInfo {
  path?: string;
  holder: string;
  expiresAt: string | Date;
  reason?: string;
}

export interface GraphRelation {
  source: string;
  relation: string;
  target: string;
  createdAt: string | Date | null;
}

export function parseFileEntry(raw: Record<string, unknown>): FileEntry {
  return {
    id: raw.id as RecordId<string>,
    filename: String(raw.filename ?? ""),
    path: String(raw.path ?? ""),
    contentType: String(raw.content_type ?? raw.contentType ?? "application/octet-stream"),
    isFolder: Boolean(raw.is_folder ?? raw.isFolder),
    owner: String(raw.owner ?? "root"),
    mode: Number(raw.mode ?? 0o666),
    gate: String(raw.gate ?? "open"),
    hash: (raw.hash as string) ?? null,
    generation: Number(raw.generation ?? 1),
    createdAt: (raw.created_at as string) ?? null,
    updatedAt: (raw.updated_at as string) ?? null,
    size: Number(raw.size ?? 0),
    meta: (raw.meta as Record<string, unknown>) ?? {},
    crdt: Boolean(raw.crdt),
    content: raw.content !== undefined ? (raw.content as string) : undefined,
    file: raw.file !== undefined ? (raw.file as string) : undefined,
  };
}
