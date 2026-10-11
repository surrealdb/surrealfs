import { Box, Button, Group, ScrollArea, Stack, Text, TextInput, UnstyledButton } from "@mantine/core";
import { useDebouncedValue } from "@mantine/hooks";
import {
    Icon,
    iconFolderPlus,
    iconPlus,
    iconRefresh,
    iconSearch,
} from "@surrealdb/ui";
import { useEffect, useState } from "react";

import { json } from "../api";
import type { Entry, Hit } from "../types";
import { FileTree } from "./FileTree";
import { SurrealGlyph } from "./SurrealLogo";

interface Results {
    hybrid: boolean;
    results: Hit[];
}

export function Sidebar({
    entries,
    current,
    view = "editor",
    onViewChange,
    onGoHome,
    onOpen,
    onRefresh,
    onCreate,
    onError,
}: {
    entries: Entry[];
    current: Entry | null;
    view?: "editor" | "canvas";
    onViewChange?: (view: "editor" | "canvas") => void;
    onGoHome?: () => void;
    onOpen: (path: string) => void;
    onRefresh: () => void;
    onCreate: (folder: boolean) => void;
    onError: (message: string) => void;
}) {
    const [query, setQuery] = useState("");
    const [debounced] = useDebouncedValue(query, 250);
    const [found, setFound] = useState<Results | null>(null);

    useEffect(() => {
        if (!debounced.trim()) {
            setFound(null);
            return;
        }
        let live = true;
        json<Results>(`/api/search?q=${encodeURIComponent(debounced)}`)
            .then((r) => live && setFound(r))
            .catch((e: Error) => live && onError(e.message));
        return () => {
            live = false;
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [debounced]);

    return (
        <Box className="side-rail no-print">
            {/* Top Brand Block */}
            <div className="side-brand">
                <div
                    className="gl"
                    style={{ cursor: "pointer" }}
                    onClick={onGoHome}
                    title="Return to Home / Overview"
                >
                    <SurrealGlyph />
                </div>
                <div className="tag">
                    SurrealFS<br />Virtual Agent Filesystem
                </div>
            </div>

            {/* Navigation Tracks */}
            <div className="side-nav">
                <div
                    className={`side-nav-item ${view === "editor" ? "active" : ""}`}
                    onClick={() => onViewChange?.("editor")}
                >
                    <span>Explorer</span>
                    <span className="mono" style={{ fontSize: 10, opacity: 0.6 }}>
                        {entries.length}
                    </span>
                </div>
                <div
                    className={`side-nav-item ${view === "canvas" ? "active" : ""}`}
                    onClick={() => onViewChange?.("canvas")}
                >
                    <span>Brain Studio ✦</span>
                    <span className="status keep" style={{ fontSize: 9, padding: "2px 5px" }}>
                        2.0
                    </span>
                </div>
            </div>

            {/* Search and Action Bar */}
            <Stack gap="xs" p="sm" style={{ borderBottom: "1px solid var(--line)" }}>
                <TextInput
                    variant="surreal"
                    placeholder="Search filesystem…"
                    autoComplete="off"
                    value={query}
                    leftSection={<Icon path={iconSearch} size="sm" />}
                    onChange={(e) => setQuery(e.currentTarget.value)}
                />
                {found ? (
                    <Text size="xs" c="dimmed">
                        {found.hybrid
                            ? `${found.results.length} hit(s) · full-text + vector`
                            : `${found.results.length} hit(s) · full-text only (set OPENAI_API_KEY)`}
                    </Text>
                ) : null}
                <Group gap="xs" grow>
                    <Button
                        size="xs"
                        className="btn-pink"
                        leftSection={<Icon path={iconPlus} size="sm" />}
                        onClick={() => onCreate(false)}
                    >
                        File
                    </Button>
                    <Button
                        size="xs"
                        className="btn-outline"
                        leftSection={<Icon path={iconFolderPlus} size="sm" />}
                        onClick={() => onCreate(true)}
                    >
                        Folder
                    </Button>
                    <Button
                        size="xs"
                        className="btn-outline"
                        aria-label="Reload the tree"
                        title="Reload the tree"
                        onClick={onRefresh}
                        style={{ flexGrow: 0, padding: "0 8px" }}
                    >
                        <Icon path={iconRefresh} size="sm" />
                    </Button>
                </Group>
            </Stack>

            {/* Flat / Search Result Tree */}
            <ScrollArea flex={1} p="xs">
                {found ? (
                    found.results.length ? (
                        <Stack gap="sm" px="xs">
                            {found.results.map((hit) => (
                                <UnstyledButton key={hit.path} onClick={() => onOpen(hit.path)}>
                                    <Text size="sm" c="bright" truncate>
                                        {hit.path}
                                    </Text>
                                    <Text size="xs" c="dimmed" ff="monospace" lineClamp={3}>
                                        {hit.snippet}
                                    </Text>
                                </UnstyledButton>
                            ))}
                        </Stack>
                    ) : (
                        <Text c="dimmed" p="md" size="sm">
                            No matches.
                        </Text>
                    )
                ) : (
                    <FileTree
                        entries={entries}
                        selected={current?.path ?? null}
                        onOpen={onOpen}
                    />
                )}
            </ScrollArea>

            {/* Bottom Status Info */}
            <div className="side-footer">
                <div className="status-indicator">SurrealDB 3.x Online</div>
                <div>ACID · Zero-Copy Branching</div>
            </div>
        </Box>
    );
}
