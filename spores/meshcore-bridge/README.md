# meshcore-bridge Spore

The Soil policy half of the MeshCore bridge. It polls the Drawbridge host,
routes inbound MeshCore conversations into Matrix rooms, sends owner messages
back to the radio, and acks the network cursor only after a page's effects are
durable.

```text
/meshcore-bridge --owner @you:server [--limit 100] [--interval-ms 2000] [--once]
```

The command is long-lived by default. Use `--once` in tests.

Owner commands:

```text
/mesh status
/mesh advert [flood|zerohop]
/mesh name <name>
/mesh txpower <0-30>
/mesh pathmode <0|1|2>
/mesh autoadd on|off
/mesh channels
/mesh contacts
/mesh channel create <name>
/mesh channel key <index>
/mesh channel share <index>
```
