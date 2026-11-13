Write binary data (base64-encoded in `data_base64`) to a path in SurrealFS.

Creates the file or replaces it entirely, creating missing parent folders. Set
`content_type` to the correct media type, e.g. `image/png`.

The whole file travels in one call, so this suits small files. There is no
tool for writing a large file in pieces.
