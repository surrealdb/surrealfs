List the contents of a folder in SurrealFS.

SurrealFS is a filesystem stored in a SurrealDB table, not the machine's local
disk. Nothing here is visible to a shell, and any other tools you have for
reading or writing local files address a different filesystem entirely. It
persists across sessions, so treat it as your own durable storage.

Paths are absolute; there is no working directory. `path` defaults to the root
`/`. Set `recursive` to descend into subfolders.

Output is one entry per line: a size in bytes, then the path. Folders show `-`
for size and end in `/`.
