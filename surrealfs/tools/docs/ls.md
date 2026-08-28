List the contents of a folder in SurrealFS.

SurrealFS is a filesystem stored in a SurrealDB table, not the machine's local
disk. Nothing here is visible to a shell, and any other tools you have for
reading or writing local files address a different filesystem entirely. It
persists across sessions, so treat it as your own durable storage.

Paths are absolute; there is no working directory. `path` defaults to the root
`/`. Set `recursive` to descend into subfolders.

Output is one entry per line, like `ls -l`: permissions, owner, size in bytes,
then the path. Folders show `-` for size and end in `/`.

    drwx------  alice            -  /home/alice/
    -rw-rw-rw-  bob             11  /projects/notes.md

Permissions are unix mode bits — see `chmod`, which is how you change them.
Listing a folder shows every name in it, but a folder you may not enter is not
descended into, so a recursive listing stops at other users' homes.
