# Drawbridge Soil library

This directory is the canonical Soil abstraction over the Drawbridge native
host. It is the layer a bridge Spore imports:

```
spores/meshcore-bridge          policy, room mapping, commands
        │
        ▼
soil/std/messaging/*            shared messaging helpers
        │
        ▼
soil/bindings/drawbridge.soil   WIT binding for ajmwagar:drawbridge
        │
        ▼
drawbridge-host sidecar         Matrix, MeshCore/Rakazo, SQLite, retries
```

The Rust crates in this repository are the host implementation. The Soil
library is the policy-facing half: it never sees a Matrix token, a sidecar
bearer token, a socket, or the SQLite file.

## Files

| Path | Purpose |
| --- | --- |
| `bindings/drawbridge.soil` | Hand-written binding for `wit/drawbridge.wit`. Regenerate with `soil-witgen` when the WIT changes. |
| `std/messaging/core.soil` | Conversation keys, room naming, allowlists, small helpers. |
| `std/messaging/commands.soil` | Owner command parser/dispatch (`/mesh ...`). |
| `std/messaging/contacts.soil` | Explicit ∪ MeshCore chat contacts ∪ Matrix DM names, deduped. |
| `std/messaging/ack.soil` | Acknowledgement policy, promise identity and the broken-promise notice. |
| `std/rakazo/bots.soil` | Rakazo bot selection: explicit ∪ orchestrator roots/children ∪ named active bots. |

## Status

This is the first scaffold, not a running Spore yet. The native sidecar and the
Wasm adapter are the next implementation step. Until then, the Rust
`meshcore-mautrix` runtime is the reference behaviour and the commands below are
test vectors.

## Design rules

- The Spore owns **which** conversation maps to **which** room and **what** a
  command means.
- The host owns **delivery**, **claims**, **cursors**, **retries** and **secrets**.
- A command is parsed before `send`; it is never forwarded as MeshCore text.
- A private channel key is a shared secret. Never put it in a public room.
