List the contents of a folder in SurrealFS.

SurrealFS is a filesystem stored in a SurrealDB table, not the machine's local
disk. Nothing here is visible to a shell, and any other tools you have for
reading or writing local files address a different filesystem entirely. It
persists across sessions, so treat it as your own durable storage.

Paths are absolute; there is no working directory. `path` defaults to the root
`/`. Set `recursive` to descend into subfolders.

Output is one entry per line: size in bytes, then the absolute path. Folders
show `-` for size and end in `/`.

           -  /home/alice/
          11  /projects/notes.md

Set `long` to prefix each line with the unix mode and the owner, as `ls -l`
does — worth it when permissions are the question, since `chmod` is how you
change them.

    drwx------  alice            -  /home/alice/
    -rw-rw-rw-  bob             11  /projects/notes.md

Listing a folder shows every name in it, but a folder you may not enter is not
descended into, so a recursive listing stops at other users' homes.
