Atomically append text to the end of a file in SurrealFS.

Useful for logs, journals, task queues, and audit trails where multiple agents or turns append entries without overwriting previous content.

Optionally pass `if_generation` to guarantee optimistic concurrency and prevent conflicting writes.
