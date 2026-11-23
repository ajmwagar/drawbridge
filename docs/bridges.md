# Bridges built on Drawbridge

Drawbridge is the shared core, not a monorepo for every bridge. A bridge stays
in its own repo and supplies:

1. a native network adapter (Matrix appservice, MeshCore sidecar, Rakazo HTTP),
2. a Soil Spore for routing/policy, and
3. a package manifest that links the Drawbridge host capability.

## Current consumers

| Repo | Network | Today | Next |
| --- | --- | --- | --- |
| [`meshcore-drawbridge`](https://github.com/ajmwagar/meshcore-drawbridge) | MeshCore sidecar HTTP | Rust `meshcore-mautrix` runtime, private channels, slash commands, owner auto-join | `drawbridge-host` backend + Wasm adapter + `spores/meshcore-bridge` |
| [`rakazo-drawbridge`](https://github.com/FuturePresentLabs/rakazo-drawbridge) | Rakazo HTTP/WS | Rust `rakazo-beeper` runtime with agents, history, avatars, ack policy | extract text/mapping/ack policy into a Soil Spore; keep Rakazo media/history in the adapter |

## Migration order

### MeshCore

The smallest vertical and the reference for the host contract:

1. Extract the MeshCore client into `drawbridge-meshcore`.
2. Implement `drawbridge-host` methods: `status`, `poll`, `send`, `deliver`,
   `bind`, `bindings`, `claim`, `commit`, `rollback`.
3. Build the Wasm adapter exporting `ajmwagar:drawbridge`.
4. Port `meshcore-mautrix`'s `runtime.rs` routing and commands to
   `spores/meshcore-bridge`.
5. Cut over one tenant; keep the Rust binary as fallback.

### Rakazo

Do not port the whole app. Strangle the policy first:

1. Keep Rakazo HTTP, media, avatars and history in the native adapter.
2. Move room mapping, formatting, commands and ack policy to the Soil Spore.
3. Port one tenant at a time; the Rust app remains the fallback.
4. Only then consider moving history/backfill; never move tokens into the guest.

## Other bridges in the fleet (not Drawbridge consumers yet)

- [`mautrix-mattermost`](https://github.com/FuturePresentLabs/mautrix-mattermost) — fork; not ported.
- [`bifrost`](https://github.com/FuturePresentLabs/bifrost) — LLM gateway/bridge, private; different contract.
- [`beeper-bridges`](https://github.com/FuturePresentLabs/beeper-bridges) — OpenTofu deployment for self-hosted Beeper bridges, not a bridge itself.
- `bd-forest` and `inception` are registry/orchestration bridges, not messaging bridges.
