---
name: brain-memory
description: Ask the company brain's memory layer first, with the brain_recall tool — it answers from every version ever filed plus the entities and relationships extracted out of the prose, and labels each hit with the SurrealFS path it came from, so it finds the files that matter faster than listing folders does. Use it only when that tool is present; it is absent unless agent memory is configured, and then the `brain` skill alone is the whole answer.
---

# Recalling from agent memory

**Requires the `brain_recall` tool.** It exists only when the `surrealfs` server
has an agent memory key. If you do not see it, stop here: use the `brain` skill and
answer from the files, which are complete on their own.

Agent memory is the memory layer behind SurrealFS. Every text file written
through the server is mirrored into it automatically, so it keeps every version
that was ever filed plus the entities and relationships it extracted from the
prose. That is what makes it the place to start: one question tells you both what
the brain knows and which files to open, where `ls` and `cat` only find the second
of those, a folder at a time.

## How to use it

1. **Ask recall first, before reading anything.** A question, not a keyword:
   "What is blocking the SOC2 audit" recalls; "SOC2" does not.
2. **Open the paths the hits name.** Each hit is labelled with the SurrealFS path
   it was mirrored from. `cat` those paths before reporting current state — the
   hit itself may be a superseded version of that file. Recall locates; the file
   confirms.
3. **Fall back to the files** when recall comes back with nothing, or when the
   hits are entities and attributes that name no path: `ls`, `cat` and `search`
   under `/brain/` per the `brain` skill. `search` — not recall — is the keyword
   search over what the files say now.
4. **Say where a claim came from.** If a claim came *only* from recall, mark it as
   such — it means nothing in the filesystem backs it up yet, and that is usually
   worth fixing by writing it down.

## What it is good for

- **Why the current state is what it is.** A risk file says what is true now;
  recall has the versions that said otherwise, and the wording that changed.
- **A risk that was pruned.** A long-resolved risk removed from the board is still
  recallable, so `brain_recall` can rebuild the file if it turns out to matter.
- **What the files omit.** A relationship between two findings that nobody wrote
  down, or a note filed by a routine or a colleague's session you never saw.

## Pitfalls

- A hit is a pointer, not the current truth. Agent memory keeps every version ever
  mirrored and replaces none of them, so the top hit for a risk can be the wording
  that risk had months ago. Answer from recall alone only for history and why;
  for what is true now, open the file the hit names.
- Nothing you say in chat reaches it. Only what you *write* through the SurrealFS
  tools gets mirrored — so file the outcome.
- `write_bytes` is not mirrored. Agent memory indexes prose.
