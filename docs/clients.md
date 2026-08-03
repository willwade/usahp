# Client integration

Connect to `ws://127.0.0.1:7312` and treat the first `hello` as the complete current state. Protocol `0.2.0` supports passive listeners and one managed client.

## Passive listener

A passive listener sends nothing. It receives legacy broadcasts while no managed session is active. On any disconnect, discard assumptions about missed edges, reconnect, and replace local state from the next snapshot.

```js
const states = new Map()
const socket = new WebSocket('ws://127.0.0.1:7312')

socket.addEventListener('message', ({ data }) => {
  const message = JSON.parse(data)
  if (message.protocol_version !== '0.2.0') return socket.close(1002)
  if (message.type === 'hello') {
    states.clear()
    for (const item of message.switches) states.set(item.switch_id, item.state)
  } else if (message.type === 'switch_event') {
    states.set(message.switch_id, message.action)
  }
})
```

## Managed client

Send a handshake after `hello`. Do not choose a heartbeat interval: the accepted response reports the server-owned 500 ms interval and three-miss limit.

```js
socket.send(JSON.stringify({
  type: 'handshake',
  protocol_version: '0.2.0',
  app_id: 'org.example.switch-app',
  requested_mode: 'exclusive_foreground'
}))

// After ACCEPTED, repeat at the reported interval:
socket.send(JSON.stringify({ type: 'heartbeat', session_id }))
```

A competing handshake receives `SESSION_BUSY`. On `session_revoked`, stop acting on input and reconnect or request a new session. `CAPTURE_UNAVAILABLE` means the daemon could not reacquire an operating-system backend.

Run the supplied passive reference listener in readable or JSON mode:

```shell
cargo run -p usahp-listen -- ws://127.0.0.1:7312
cargo run -p usahp-listen -- --json ws://127.0.0.1:7312
```

Keep debounce, holds, confidence thresholds, and activation policy in the client. `monotonic_us` is process-relative, and a sequence gap requires reconnection because there is no replay.
