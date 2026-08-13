Read a text file from SurrealFS and return its full contents.

SurrealFS is a filesystem stored in a SurrealDB table, not the machine's local
disk — this cannot read local files.

Errors if the path does not exist, names a folder, or holds binary data — use
`read_bytes` for binary files.
