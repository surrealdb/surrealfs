import { ActionIcon, Group, SegmentedControl, Text, Tooltip } from "@mantine/core";
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
    view,
    onMode,
    onViewChange,
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
    view: "editor" | "canvas";
    onMode: (mode: Mode) => void;
    onViewChange: (view: "editor" | "canvas") => void;
    onSave: () => void;
    onRename: () => void;
    onDelete: () => void;
    onPrint: () => void;
    onOpen: (path: string) => void;
}) {
    if (!current)
        return (
            <Group className="toolbar-header no-print" justify="space-between">
                <Group gap="sm">
                    <Text size="xs" className="mono" style={{ color: "#fff", fontWeight: 600 }}>
                        {view === "canvas" ? "Spatial Brain Studio" : "SurrealFS"}
                    </Text>
                    <span className={view === "canvas" ? "status keep" : "status new"}>
                        {view === "canvas" ? "2D Graph" : "Overview"}
                    </span>
                </Group>
                <SegmentedControl
                    size="xs"
                    value={view}
                    onChange={(val) => onViewChange(val as "editor" | "canvas")}
                    data={[
                        { label: "Document", value: "editor" },
                        { label: "Brain Studio ✦", value: "canvas" },
                    ]}
                />
            </Group>
        );

    const ct = current.content_type;
    const segments = current.path.split("/").filter(Boolean);
    const known = new Set(entries.map((e) => e.path));

    return (
        <Group className="toolbar-header no-print" gap="xs" wrap="nowrap">
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
                            style={{ flexShrink: last ? 1 : 0 }}
                        >
                            {name}
                        </BreadcrumbButton>
                    );
                })}
            </Group>

            {dirty ? <span className="status new">unsaved</span> : null}

            <Text size="xs" className="mono" style={{ color: "var(--mute)", whiteSpace: "nowrap" }}>
                {current.is_folder ? "folder" : `${ct} · ${formatSize(current.size)}`}
            </Text>

            <Group gap="xs" ml="auto" wrap="nowrap">
                <SegmentedControl
                    size="xs"
                    value={view}
                    onChange={(val) => onViewChange(val as "editor" | "canvas")}
                    data={[
                        { label: "Document", value: "editor" },
                        { label: "Brain Studio ✦", value: "canvas" },
                    ]}
                />

                {view === "editor" && !current.is_folder && hasPreview(ct) ? (
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

                {view === "editor" && ct === "text/markdown" ? (
                    <Tooltip label="Print / save as PDF">
                        <ActionIcon
                            variant="light"
                            color="obsidian"
                            onClick={onPrint}
                            aria-label="Print or save as PDF"
                            style={{ border: "1px solid var(--line)" }}
                        >
                            <Icon path={iconDownload} size="sm" />
                        </ActionIcon>
                    </Tooltip>
                ) : null}

                {view === "editor" && !current.is_folder && isText(ct) ? (
                    <Tooltip label="Save (⌘S)">
                        <ActionIcon
                            variant="filled"
                            color="pink"
                            disabled={!dirty}
                            onClick={onSave}
                            aria-label="Save"
                            style={{ background: dirty ? "var(--pink)" : undefined }}
                        >
                            <Icon path={iconFloppy} size="sm" />
                        </ActionIcon>
                    </Tooltip>
                ) : null}

                {view === "editor" && (
                    <Tooltip label="Rename or move">
                        <ActionIcon
                            variant="light"
                            color="obsidian"
                            onClick={onRename}
                            aria-label="Rename or move"
                            style={{ border: "1px solid var(--line)" }}
                        >
                            <Icon path={iconEdit} size="sm" />
                        </ActionIcon>
                    </Tooltip>
                )}

                {view === "editor" && (
                    <Tooltip label="Delete">
                        <ActionIcon
                            variant="light"
                            color="red"
                            onClick={onDelete}
                            aria-label="Delete"
                            style={{ border: "1px solid var(--line)" }}
                        >
                            <Icon path={iconTrash} size="sm" />
                        </ActionIcon>
                    </Tooltip>
                )}
            </Group>
        </Group>
    );
}
