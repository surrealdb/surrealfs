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
