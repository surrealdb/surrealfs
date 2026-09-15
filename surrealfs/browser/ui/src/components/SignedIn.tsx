import { Anchor, Group, Text } from "@mantine/core";
import { useEffect, useState } from "react";

import { json } from "../api";

/**
 * Who you are signed in as, on the browser that has a login.
 *
 * Renders nothing at all on the plain `surrealfs-browser`, where `/api/whoami`
 * does not exist: one UI build serves both, so this asks rather than being told
 * which one it is running against.
 */
export function SignedIn() {
    const [user, setUser] = useState<string | null>(null);

    useEffect(() => {
        let live = true;
        json<{ user: string }>("/api/whoami")
            // A 404 is the single-user browser, not a failure worth reporting.
            .then((r) => live && setUser(r.user))
            .catch(() => {});
        return () => {
            live = false;
        };
    }, []);

    if (!user) return null;

    return (
        <Group gap="xs" mt={6} wrap="nowrap" justify="space-between">
            <Text size="xs" c="dimmed" truncate title={user}>
                {user}
            </Text>
            {/* Cloudflare Access's own logout, which clears the session cookie
                it set. There is nothing of ours to sign out of. */}
            <Anchor href="/cdn-cgi/access/logout" size="xs" c="dimmed">
                Sign out
            </Anchor>
        </Group>
    );
}
