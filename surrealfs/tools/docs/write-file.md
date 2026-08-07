Write text to a path in SurrealFS, creating the file or replacing it entirely.

SurrealFS is a filesystem stored in a SurrealDB table, not the machine's local
disk — this never touches local files.

Missing parent folders are created automatically. The media type is inferred
from the file extension unless you pass `content_type`.

This overwrites the whole file. To change part of a file, use `edit` — it is
safer and shows you exactly what changed.
