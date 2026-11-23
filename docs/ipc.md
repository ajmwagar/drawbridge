# Drawbridge host IPC

The native sidecar speaks line-delimited JSON. One request per line, one
response per line, both UTF-8. This is the wire behind Loam's
`loam:plugin/ipc`; the Wasm adapter translates WIT calls into these methods.

```json
{"id":1,"method":"status","params":{}}
{"id":1,"ok":true,"result":{"sidecar-connected":true,"radio-name":"Radioman-Pi","contacts":350}}
```

`id` is optional and echoed back. A malformed line gets `{"ok":false,...}` and
does not stop the session.

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `status` | `{}` | health object |
| `poll` | `{"cursor": string?, "limit": u32}` | `{"events": [...], "next": string?}` |
| `send` | `{"conversation": {"kind":"channel|contact","id":string}, "text":string}` | `{"request-id":string,"state":string}` |
| `deliver` | `{"room":string,"transaction":string,"text":string}` | Matrix event id |
| `bind` | `{"conversation":..., "room":string, "ghost":string, "display-name":string}` | `{}` |
| `bindings` | `{}` | list of bindings |
| `claim` | `{"key":string}` | `true` if this caller owns it |
| `commit` | `{"transaction":string}` | `{}` |
| `rollback` | `{"transaction":string}` | `{}` |

`poll` events use the WIT variant shape:

```json
{"meshcore": {"id":"mc:...","conversation":{"kind":"channel","id":"0"},"sender":"...","text":"..."}}
{"matrix": {"id":"txn","events":[{"event-id":"$...","room":"!...","sender":"@...","kind":"m.room.message","body":"..."}]}}
```

## Invariants

- The host owns the Matrix appservice transaction claim; `commit` is the
  acknowledgement. A failed Spore calls `rollback` so the homeserver retries.
- `deliver` is idempotent on `transaction`; a retry returns the same event id.
- `claim` is the exactly-once gate for network message ids and Matrix event ids.
- `poll` never returns raw credentials, media bytes, or SQLite rows.
- The Spore is never given the sidecar token or the owner token.

## Adapter

The adapter is a small Wasm component that exports `ajmwagar:drawbridge` and
imports `loam:plugin/ipc`. It reads the typed WIT arguments, serializes them
into the JSON above, calls `request(method, payload)`, and deserializes the
response. It has no network, filesystem, or clock access.
