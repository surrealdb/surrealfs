import { useCallback, useRef } from "react";
import { useConfirmation } from "@surrealdb/ui";

/**
 * `window.confirm`, in the design system's dialog.
 *
 * The kit's `useConfirmation` is callback-shaped, but every use here is a guard
 * in the middle of an async handler -- `if (!(await confirm(...))) return`. Both
 * of its exits are wired up, so the promise always settles: `onDismiss` fires for
 * the Close button, the X, escape and the overlay alike.
 */
export function useConfirm(): (message: string, confirmText?: string) => Promise<boolean> {
    const settle = useRef<(ok: boolean) => void>(() => {});
    const ask = useConfirmation<{ message: string; confirmText: string }>({
        message: (v) => v.message,
        confirmText: (v) => v.confirmText,
        confirmProps: { color: "red" },
        onConfirm: () => settle.current(true),
        onDismiss: () => settle.current(false),
    });

    return useCallback(
        (message: string, confirmText = "Continue") =>
            new Promise<boolean>((resolve) => {
                settle.current = resolve;
                ask({ message, confirmText });
            }),
        [ask],
    );
}
