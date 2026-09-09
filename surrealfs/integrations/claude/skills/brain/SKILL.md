---
name: brain
description: Answer questions about the state of a project — what is blocking it, what to do next, what changed — from the company brain in SurrealFS. Use whenever the question is about status, priorities, blockers, risks, or history rather than about code in front of you; and use it before writing anything into /brain, so the file lands in the right place.
---

# The company brain

`/brain/<project>/` in SurrealFS is a shared, persistent record of a project's
security and compliance state. Routines write to it on a schedule; people read it;
other agents read and write it too.

It is **not** the local disk and not this conversation's memory. Reach it only
through the SurrealFS tools on the `surrealfs` server.

## Where things go

```
/brain/acme/sources/snyk/2026-09-04/SNYK-JS-AXIOS-1234.md   one file per finding, per day
/brain/acme/sources/drata/2026-09-04/CC6.1.md               never edited after filing
/brain/acme/risks/okta-sso-cert.md                          live state, edited in place
/brain/acme/digests/2026-09-04.md                           one rollup per day
/brain/acme/decisions/2026-09-04-defer-axios-bump.md        append-only, one per decision
/brain/acme/notes/oncall-handover.md                        anything from a conversation
```

The split matters and is not cosmetic:

- `sources/` is **evidence**, partitioned by date. A routine files what a platform
  said today and never goes back to edit it. Two files disagreeing is signal, not
  a bug — it is how the brain shows that something changed.
- `risks/` is **current state**, one file per risk, *edited* with `edit` rather
  than rewritten. There is exactly one file per risk and it is always the truth as
  of now. Its history is the `decisions/` file that changed it, so a risk whose
  reasoning is not written down anywhere else must not be `rm`'d — the file is the
  only copy.
- `decisions/` is **why**, and is append-only. Never edit a decision; a reversal
  is a new file that says so.
- `digests/` and `notes/` are for people. Write them for a colleague opening the
  file cold, with a date and enough context to stand alone.

Everything under `/brain/` is shared read-write, deliberately: it is how routines,
agents and people hand work to each other. `/home/<user>/` folders are private and
the filesystem enforces it — never put brain material there.

## How to work

1. **Every claim comes from a file you read.** If something already told you which
   paths matter, `cat` those; otherwise `ls` the folders the question touches and
   read what looks relevant, or `search` when you do not know the filename. The
   files are the current state, and reading them is cheap.
2. **Answer with citations.** Name the file each claim came from. A claim no file
   backs up is a claim to check, not to report.
3. **Write the outcome back.** A plan, a decision, an updated risk. Anything you
   only said in chat is gone; anything you file is there for the next session, and
   for whoever opens it next.
4. **Prioritise on evidence.** Severity from the source finding, exposure from
   whether it is reachable in production, urgency from a real date (a certificate
   expiry, an audit window). Say which of those drove the ranking.

## Pitfalls

- Paths are absolute and there is no working directory: `/brain/acme/risks/x.md`,
  never `brain/risks/x.md` or `~/x.md`.
- `write_file` replaces a file entirely. Updating a risk means `edit`, which shows
  you a diff and leaves the rest of the file alone.
- Do not edit anything under `sources/`. If a finding was wrong, file today's
  correction as today's evidence and update the risk.
- One risk, one file. Two files for the same risk and the brain starts
  contradicting itself.
