Search the contents of every text file.

Word matching ignores case and common word endings, so "running" also matches
"run". Results come back best-first, each with a short snippet of surrounding
text.

When this filesystem has been indexed for semantic search, the ranking also
includes files that match the *meaning* of your query even when they share no
vocabulary with it — "how do I get paid" can surface a file about invoicing.

Use this when you know roughly what a file says but not what it is called; use
`glob` when you know the name.
