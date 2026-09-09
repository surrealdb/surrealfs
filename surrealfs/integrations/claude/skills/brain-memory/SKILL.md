---
name: brain-memory
description: Reach what the company brain's files no longer say — a superseded version of a risk, a relationship nobody wrote down, context filed by a session you never saw — using the brain_recall tool. Use it only when that tool is present; it is absent unless Spectron is configured, and then the `brain` skill alone is the whole answer. Read the files first with `brain`, then come here for the why and the history behind them.
---

# Recalling from Spectron

**Requires the `brain_recall` tool.** It exists only when the `surrealfs` server
has a Spectron key. If you do not see it, stop here: use the `brain` skill and
answer from the files, which are complete on their own.

Spectron is the memory layer behind SurrealFS. Every text file written through the
server is mirrored into it automatically, so it keeps every version that was ever
filed plus the entities and relationships it extracted from the prose. That is
what makes it worth asking: it answers what the *current* files no longer say.

## How to use it

1. **Read the files first.** `ls` and `cat` under `/brain/` per the `brain` skill.
   Recall is not a search over the files — `search` is that. Calling recall
   instead of reading them gives you a worse answer more slowly.
2. **Ask a question, not a keyword.** "What is blocking the SOC2 audit" recalls;
   "SOC2" does not. Each hit comes back labelled with the SurrealFS path it was
   filed from, so open that file next when it looks relevant.
3. **Say where a claim came from.** If a claim came *only* from recall, mark it as
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

- It is behind the files, not instead of them. An answer built on recall alone,
  with the current files unread, is usually out of date.
- Nothing you say in chat reaches it. Only what you *write* through the SurrealFS
  tools gets mirrored — so file the outcome.
- `write_bytes` is not mirrored. Spectron indexes prose.
