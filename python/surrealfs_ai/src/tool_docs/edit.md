Edit

Replace text inside a SurrealFs file. Provide the target path, the text to find, and the replacement text. Set `replace_all` to true to replace every occurrence; otherwise only the first match is replaced.

Usage:
- `path`: absolute or relative to the current working directory inside SurrealFs.
- `old`: substring or pattern to replace.
- `new`: replacement text.
- `replace_all`: boolean, default false.

Common errors:
- Path does not exist or points to a directory.
- No occurrence of `old` found (operation may return unchanged content).
