Read a binary file from SurrealFS and return its contents base64-encoded.

Use this for images and other non-text files. For text, prefer `cat`, which
returns readable content directly.

At most 64 KiB (65536 bytes) come back per call, whatever the file's size, so a
large file is read in pieces: pass `offset` (default 0) and `length` (default and
maximum 64 KiB). The reply is JSON: `data_base64`, the file's total `size` in
bytes, the `offset` it started at, and `next_offset` — the offset to read from
next, or `null` once the file is exhausted. An `offset` at or past the end
returns empty data.
