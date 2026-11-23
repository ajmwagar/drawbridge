# Drawbridge

Reusable Rust primitives for small, self-hosted Matrix application-service
bridges.

Drawbridge is the part that stays the same when the network on the other side
changes: ghost namespacing, idempotent Matrix sends, registration bundles,
durable SQLite state, transaction acknowledgement, health checks and secret
loading. A bridge crate supplies the network adapter and the room policy; it
does not reimplement Matrix.

Licensed under [Apache-2.0](LICENSE). See [NOTICE](NOTICE) for extraction
provenance.

## Crates

| Crate | Purpose |
| --- | --- |
| [`drawbridge-core`](crates/drawbridge-core) | Stable IDs, secret loading, normalized inbound events, room specs. No Matrix or network types. |
| [`drawbridge-matrix`](crates/drawbridge-matrix) | Application-service client: registration bundle, ghost namespace checks, room creation/recovery, idempotent sends, media, transaction parsing. |
| [`drawbridge-store`](crates/drawbridge-store) | Single-writer SQLite with WAL, process lock, key check, XChaCha20-Poly1305 payload encryption, generic bindings/links/cursors/dedupe tables. |
| [`drawbridge-appservice`](crates/drawbridge-appservice) | Axum surface: both application-service transaction paths, constant-time `hs_token` check, body bounds, health/readiness/metrics. |

## Design rules

1. **The homeserver retries.** A transaction is acknowledged only after its
   effects are durable. Duplicate transactions, network messages and Matrix
   event IDs are claimed exactly once.
2. **Sends are idempotent.** Matrix transaction IDs are derived from the
   network message identity, so a crash between send and bookkeeping does not
   produce a duplicate after restart.
3. **Ghosts stay in the namespace.** Every generated user is checked against an
   exclusive registration namespace before it reaches the homeserver.
4. **Rooms are unencrypted unless a bridge says otherwise.** These are bridge
   rooms, not E2EE rooms; the operator can read them. The room creator is the
   ghost, the owner is invited, and encryption is denied in the power levels.
5. **Secrets come from files.** `*_FILE` with owner-only permissions is the
   supported deployment path.
6. **The database is single-writer.** A second process cannot open the same
   bridge database; the flock is deliberate.

## Minimal bridge shape

```rust,ignore
use drawbridge_appservice::{AppServiceState, TransactionHandler, router};
use drawbridge_matrix::{MatrixAppService, RegistrationBundle, parse_transaction};
use drawbridge_store::Store;

struct Bridge { /* matrix, network client, store */ }

impl TransactionHandler for Bridge {
    fn ingest(&self, txn: &str, payload: &serde_json::Value) -> anyhow::Result<()> {
        if !self.store.claim_transaction(txn)? {
            return Ok(()); // already acknowledged
        }
        for event in parse_transaction(payload)? {
            // filter, map room -> network conversation, send, record link
        }
        Ok(())
    }
}
```

The first consumer is
[`meshcore-mautrix`](https://github.com/ajmwagar/meshcore-mautrix), which bridges
a MeshCore companion radio (through the Radioman sidecar HTTP API) to Matrix.

## Soil / Loam

[`wit/drawbridge.wit`](wit/drawbridge.wit) is the draft host capability for
running bridge policy in a Soil Spore while the native Drawbridge host owns
Matrix, MeshCore HTTP, SQLite and credentials.
[`docs/soil-plugin.md`](docs/soil-plugin.md) explains the split, why this is a
native sidecar plugin rather than a pure Wasm plugin, and the open questions
(blocking `poll`, transaction paging, backfill, multi-tenancy).

## Build and verify

```sh
cargo fmt --all --check
cargo check --locked
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

## Status

`0.1.x` is an extraction, not a stable API. The crates are versioned together;
expect breaking changes until a second independent bridge uses them. Rakazo →
Beeper still carries its own copy of these modules and should migrate in a
separate, tested change.
