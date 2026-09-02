import { Button, Group, Modal, TextInput } from "@mantine/core";
import { useEffect, useState } from "react";

export interface PathPrompt {
    title: string;
    label: string;
    value: string;
    onSubmit: (path: string) => void;
}

/** `window.prompt` for a path, in the design system's dialog. */
export function PathModal({
    prompt,
    onClose,
}: {
    prompt: PathPrompt | null;
    onClose: () => void;
}) {
    const [value, setValue] = useState("");

    useEffect(() => {
        if (prompt) setValue(prompt.value);
    }, [prompt]);

    const submit = () => {
        const path = value.trim();
        onClose();
        if (path) prompt?.onSubmit(path);
    };

    return (
        <Modal opened={!!prompt} onClose={onClose} title={prompt?.title ?? ""}>
            <TextInput
                data-autofocus
                label={prompt?.label}
                value={value}
                spellCheck={false}
                onChange={(e) => setValue(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && submit()}
            />
            <Group justify="flex-end" mt="lg">
                <Button variant="light" color="obsidian" onClick={onClose}>
                    Cancel
                </Button>
                <Button onClick={submit} disabled={!value.trim()}>
                    {prompt?.title}
                </Button>
            </Group>
        </Modal>
    );
}
