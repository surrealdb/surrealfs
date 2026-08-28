---
name: notes
description: Keep notes, memories, plans, to-dos, and documents in SurrealFS — a persistent filesystem the user can read too. Use when you learn something worth remembering, when you produce anything the user should be able to open later, or before writing a file so it lands in the right folder.
---

# Notes and memory in SurrealFS

SurrealFS is a filesystem stored in SurrealDB, reached through the `surrealfs_*`
tools. It survives across sessions, so it is where your memory lives.

It is **not** the local disk. Any other file tools you have — `read_file`,
`write_file`, `patch`, `search_files` — address a different filesystem entirely, and
nothing written there is part of this one. When in doubt about which to use: if it is
a note, a memory, a plan, or something you want to find again next session, it
belongs in SurrealFS.

## Why keep things here

- **It persists.** Anything you learn in this conversation is gone next session
  unless you write it down.
- **The user can read it.** They browse the same filesystem, so this is how you hand
  over a plan, a document, a to-do list, or any other artefact. Write for them, not
  just for yourself — a file someone else will open deserves a title and enough
  context to make sense on its own.
- **Any format.** Text through `surrealfs_write_file`, and images, PDFs, or any other
  binary through `surrealfs_write_bytes`.
- **It is searchable.** `surrealfs_search` matches on wording and, when the
  filesystem has been indexed for semantic search, on meaning too — so a note about
  invoicing surfaces for "how do I get paid". You do not have to remember what you
  called a file to find it again.

## Where things go

Paths are absolute and there is no working directory. Use folders; a flat root gets
unusable fast.

```
/home/hermes-martin/todo.md       your own working files
/home/hermes-martin/scratch.md
/home/hermes-martin/memories/     your filed conversations — leave these alone
/preferences/martin/voice.md      how martin prefers to be heard
/projects/surrealfs/todo.md       one folder per project
/projects/surrealfs/notes.md
```

- `/home/<your username>/` is yours — working notes, running to-dos, anything you
  keep for your own benefit. Your username is the account you run as, or whatever
  `SURREALFS_AGENT_USER` sets; the system prompt names the folder your conversations
  are filed in, which is inside it.
- **Other `/home/` folders are not yours, and the filesystem enforces that.** This
  filesystem may be shared with other agents and with the people who own them, and each
  has a home of its own, created `0700`. You will get a permission error rather than a
  file, and search will never return one. Do not work around it.
- `memories/` inside your home is written for you, one file per conversation. Read
  it freely; write your own notes beside it rather than in it.
- `/preferences/<user>/` is what you learn about the user: how they like to be addressed,
  tools they prefer, conventions they have corrected you on. One file per topic beats
  one long file. It sits outside `/home/` because it is shared — this is where you
  hand something to the user, or to another agent working for them.
- `/projects/<name>/` is per-project work — notes, plans, and that project's to-do
  list together in one folder. Shared, for the same reason.
- **Everything outside `/home/` is shared read-write by default**, which is the point:
  it is how you hand work to the user and to other agents. `surrealfs_ls` shows each
  entry's permissions and owner. If something genuinely should not be read by others,
  `surrealfs_chmod 700` it — but your home is already private, so put it there instead
  of locking down a shared folder.

## How to work

1. **Orient before assuming.** `surrealfs_ls /` at the start of a session shows what
   already exists. Do not read a file blind on the guess that it is there.
2. **Search before creating.** `surrealfs_search` first — the note you are about to
   write may already exist, and updating it beats leaving two half-notes behind.
3. **Edit, do not overwrite.** `surrealfs_edit` changes part of a file and shows you
   a diff. `surrealfs_write_file` replaces the whole thing, so reach for it when
   creating a file or genuinely rewriting one.
4. **Write as you go.** Recording something when you learn it is more reliable than
   trying to reconstruct the conversation at the end.

## Pitfalls

- Paths must be absolute: `/preferences/martin/voice.md`, never
  `preferences/voice.md` or `~/notes.md`. There is no `cd`.
- `surrealfs_write_file` on an existing path replaces its entire contents. If you
  meant to add a line, use `surrealfs_edit`.
- `surrealfs_rm` cannot be undone, and needs `recursive` to remove a folder that has
  anything in it.
- Folders are created for you on write, so there is no need to `surrealfs_mkdir`
  first unless you want an empty one.
