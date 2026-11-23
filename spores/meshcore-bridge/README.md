# meshcore-bridge Spore

The Soil policy half of the MeshCore bridge. It polls the Drawbridge host,
routes inbound MeshCore conversations into Matrix rooms, sends owner messages
back to the radio, and acks the network cursor only after a page's effects are
durable.

```text
/meshcore-bridge --owner @you:server [--limit 100] [--interval-ms 2000] [--once]
```

The command is long-lived by default. Use `--once` in tests.

Only `/mesh status` and `/mesh help` are implemented here so far. The Rust
fallback still owns the radio settings and channel/contact commands until those
calls are added to `ajmwagar:drawbridge`.
