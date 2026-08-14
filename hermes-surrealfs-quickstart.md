# Hermes × SurrealFS Quickstart

1. Install Hermes

```bash
curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash
```

[Hermes Installation](https://hermes-agent.nousresearch.com/docs/getting-started/installation)

2. Setup

```
How would you like to set up Hermes?
  ↑↓ navigate  ENTER/SPACE select  ESC cancel

 → (●) Quick Setup (Nous Portal) — free OAuth login, no API keys, model + tools (recommended)
   (○) Full setup — configure every provider, tool & option yourself (bring your own keys)
   (○) Blank Slate — everything off except the bare minimum; opt in to each capability
```

- Quick Setup
- Model: tencent/hy3
- Terminal backend: Local
- **Optional:** set up messaging
- **Optional:** install gateway as a background service

```
✓ Installation Complete!
```

3. Install SurrealFS

```bash
hermes plugins install surrealdb/surrealfs/surrealfs/integrations/hermes_memory
```

```bash
hermes memory setup surrealfs-memory
```

4. Read the integration docs

- [SurrealFS Hermes memory provider](surrealfs/integrations/hermes_memory/README.md)
  — configuration, homes and profiles, what gets filed and recalled.
- [SurrealFS Hermes toolset](surrealfs/integrations/hermes/README.md) — the
  `surrealfs_*` tools and the bundled notes skill, installed separately.
