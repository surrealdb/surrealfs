import { ActionIcon, Badge, Box, Button, Group, Paper, Stack, Text, Textarea } from "@mantine/core";
import { Icon, MarkdownViewer, Spinner, iconPlus, iconSend } from "@surrealdb/ui";
import { useCallback, useEffect, useRef, useState } from "react";

import { api, json } from "../api";
import type { Bubble } from "../types";

const HINT =
    "Ask the agent to take notes. It reads and writes the same filesystem you are browsing.";

export function Chat({
    openPath,
    replay,
    onTurnDone,
    onError,
}: {
    openPath: string | null;
    /** A `/_sessions/…` file is open: replay it instead of the live conversation. */
    replay: string | null;
    onTurnDone: () => Promise<void>;
    onError: (message: string) => void;
}) {
    const [bubbles, setBubbles] = useState<Bubble[]>([]);
    const [text, setText] = useState("");
    const [busy, setBusy] = useState(false);
    // The panel holds only the path: the history itself lives in the file, so the
    // server reads it per turn and nothing here can drift out of sync with it.
    const [session, setSession] = useState<string | null>(null);
    const scroller = useRef<HTMLDivElement>(null);
    const box = useRef<HTMLTextAreaElement>(null);

    useEffect(() => {
        const el = scroller.current;
        if (el) el.scrollTop = el.scrollHeight;
    });

    const newChat = useCallback(() => {
        setSession(null);
        setBubbles([]);
    }, []);

    // A conversation is a file too: opening one replays it here.
    useEffect(() => {
        if (!replay) return;
        let live = true;
        json<Bubble[]>(`/api/session?path=${encodeURIComponent(replay)}`)
            .then((stored) => {
                if (!live) return;
                setSession(replay);
                setBubbles(stored.map((b) => ({ ...b, done: true })));
            })
            .catch((e: Error) => live && onError(e.message));
        return () => {
            live = false;
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [replay]);

    const send = async () => {
        const message = text.trim();
        if (!message || busy) return;
        setText("");
        setBubbles((b) => [
            ...b,
            { who: "you", text: message, done: true },
            { who: "agent", text: "", tools: [], done: false },
        ]);
        // Everything below appends to the bubble this turn just pushed.
        const patch = (fn: (b: Bubble) => Bubble) =>
            setBubbles((all) => all.map((b, i) => (i === all.length - 1 ? fn(b) : b)));

        setBusy(true);
        try {
            const res = await api("/api/chat", {
                method: "POST",
                headers: { "content-type": "application/json" },
                // Nearly every question is about the file on screen, so the agent
                // gets its path -- not its content: it has `cat`, and the file may
                // be huge.
                body: JSON.stringify({ message, path: openPath, session }),
            });
            // NDJSON: one JSON object per line, and a chunk can split a line.
            const reader = (res.body as ReadableStream<Uint8Array>).getReader();
            const decoder = new TextDecoder();
            let buffer = "";
            for (;;) {
                const { value, done } = await reader.read();
                buffer += done ? "\n" : decoder.decode(value, { stream: true });
                const lines = buffer.split("\n");
                buffer = lines.pop() as string;
                for (const line of lines.filter(Boolean)) {
                    const event = JSON.parse(line);
                    if (event.delta)
                        patch((b) => ({ ...b, text: (b.text + event.delta).trimStart() }));
                    else if (event.tool)
                        patch((b) => ({ ...b, tools: [...(b.tools ?? []), event.tool] }));
                    else if (event.error) throw new Error(event.error);
                    // The turn was stored under `event.session` -- adopt it so the
                    // next message continues the same file rather than starting
                    // another. Deltas render as text; the finished reply is the
                    // model's markdown, rendered once it is whole.
                    else if (event.done) {
                        setSession(event.session);
                        patch((b) => ({ ...b, done: true }));
                    }
                }
                if (done) break;
            }
            await onTurnDone();
        } catch (e) {
            onError((e as Error).message);
        } finally {
            setBusy(false);
            box.current?.focus();
        }
    };

    return (
        <Box
            className="pane no-print"
            bg="obsidian.8"
            pos="relative"
            style={{ borderLeft: "1px solid var(--mantine-color-obsidian-6)" }}
        >
            <Resizer />

            <Group
                justify="space-between"
                p="md"
                style={{ borderBottom: "1px solid var(--mantine-color-obsidian-7)" }}
            >
                <Text size="xs" tt="uppercase">
                    <span className="brand-mark">Agent chat</span>
                </Text>
                <Button
                    size="compact-xs"
                    variant="light"
                    color="obsidian"
                    disabled={busy}
                    leftSection={<Icon path={iconPlus} size="xs" />}
                    onClick={newChat}
                >
                    New
                </Button>
            </Group>

            <Box ref={scroller} flex={1} mih={0} p="md" style={{ overflow: "auto" }}>
                {bubbles.length ? (
                    <Stack gap="md">
                        {bubbles.map((bubble, i) => (
                            // Bubbles are only ever appended to, and the last one is
                            // the one being written -- the index is a stable key.
                            // biome-ignore lint/suspicious/noArrayIndexKey: append-only
                            <Paper key={i} p="sm" bg={bubble.who === "you" ? "obsidian.7" : undefined}>
                                <Text size="xs" tt="uppercase" c="dimmed" mb={4}>
                                    {bubble.who}
                                </Text>
                                {bubble.tools?.length ? (
                                    <Group gap={4} mb="xs">
                                        {bubble.tools.map((tool, t) => (
                                            // biome-ignore lint/suspicious/noArrayIndexKey: append-only
                                            <Badge key={t} size="xs" variant="light" tt="none">
                                                {tool}
                                            </Badge>
                                        ))}
                                    </Group>
                                ) : null}
                                {bubble.who === "agent" && bubble.done ? (
                                    <MarkdownViewer content={bubble.text} fz="sm" />
                                ) : (
                                    <Text size="sm" style={{ whiteSpace: "pre-wrap" }}>
                                        {bubble.text}
                                    </Text>
                                )}
                                {bubble.who === "agent" && !bubble.done && !bubble.text ? (
                                    <Spinner size="sm" />
                                ) : null}
                            </Paper>
                        ))}
                    </Stack>
                ) : (
                    <Text size="sm" c="dimmed">
                        {HINT}
                    </Text>
                )}
            </Box>

            <Group
                p="sm"
                gap="xs"
                align="flex-end"
                wrap="nowrap"
                style={{ borderTop: "1px solid var(--mantine-color-obsidian-7)" }}
            >
                <Textarea
                    ref={box}
                    flex={1}
                    variant="surreal"
                    autosize
                    minRows={3}
                    maxRows={10}
                    spellCheck={false}
                    placeholder="Ask the agent…  (Enter to send, Shift+Enter for a newline)"
                    value={text}
                    onChange={(e) => setText(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter" && !e.shiftKey) {
                            e.preventDefault();
                            void send();
                        }
                    }}
                />
                <ActionIcon
                    variant="surreal"
                    size="lg"
                    aria-label="Send"
                    disabled={busy || !text.trim()}
                    onClick={() => void send()}
                >
                    <Icon path={iconSend} size="sm" />
                </ActionIcon>
            </Group>
        </Box>
    );
}

/**
 * The chat pane is a grid track, so one CSS variable resizes it. Pointer capture
 * keeps the drag alive over the sandboxed iframe preview, which would otherwise
 * swallow the pointermove.
 */
function Resizer() {
    const set = (w: number) => {
        const clamped = Math.min(Math.max(w, 240), Math.max(240, window.innerWidth - 520));
        document.documentElement.style.setProperty("--chat-w", `${clamped}px`);
        return clamped;
    };

    useEffect(() => {
        const stored = localStorage.getItem("chatWidth");
        if (stored) set(+stored);
    }, []);

    return (
        <div
            className="drag"
            onPointerDown={(e) => {
                e.preventDefault();
                const strip = e.currentTarget;
                const drag = (ev: PointerEvent) =>
                    localStorage.setItem("chatWidth", String(set(window.innerWidth - ev.clientX)));
                strip.setPointerCapture(e.pointerId);
                strip.addEventListener("pointermove", drag);
                strip.addEventListener(
                    "pointerup",
                    () => strip.removeEventListener("pointermove", drag),
                    { once: true },
                );
            }}
        />
    );
}
