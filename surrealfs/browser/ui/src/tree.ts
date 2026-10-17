import type { Entry } from "./types";

export interface Node {
    value: string;
    label: string;
    /** Absent for an implicit folder -- a path segment with no row of its own. */
    entry?: Entry;
    children: Node[];
}

export const isDir = (node: Node) => !!node.entry?.is_folder || node.children.length > 0;

/**
 * The flat `/api/tree` list as a tree, synthesising the folders that have no row
 * of their own, and sorting folders first.
 */
export function toTreeData(entries: Entry[]): Node[] {
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
