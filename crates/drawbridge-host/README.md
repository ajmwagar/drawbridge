# drawbridge-host

The native side of the Drawbridge Loam plugin.

```sh
cargo run -p drawbridge-host --bin meshcore-host -- --config drawbridge-host.example.toml
```

It reads one JSON request per line on stdin and writes one JSON response per
line on stdout. See [`docs/ipc.md`](../../docs/ipc.md) for the method contract.

Current MeshCore methods:

```text
status
poll
send
advert
settings
channel.create
channel.key
claim
bind
bindings
deliver   (Matrix delivery is the next slice)
commit
rollback
```

The host owns the sidecar token, the SQLite database key and the claim/cursor
store. The Spore sees neither credential.

## Example

```sh
printf '%s\n' '{"id":1,"method":"status","params":{}}' \
  '{"id":2,"method":"poll","params":{"limit":5}}' \
  | MESHCORE_TOKEN_FILE=... DRAWBRIDGE_HOST_KEY_FILE=... \
    cargo run -q -p drawbridge-host --bin meshcore-host -- --config drawbridge-host.example.toml
```
