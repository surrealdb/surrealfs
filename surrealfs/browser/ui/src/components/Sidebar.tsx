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

interface Results {
    hybrid: boolean;
    results: Hit[];
}

export function Sidebar({
    entries,
    current,
    onOpen,
    onRefresh,
    onCreate,
    onError,
}: {
    entries: Entry[];
    current: Entry | null;
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
        <Box className="pane no-print" bg="obsidian.8" style={{ borderRight: "1px solid var(--mantine-color-obsidian-6)" }}>
            <Box p="md" style={{ borderBottom: "1px solid var(--mantine-color-obsidian-7)" }}>
                <Text size="xs" tt="uppercase">
                    <span className="brand-mark">SurrealFS browser</span>
                </Text>
            </Box>

            <Stack gap="xs" p="sm" style={{ borderBottom: "1px solid var(--mantine-color-obsidian-7)" }}>
                <TextInput
                    variant="surreal"
                    placeholder="Search files…"
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
                        variant="light"
                        color="obsidian"
                        leftSection={<Icon path={iconPlus} size="sm" />}
                        onClick={() => onCreate(false)}
                    >
                        File
                    </Button>
                    <Button
                        size="xs"
                        variant="light"
                        color="obsidian"
                        leftSection={<Icon path={iconFolderPlus} size="sm" />}
                        onClick={() => onCreate(true)}
                    >
                        Folder
                    </Button>
                    <Button
                        size="xs"
                        variant="light"
                        color="obsidian"
                        aria-label="Reload the tree"
                        title="Reload the tree"
                        onClick={onRefresh}
                        style={{ flexGrow: 0 }}
                    >
                        <Icon path={iconRefresh} size="sm" />
                    </Button>
                </Group>
            </Stack>

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
        </Box>
    );
}
