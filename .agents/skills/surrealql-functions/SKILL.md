---
name: surrealql-functions
description: "Discover and use SurrealDB's built-in SurrealQL functions with accurate, version-current signatures by running the SurrealQL language server (LSP) and tree-sitter grammar, with a namespace catalog linking every function group to its docs. Use when looking up which built-in function to use, its exact signature/arguments, getting editor completions/hover/signature-help for .surql files, or confirming a function exists in the installed SurrealDB version. Triggers: SurrealQL function, built-in function, function signature, string::, array::, math::, type::, LSP, language server, completions, surrealql-language-server."
metadata:
  author: surrealdb
  version: "0.1.0"
---

# SurrealQL Functions

SurrealDB ships hundreds of built-in functions grouped into namespaces
(`string::`, `array::`, `math::`, `type::`, …). Function names and signatures
change between releases, so the authoritative, **version-current** source is the
SurrealQL tooling rather than memory:

- [surrealql-language-server](https://github.com/surrealdb/surrealql-language-server) (LSP) — completions for built-in functions, hover with type info and function signatures, and signature help.
- [surrealql-tree-sitter](https://github.com/surrealdb/surrealql-tree-sitter) — the grammar the LSP parses with (targets SurrealDB v3+).

For mapping any function to authoritative documentation, see
[references/catalog.md](references/catalog.md).

## When to use this skill

- Looking up which built-in function does what, or its exact arguments
- Getting live completions, hover, and signature help for `.surql` files
- Confirming a function exists / has the expected signature in the installed
  SurrealDB version (avoids emitting renamed or removed functions)

## Setup

Install the language server from crates.io — the published crate bundles the
grammar, so no separate tree-sitter checkout is needed:

```bash
cargo install surrealql-language-server
#   -> installs the `surrealql-language-server` binary on your PATH (~/.cargo/bin)
```

Verify it is on your PATH:

```bash
surrealql-language-server --help
```

### Build from source (optional)

Only needed for development or to track unreleased changes. The source build
requires the [tree-sitter grammar](https://github.com/surrealdb/surrealql-tree-sitter)
checked out as a **sibling** directory:

```bash
git clone https://github.com/surrealdb/surrealql-language-server.git
cd surrealql-language-server
bash scripts/setup-grammar.sh                 # clones/updates the grammar
#   ...or: export TREE_SITTER_SURREALQL_DIR=/abs/path/to/surrealql-tree-sitter
cargo build --release                         # -> target/release/surrealql-language-server
```

For browser/editor embedding, build the wasm-bindgen npm package instead with
`bash scripts/build-wasm.sh` (outputs to `pkg/`).

## Editor integration

Point your editor's LSP client at the `surrealql-language-server` binary for the
`.surql` / `surrealql` language. Generic stdio LSP configuration:

- **Command:** `surrealql-language-server` (or the absolute path from a source build)
- **File types:** `*.surql`

The server then provides, against the installed grammar version:

- **Completions** — built-in function names per namespace, field names,
  `record<table>` types, and keywords
- **Hover** — type info, permission posture, and function signatures
- **Signature help** — argument shapes while typing a call

## Using it to discover functions

- Open or create a `.surql` buffer connected to the LSP.
- Type a namespace prefix (e.g. `string::`) and trigger completion to enumerate
  that namespace's built-in functions for the installed version.
- Hover a function call, or invoke signature help inside the parentheses, to see
  its exact signature and argument types.

Because completions and signatures come from the installed grammar, they reflect
the current SurrealDB version — preventing stale or renamed functions from older
releases. To read full documentation and examples for any namespace, follow the
links in [references/catalog.md](references/catalog.md).
