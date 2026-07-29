Replace text inside an existing file.

Finds `old` and replaces it with `new`, returning a unified diff of the change.
By default only the first occurrence is replaced; set `replace_all` to true to
replace every occurrence.

`old` must appear in the file exactly. If it does not, the call fails and
nothing is written — re-read the file with `cat` and try again with the exact
text.
