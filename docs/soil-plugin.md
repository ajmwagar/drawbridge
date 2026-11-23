# Drawbridge as a Loam capability (Soil bridge logic)

Short answer: **yes, and the boundary should be coarse.** Put transport,
credentials, durability and Matrix appservice mechanics in the native
Drawbridge host; put routing and policy in a Soil Spore. Do not expose
`soil:http`, SQLite, or Matrix tokens to the Spore.

## Why a native plugin, not a pure Wasm plugin

Loam supports two plugin forms:

1. a Wasm component exporting a capability WIT, and
2. a native sidecar plus a generated Wasm adapter that forwards over
   `loam:plugin/ipc`.

Drawbridge needs long-lived sockets (the Beeper appservice websocket), HTTP to
the MeshCore sidecar, SQLite with a process lock, and secret files. Those
belong in a native process. The right shape is **(2)**: a `drawbridge-host`
native sidecar that implements `ajmwagar:drawbridge`, plus the small Wasm
adapter Loam generates. No Loam rebuild is required; the plugin package is
signed and declares the WIT it provides.

## Split of responsibilities

| Native Drawbridge host | Soil Spore |
| --- | --- |
| Matrix appservice websocket/HTTP, `hs_token` checks | room naming and topics |
| MeshCore sidecar HTTP client | which conversation goes to which room |
| SQLite: cursor, bindings, links, claims | allowlists, owner checks, formatting |
| retries, dedupe, idempotent Matrix transaction IDs | ordering of a transaction's effects |
| secret files and credential handles | policy experiments without a Rust rebuild |
| appservice transaction acknowledgement | *when* a transaction is done |

The host must still enforce hard limits independently: only allowlisted
conversations, bounded text, no raw credentials, owner-only Matrix writes. A
Spore bug must not become a radio-control or token-exfiltration bug.

## Event model

A Spore is a command, so it runs a loop rather than exporting callbacks:

```text
next = none
loop:
  page = bridge.poll(next, 100)
  for event in page.events:
    match event:
      meshcore(message) => route to a room, bridge.deliver(...)
      matrix(transaction) => for each owner message: bridge.send(...)
                              bridge.commit(txn.id)   // only after effects
  next = page.next
```

The host owns the appservice transaction claim. It keeps the transaction
unacknowledged until `commit` returns; if the Spore fails, it calls `rollback`
or lets the homeserver retry. `claim(key)` gives the Spore the same exactly-once
semantics for individual network/Matrix event ids.

`deliver` takes a `transaction-id`, not a random one: a restart between the
Matrix send and the Spore's bookkeeping returns the same event id instead of a
duplicate. `send` returns the sidecar's request id, which is the stable key for
the MeshCore side.

## What ships in Drawbridge now

- `wit/drawbridge.wit` — the capability contract, including `ack` and
  `ensure-room`.
- `crates/drawbridge-host` — native sidecars with the Matrix appservice,
  MeshCore host, Rakazo host, SQLite, claims and idempotent delivery.
- `plugin/meshcore` — the Loam plugin package: a Wasm adapter exporting
  `ajmwagar:drawbridge` over `loam:plugin/ipc`, plus the native sidecar.
- `spores/meshcore-bridge` — the Soil policy loop. It polls, routes, delivers,
  sends owner messages and acks the network cursor.
- `meshcore-drawbridge` — the Rust fallback app, still authoritative for radio
  settings and channel/contact commands.

## Open questions

- **Blocking vs. event-driven:** `poll` returning an empty page is simple but
  burns a tick. A host-pushed `next` that blocks until an event or timeout is
  better for a daemon; Loam needs to support the blocking host call without
  stalling unrelated Spores.
- **Transaction size:** a Matrix transaction can carry many events; the host
  should page it or cap it so a Spore cannot exceed its fuel/response budget.
- **Backfill:** keep the cursor policy in the host. A Spore should not be able
  to force a full history replay by accident; `backfill` is an operator flag.
- **Multiple tenants:** the WIT currently assumes one bridge instance. A
  multi-tenant host would add a `tenant` field or one capability instance per
  tenant.
- **Encryption:** bridge rooms are unencrypted. If a Spore ever needs E2EE, the
  host must own the crypto and never hand keys to the guest.
