import { Box, Button, Image, Text } from "@mantine/core";
import {
    CodeBlock,
    CodeEditor,
    Icon,
    MarkdownViewer,
    Spinner,
    iconDownload,
    useEditor,
} from "@surrealdb/ui";
import { type ReactNode, useEffect, useState } from "react";

import { api, formatSize, isJson, isText, raw } from "../api";
import type { Mode } from "../App";
import type { Entry } from "../types";

/** What `useEditor` wants for the file on screen, if it knows the language. */
const LANGUAGES: Record<string, "json" | "yaml" | "html" | "markdown" | "python" | "bash"> = {
    "application/json": "json",
    "text/html": "html",
    "text/markdown": "markdown",
    "text/x-python": "python",
    "text/x-sh": "bash",
    "text/yaml": "yaml",
    "application/x-yaml": "yaml",
};

const pad = { padding: "var(--mantine-spacing-lg)" };

export function Viewer({
    entry,
    entries,
    mode,
    onEdit,
    onReady,
    onError,
}: {
    entry: Entry | null;
    entries: Entry[];
    mode: Mode;
    onEdit: (text: string) => void;
    onReady: () => void;
    onError: (message: string) => void;
}) {
    const ct = entry?.content_type ?? "";
    // Only the branches that show the bytes need them.
    const wantsText =
        !!entry && !entry.is_folder && isText(ct) && (mode === "source" || ct !== "text/html");
    const [text, setText] = useState<string | null>(null);

    useEffect(() => {
        if (!entry) return;
        if (!wantsText) {
            onReady();
            return;
        }
        let live = true;
        api(raw(entry.path))
            .then((r) => r.text())
            .then((body) => {
                if (!live) return;
                setText(body);
                onReady();
            })
            .catch((e: Error) => live && onError(e.message));
        return () => {
            live = false;
        };
        // The whole viewer is remounted when the file, the mode or the content
        // changes, so this runs once per view.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    if (!entry)
        return (
            <Body>
                <Text c="dimmed">Pick a file from the tree.</Text>
            </Body>
        );

    if (entry.is_folder) {
        const inside = entries.filter((e) => e.path.startsWith(`${entry.path}/`)).length;
        return (
            <Body>
                <Text c="dimmed">{inside} item(s) inside.</Text>
            </Body>
        );
    }

    if (wantsText && text === null)
        return (
            <Body>
                <Spinner />
            </Body>
        );

    if (mode === "source" && text !== null)
        return <Source text={text} language={LANGUAGES[ct]} onEdit={onEdit} />;

    if (ct === "text/markdown" && text !== null)
        return (
            <Body className="print-body">
                <MarkdownViewer content={text} maw={860} />
            </Body>
        );

    if (ct === "text/html")
        return (
            <iframe
                className="html-preview"
                title={entry.filename}
                src={raw(entry.path)}
                sandbox=""
            />
        );

    if (ct.startsWith("image/"))
        return (
            <Body>
                <Image src={raw(entry.path)} alt={entry.filename} fit="contain" />
            </Body>
        );

    if (text !== null) {
        // Stored JSON is one long line -- pydantic-ai's dump_json is compact.
        // Anything that will not parse is shown exactly as stored.
        let body = text;
        if (isJson(ct))
            try {
                body = JSON.stringify(JSON.parse(text), null, 2);
            } catch {
                /* as-is */
            }
        return (
            <Body>
                <CodeBlock value={body} lang={LANGUAGES[ct] ?? "text"} />
            </Body>
        );
    }

    return (
        <Body>
            <Text c="dimmed" mb="md">
                {ct} · {formatSize(entry.size)} — no preview.
            </Text>
            <Button
                component="a"
                href={raw(entry.path)}
                download={entry.filename}
                leftSection={<Icon path={iconDownload} size="sm" />}
            >
                Download
            </Button>
        </Body>
    );
}

/** The editable source view. Its own component so the controller is per-file. */
function Source({
    text,
    language,
    onEdit,
}: {
    text: string;
    language?: "json" | "yaml" | "html" | "markdown" | "python" | "bash";
    onEdit: (text: string) => void;
}) {
    const controller = useEditor({
        document: text,
        language,
        lineNumbers: true,
        // CodeMirror reports setting the initial document as a change, which
        // would light up "unsaved" before the user has touched anything.
        onChangeDocument: (next) => next !== text && onEdit(next),
    });
    return <CodeEditor controller={controller} flex={1} mih={0} autoFocus />;
}

function Body({ children, className }: { children: ReactNode; className?: string }) {
    return (
        <Box flex={1} mih={0} style={{ ...pad, overflow: "auto" }} className={className}>
            {children}
        </Box>
    );
}
