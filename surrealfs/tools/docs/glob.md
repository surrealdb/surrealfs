Find files in SurrealFS whose path matches a shell-style glob.

Supports `*` (within one path segment), `**` (across segments), `?`, and
character classes like `[abc]`. Examples: `/notes/**/*.md`, `/projects/*/todo.md`.

Prefer this over `ls` with `recursive` when you know roughly what a file is
called but not where it lives.
