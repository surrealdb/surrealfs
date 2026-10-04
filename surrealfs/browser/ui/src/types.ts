/** One row of `/api/tree`, `/api/search`, and every mutating `/api/file` reply. */
export interface Entry {
    path: string;
    filename: string;
    is_folder: boolean;
    content_type: string;
    size: number;
    updated_at: string;
}

export interface Hit extends Entry {
    snippet: string;
}

/** One chat bubble, live or replayed from `/api/session`. */
export interface Bubble {
    who: "you" | "agent";
    text: string;
    tools?: string[];
    /** Streaming text renders as plain text; a finished reply renders as markdown. */
    done?: boolean;
}

/** Graph Canvas Node and Edge structures for Spatial Brain Studio. */
export interface GraphNode {
    id: string;
    label: string;
    path: string;
    is_folder: boolean;
    size: number;
    updated_at: string;
    content_type: string;
}

export interface GraphEdge {
    source: string;
    target: string;
    relation: string;
}

export interface GraphData {
    nodes: GraphNode[];
    edges: GraphEdge[];
}

/** Active Swarm Advisory Leases. */
export interface FileLockInfo {
    path: string;
    holder: string;
    expires_at: string;
    reason?: string;
}

/** Time-Travel File Version History. */
export interface FileVersionInfo {
    path: string;
    generation: number;
    author: string;
    op: string;
    created_at: string;
    reason?: string;
}

export interface ActivityData {
    locks: FileLockInfo[];
    recent_versions: FileVersionInfo[];
}
