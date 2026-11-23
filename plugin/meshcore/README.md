# MeshCore Drawbridge host package

The native half of the MeshCore bridge. It owns:

- the Beeper appservice endpoint and transaction queue,
- the MeshCore sidecar HTTP client and durable cursor,
- SQLite bindings, claims, and idempotent delivery,
- secret files and owner auto-join.

`module.wasm` is the adapter in `adapter/`. It exports
`ajmwagar:drawbridge` and forwards each call over `loam:plugin/ipc` to
`bin/meshcore-host`. Build it with:

```bash
cargo build --manifest-path adapter/Cargo.toml --target wasm32-wasip2 --release
cp adapter/target/wasm32-wasip2/release/drawbridge_meshcore_adapter.wasm module.wasm
```

Build the sidecar from the workspace root:

```bash
cargo build -p drawbridge-host --bin meshcore-host --release
cp target/release/meshcore-host bin/meshcore-host
```

Run it with a config based on `crates/drawbridge-host/drawbridge-host.example.toml`.
The sidecar reads line-delimited JSON on stdin and serves the appservice on
`listen`.
