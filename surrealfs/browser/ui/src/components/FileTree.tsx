import { Box, Group, Text, Tree, type TreeNodeData, useTree } from "@mantine/core";
import { Icon, iconChevronRight, iconFile, iconOpenFolder } from "@surrealdb/ui";
import { useEffect, useMemo, useState } from "react";

import type { Entry } from "../types";

interface Node extends TreeNodeData {
    /** Absent for an implicit folder -- a path segment with no row of its own. */
    entry?: Entry;
    children: Node[];
}

const isDir = (node: Node) => !!node.entry?.is_folder || node.children.length > 0;

/**
 * The flat `/api/tree` list as a tree, synthesising the folders that have no row
 * of their own, and sorting folders first.
 */
function toTreeData(entries: Entry[]): Node[] {
    const roots: Node[] = [];
    const byPath = new Map<string, Node>();

    for (const entry of entries) {
        const parts = entry.path.split("/").filter(Boolean);
        let prefix = "";
        let siblings = roots;
        parts.forEach((part, i) => {
            prefix += `/${part}`;
            let node = byPath.get(prefix);
            if (!node) {
                node = { value: prefix, label: part, children: [] };
                byPath.set(prefix, node);
                siblings.push(node);
            }
            if (i === parts.length - 1) node.entry = entry;
            siblings = node.children;
        });
    }

    const sort = (nodes: Node[]): Node[] => {
        nodes.sort(
            (a, b) => Number(isDir(b)) - Number(isDir(a)) || a.value.localeCompare(b.value),
        );
        for (const node of nodes) sort(node.children);
        return nodes;
    };
    return sort(roots);
}

export function FileTree({
    entries,
    selected,
    onOpen,
}: {
    entries: Entry[];
    selected: string | null;
    onOpen: (path: string) => void;
}) {
    const data = useMemo(() => toTreeData(entries), [entries]);
    // Controlled, because `Tree` re-runs `initialize(data)` on every reload and
    // that replaces the expanded state wholesale. Owning it here means the
    // defaults below can be merged in afterwards, with a functional update that
    // sees what `initialize` just wrote.
    const [expandedState, setExpandedState] = useState<Record<string, boolean>>({});
    const tree = useTree({ expandedState, onExpandedStateChange: setExpandedState });

    // Folders start open to two levels deep, and only the first time they are
    // seen -- a folder the user collapsed stays collapsed across a reload.
    const seen = useMemo(() => new Set<string>(), []);
    useEffect(() => {
        const fresh: string[] = [];
        const walk = (nodes: Node[], depth: number) => {
            for (const node of nodes) {
                if (!seen.has(node.value)) {
                    seen.add(node.value);
                    if (depth < 2 && node.children.length) fresh.push(node.value);
                }
                walk(node.children, depth + 1);
            }
        };
        walk(data, 0);
        if (fresh.length)
            setExpandedState((prev) => ({
                ...prev,
                ...Object.fromEntries(fresh.map((value) => [value, true])),
            }));
    }, [data, seen]);

    if (!entries.length)
        return (
            <Text c="dimmed" p="md" size="sm">
                No files yet.
            </Text>
        );

    return (
        <Tree
            data={data}
            tree={tree}
            levelOffset="md"
            renderNode={({ node, expanded, elementProps }) => {
                const item = node as Node;
                const dir = isDir(item);
                const open = item.entry ? () => onOpen(item.entry!.path) : undefined;
                return (
                    <Group
                        {...elementProps}
                        gap={4}
                        wrap="nowrap"
                        py={2}
                        bg={item.value === selected ? "obsidian.7" : undefined}
                        // The twisty toggles; the name selects, so a folder can
                        // be renamed or deleted like anything else. Only an
                        // implicit folder -- a path segment with no row of its
                        // own -- has nothing to select, and keeps the default.
                        onClick={(e) => {
                            if (!open) return elementProps.onClick(e);
                            e.stopPropagation();
                            open();
                        }}
                    >
                        <Box
                            w={16}
                            style={{ flexShrink: 0 }}
                            onClick={(e) => {
                                e.stopPropagation();
                                tree.toggleExpanded(node.value);
                            }}
                        >
                            {dir ? (
                                <Icon
                                    path={iconChevronRight}
                                    size="sm"
                                    style={{
                                        transform: expanded ? "rotate(90deg)" : undefined,
                                    }}
                                />
                            ) : null}
                        </Box>
                        <Icon
                            path={dir ? iconOpenFolder : iconFile}
                            size="sm"
                            c={item.entry ? undefined : "dimmed"}
                        />
                        <Text size="sm" truncate c={item.value === selected ? "bright" : undefined}>
                            {node.label}
                        </Text>
                    </Group>
                );
            }}
        />
    );
}
