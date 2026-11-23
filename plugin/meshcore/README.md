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

## Run under Loam

Build the adapter and sidecar, then install the provider and the consumer into
one Loam root:

```sh
cargo build --manifest-path adapter/Cargo.toml --target wasm32-wasip2 --release
cp adapter/target/wasm32-wasip2/release/drawbridge_meshcore_adapter.wasm module.wasm
cargo build -p drawbridge-host --bin meshcore-host
cp ../../target/debug/meshcore-host bin/meshcore-host
loam plugin pack .
loam install . --root /tmp/loam-root --allow-unsigned-dev
loam install ../../spores/meshcore-bridge --root /tmp/loam-root   --provider-root /tmp/loam-root --allow-unsigned-dev
loam run --root /tmp/loam-root meshcore-bridge --owner @you:server --once
```

The sidecar runs with the plugin package as its working directory, so its
`meshcore-host.toml` sits beside `spore.toml`. Logs go to stderr; stdout is the
line-delimited IPC channel.
