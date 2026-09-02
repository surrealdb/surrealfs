import { Alert, Box } from "@mantine/core";
import { useHotkeys } from "@mantine/hooks";
import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError, SESSIONS, api, hasPreview, json, send } from "./api";
import { Chat } from "./components/Chat";
import { PathModal, type PathPrompt } from "./components/PathModal";
import { Sidebar } from "./components/Sidebar";
import { Toolbar } from "./components/Toolbar";
import { Viewer } from "./components/Viewer";
import type { Entry } from "./types";
import { useConfirm } from "./useConfirm";

type Status = { text: string; error: boolean } | null;
export type Mode = "preview" | "source";

export function App() {
    const confirm = useConfirm();

    const [entries, setEntries] = useState<Entry[]>([]);
    const [current, setCurrent] = useState<Entry | null>(null);
    const [mode, setMode] = useState<Mode>("preview");
    const [dirty, setDirty] = useState(false);
    const [status, setStatus] = useState<Status>(null);
    const [prompt, setPrompt] = useState<PathPrompt | null>(null);
    // Bumped to remount the viewer: opening a file, or the agent rewriting the
    // one on screen. Saving deliberately does not bump it -- the editor keeps
    // its cursor.
    const [reloadKey, setReloadKey] = useState(0);
    // The editor's text while unsaved. `dirty` and this move together.
    const draft = useRef<string | null>(null);
    // Set when the PDF button had to switch to preview mode first.
    const wantPrint = useRef(false);

    const say = (text: string, error = false) => setStatus({ text, error });

    /** Every handler's error boundary: failures land in the status line. */
    const guard =
        <A extends unknown[]>(fn: (...args: A) => Promise<unknown> | unknown) =>
        async (...args: A) => {
            try {
                setStatus(null);
                await fn(...args);
            } catch (e) {
                say((e as Error).message, true);
            }
        };

    const closeFile = useCallback(() => {
        setCurrent(null);
        setDirty(false);
        draft.current = null;
    }, []);

    /** Reload the flat tree. Returns it, since state lands a render later. */
    const loadTree = useCallback(
        async (keepSelection = true) => {
            const next = await json<Entry[]>("/api/tree");
            setEntries(next);
            setCurrent((open) => {
                if (!open) return open;
                const found = next.find((e) => e.path === open.path);
                if (found) return found;
                if (!keepSelection) return open;
                setDirty(false);
                draft.current = null;
                return null;
            });
            return next;
        },
        [],
    );

    const openFile = useCallback(
        async (path: string) => {
            if (dirty && !(await confirm("Discard unsaved changes?", "Discard"))) return;
            let found = entries.find((e) => e.path === path);
            if (!found) found = (await loadTree(false)).find((e) => e.path === path);
            if (!found) throw new ApiError(`Not found: ${path}`, 404);
            setCurrent(found);
            setDirty(false);
            draft.current = null;
            setMode(hasPreview(found.content_type) ? "preview" : "source");
            setReloadKey((n) => n + 1);
        },
        [confirm, dirty, entries, loadTree],
    );

    const switchMode = useCallback(
        async (next: Mode) => {
            if (next === mode) return;
            if (dirty && !(await confirm("Discard unsaved changes?", "Discard"))) return;
            setDirty(false);
            draft.current = null;
            setMode(next);
            setReloadKey((n) => n + 1);
        },
        [confirm, dirty, mode],
    );

    const save = useCallback(async () => {
        if (!current || draft.current === null) return;
        const saved = await send<Entry>("/api/file", "PUT", {
            path: current.path,
            content: draft.current,
        });
        setDirty(false);
        draft.current = null;
        setCurrent(saved);
        await loadTree();
        say(`Saved ${saved.path}`);
    }, [current, loadTree]);

    const rename = useCallback(
        (dst: string) =>
            guard(async () => {
                if (!current || dst === current.path) return;
                await send("/api/move", "POST", { src: current.path, dst });
                setDirty(false);
                draft.current = null;
                await loadTree(false);
                await openFile(dst);
                say(`Moved to ${dst}`);
            })(),
        [current, loadTree, openFile],
    );

    const remove = useCallback(async () => {
        if (!current) return;
        const path = current.path;
        if (!(await confirm(`Delete ${path}?`, "Delete"))) return;
        const url = `/api/file?path=${encodeURIComponent(path)}`;
        try {
            await api(url, { method: "DELETE" });
        } catch (e) {
            // 409 means a non-empty folder; recursion gets its own confirmation.
            const conflict = e instanceof ApiError && e.status === 409;
            if (
                !conflict ||
                !(await confirm(
                    `${(e as Error).message} — delete it and everything inside?`,
                    "Delete everything",
                ))
            )
                throw e;
            await api(`${url}&recursive=1`, { method: "DELETE" });
        }
        closeFile();
        await loadTree(false);
        say(`Deleted ${path}`);
    }, [closeFile, confirm, current, loadTree]);

    const create = useCallback(
        (path: string, folder: boolean) =>
            guard(async () => {
                await send("/api/file", "POST", { path, folder });
                await loadTree(false);
                if (!folder) await openFile(path);
                say(`Created ${path}`);
            })(),
        [loadTree, openFile],
    );

    /** The new-file/new-folder path, prefilled the way `prompt()` was. */
    const askCreate = (folder: boolean) => {
        const base = !current
            ? "/"
            : current.is_folder
              ? `${current.path}/`
              : current.path.replace(/[^/]*$/, "");
        setPrompt({
            title: folder ? "New folder" : "New file",
            label: "Path",
            value: base,
            onSubmit: (path) => create(path, folder),
        });
    };

    /**
     * Print is the export: "Save as PDF" is a destination in every browser's
     * print dialog, and `@media print` strips the page down to the document.
     */
    const doPrint = useCallback(() => {
        if (!current) return;
        // The title is what the browser uses for the PDF's filename and header.
        const title = document.title;
        document.title = current.filename.replace(/\.md$/, "");
        window.addEventListener("afterprint", () => {
            document.title = title;
        }, { once: true });
        window.print();
    }, [current]);

    const printFile = useCallback(async () => {
        if (!current) return;
        if (mode === "preview") return doPrint();
        if (dirty && !(await confirm("Discard unsaved changes?", "Discard"))) return;
        // The preview has to fetch and render before there is anything to print,
        // so hand off to the viewer: it says when it is ready.
        wantPrint.current = true;
        setDirty(false);
        draft.current = null;
        setMode("preview");
        setReloadKey((n) => n + 1);
    }, [confirm, current, dirty, doPrint, mode]);

    /**
     * The agent may have rewritten what is on screen. Unsaved edits win --
     * reloading the editor underneath the user would throw away their typing.
     */
    const afterAgentTurn = useCallback(async () => {
        await loadTree();
        if (dirty) say("The agent may have changed this file; your unsaved edits were kept.");
        else setReloadKey((n) => n + 1);
    }, [dirty, loadTree]);

    // `triggerOnContentEditable`: the editor is a contenteditable div, and the
    // hook skips those by default -- which is exactly where ⌘S gets pressed.
    useHotkeys([["mod+S", () => dirty && guard(save)()]], [], true);

    useEffect(() => {
        const warn = (e: BeforeUnloadEvent) => {
            if (dirty) e.preventDefault();
        };
        window.addEventListener("beforeunload", warn);
        return () => window.removeEventListener("beforeunload", warn);
    }, [dirty]);

    // Boot.
    useEffect(() => {
        guard(() => loadTree(false))();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    return (
        <Box className="shell">
            <Sidebar
                entries={entries}
                current={current}
                onOpen={guard(openFile)}
                onRefresh={guard(() => loadTree())}
                onCreate={askCreate}
                onError={(message) => say(message, true)}
            />

            <Box className="pane">
                <Toolbar
                    current={current}
                    entries={entries}
                    mode={mode}
                    dirty={dirty}
                    onMode={guard(switchMode)}
                    onSave={guard(save)}
                    onRename={() =>
                        current &&
                        setPrompt({
                            title: "Move",
                            label: "Move to",
                            value: current.path,
                            onSubmit: rename,
                        })
                    }
                    onDelete={guard(remove)}
                    onPrint={guard(printFile)}
                    onOpen={guard(openFile)}
                />

                <Viewer
                    key={`${current?.path ?? ""}:${mode}:${reloadKey}`}
                    entry={current}
                    entries={entries}
                    mode={mode}
                    onEdit={(text) => {
                        draft.current = text;
                        setDirty(true);
                    }}
                    onReady={() => {
                        if (!wantPrint.current) return;
                        wantPrint.current = false;
                        doPrint();
                    }}
                    onError={(message) => say(message, true)}
                />

                {status?.text ? (
                    <Alert
                        className="no-print"
                        color={status.error ? "red" : "surreal"}
                        radius={0}
                        py="xs"
                        withCloseButton
                        onClose={() => setStatus(null)}
                    >
                        {status.text}
                    </Alert>
                ) : null}
            </Box>

            <Chat
                openPath={current?.path ?? null}
                replay={current?.path.startsWith(`${SESSIONS}/`) ? current.path : null}
                onTurnDone={afterAgentTurn}
                onError={(message) => say(message, true)}
            />

            <PathModal prompt={prompt} onClose={() => setPrompt(null)} />
        </Box>
    );
}
