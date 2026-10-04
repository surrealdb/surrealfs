# SurrealFS Menu (`apps/macos-menu`)

Native macOS menu bar application for SurrealFS (§17).

## Features

- **Status Bar Icon & State**: Visual connection indicator (🟢 Connected, 🟡 Connecting, 🔴 Disconnected).
- **One-Click FUSE Mount**: Status indicator for mount path, branch status, and instant "Open in Finder" and "Open in Terminal".
- **Connection Profiles**: Switch between SurrealDB Cloud (`wss://cloud.surreal.io`) and local dev instances, stored securely with Keychain tokens.
- **Live Activity HUD**: Streams real-time file updates, locks, and branch actions as autonomous agents work in the brain, with one-click "Undo".
- **Global Spotlight Search (`⌘⇧S`)**: Floating search window with full-text and vector search, syntax-highlighted previews, and keyboard navigation.
- **Preferences**: Connection profile manager, auto-mount triggers, and global hotkeys.

## Building & Testing

```bash
cd apps/macos-menu
swift build
swift test
```
