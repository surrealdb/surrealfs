# surrealfs

Rust library and CLI for a SurrealDB-backed filesystem facade. See `PYTHON.md` for Python binding instructions.

## Curl piping examples

- Save a URL directly to a path: `curl https://example.com > /pages/example.html`
- Use the pipeline form: `curl https://example.com | write_file /pages/example.html`
- Include headers or data and still redirect: `curl -H "Accept: application/json" https://api.example.com/items > /data/items.json`
