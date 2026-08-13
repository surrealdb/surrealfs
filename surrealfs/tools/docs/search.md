Search the contents of every text file in SurrealFS.

SurrealFS is a filesystem stored in a SurrealDB table, not the machine's local
disk, so this searches only files written through these tools — never the local
working tree.

Word matching ignores case and common word endings, so "running" also matches
"run". A file is returned when it shares *any* word with your query, and results
come back best-first with a short snippet each: rarer words count for more, so the
files that match the distinctive part of your query lead. The ones further down may
share only a common word, so read the snippets rather than assuming the first hit is
right.

Plain language works, but the words you expect to find in the file work better —
matching is on words, not meaning, unless this filesystem has been indexed for
semantic search.

When this filesystem has been indexed for semantic search, the ranking also
includes files that match the *meaning* of your query even when they share no
vocabulary with it — "how do I get paid" can surface a file about invoicing.

Use this when you know roughly what a file says but not what it is called; use
`glob` when you know the name.
