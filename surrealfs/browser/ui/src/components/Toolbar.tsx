import { ActionIcon, Badge, Group, SegmentedControl, Text, Tooltip } from "@mantine/core";
import {
    BreadcrumbButton,
    Icon,
    iconDownload,
    iconEdit,
    iconFloppy,
    iconTrash,
} from "@surrealdb/ui";

import { formatSize, hasPreview, isText } from "../api";
import type { Mode } from "../App";
import type { Entry } from "../types";

export function Toolbar({
    current,
    entries,
    mode,
    dirty,
    onMode,
    onSave,
    onRename,
    onDelete,
    onPrint,
    onOpen,
}: {
    current: Entry | null;
    entries: Entry[];
    mode: Mode;
    dirty: boolean;
    onMode: (mode: Mode) => void;
    onSave: () => void;
    onRename: () => void;
    onDelete: () => void;
    onPrint: () => void;
    onOpen: (path: string) => void;
}) {
    if (!current)
        return (
            <Group className="no-print" h={48} px="md" style={border}>
                <Text c="dimmed" size="sm">
                    Nothing open
                </Text>
            </Group>
        );

    const ct = current.content_type;
    const segments = current.path.split("/").filter(Boolean);
    const known = new Set(entries.map((e) => e.path));

    return (
        <Group className="no-print" h={48} px="md" gap="xs" wrap="nowrap" style={border}>
            <Group gap={2} wrap="nowrap" style={{ overflow: "hidden" }}>
                {segments.map((name, i) => {
                    const path = `/${segments.slice(0, i + 1).join("/")}`;
                    const last = i === segments.length - 1;
                    return (
                        <BreadcrumbButton
                            key={path}
                            size="compact-sm"
                            disabled={!known.has(path) || path === current.path}
                            onClick={() => onOpen(path)}
                            // Only the filename gives way when the path is long;
                            // half a squashed folder name reads as a typo.
                            style={{ flexShrink: last ? 1 : 0 }}
                        >
                            {name}
                        </BreadcrumbButton>
                    );
                })}
            </Group>

            {dirty ? (
                <Badge size="sm" color="yellow" variant="light">
                    unsaved
                </Badge>
            ) : null}

            <Text size="xs" c="dimmed" ff="monospace" style={{ whiteSpace: "nowrap" }}>
                {current.is_folder ? "folder" : `${ct} · ${formatSize(current.size)}`}
            </Text>

            <Group gap="xs" ml="auto" wrap="nowrap">
                {!current.is_folder && hasPreview(ct) ? (
                    <SegmentedControl
                        size="xs"
                        value={mode}
                        onChange={(value) => onMode(value as Mode)}
                        data={[
                            { label: "Preview", value: "preview" },
                            { label: "Source", value: "source" },
                        ]}
                    />
                ) : null}

                {ct === "text/markdown" ? (
                    <Tooltip label="Print / save as PDF">
                        <ActionIcon variant="light" color="obsidian" onClick={onPrint} aria-label="Print or save as PDF">
                            <Icon path={iconDownload} size="sm" />
                        </ActionIcon>
                    </Tooltip>
                ) : null}

                {!current.is_folder && isText(ct) ? (
                    <Tooltip label="Save (⌘S)">
                        <ActionIcon variant="surreal" disabled={!dirty} onClick={onSave} aria-label="Save">
                            <Icon path={iconFloppy} size="sm" />
                        </ActionIcon>
                    </Tooltip>
                ) : null}

                <Tooltip label="Rename or move">
                    <ActionIcon variant="light" color="obsidian" onClick={onRename} aria-label="Rename or move">
                        <Icon path={iconEdit} size="sm" />
                    </ActionIcon>
                </Tooltip>

                <Tooltip label="Delete">
                    <ActionIcon variant="light" color="red" onClick={onDelete} aria-label="Delete">
                        <Icon path={iconTrash} size="sm" />
                    </ActionIcon>
                </Tooltip>
            </Group>
        </Group>
    );
}

const border = { borderBottom: "1px solid var(--mantine-color-obsidian-7)", flexShrink: 0 };
