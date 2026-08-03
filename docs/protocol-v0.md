# USAHP Event Broker Protocol 0.1

This page is the canonical contract for USAHP protocol version `0.1`.

## Purpose

Protocol 0.1 lets local applications observe normalized switch edges from one daemon. It is a passive broadcast protocol at the client boundary: every connected client receives the same logical events, while hardware capture and suppression remain daemon responsibilities.

## Transport

- WebSocket over TCP bound to IPv4 loopback only.
- Default URL: `ws://127.0.0.1:7312`.
- Server-to-client text frames containing one complete UTF-8 JSON object.
- Clients do not send application messages in version 0.1.
- No authentication, encryption, remote binding, filtering, or replay.

## Hello

The first message sent after a successful WebSocket handshake is a complete snapshot of configured logical switches:

```json
{
  "type": "hello",
  "protocol_version": "0.1",
  "switches": [
    {"switch_id": "switch_1", "state": "released"}
  ]
}
```

Switches are ordered lexicographically by `switch_id`. A client connecting while a mapped input is held receives `"state": "pressed"`.

## Switch event

```json
{
  "type": "switch_event",
  "protocol_version": "0.1",
  "sequence": 42,
  "monotonic_us": 1843201,
  "switch_id": "switch_1",
  "action": "pressed",
  "confidence": 100.0
}
```

- `sequence` begins at 1 and increases globally for every emitted logical transition during one daemon process.
- `monotonic_us` is elapsed microseconds since daemon startup. It supports ordering and duration measurement but is not a wall-clock timestamp.
- `action` is exactly `pressed` or `released`.
- `confidence` is an analog activation score in the range `0.0`–`100.0`. For binary switches the daemon emits `100.0` on `pressed` and `0.0` on `released`. Analog sources (BCI, facial-gesture, pressure) stream a raw probability instead, so clients can apply their own activation thresholding. The field is optional on the wire (`serde(default)`): older servers omit it, and clients should treat a missing value as `0.0`.
- Duplicate physical edges are ignored and not assigned sequence numbers.
- With many-to-one mappings, only the first press and final release produce logical events.

> **Forward-compatible addition.** `confidence` was added to protocol 0.1 as an optional, additive field. It does not change the version tag and is safe to ignore. The broker in this release fills binary values only; streaming real analog confidence end-to-end (from capture through the state machine) is future work.

## Delivery and recovery

Events are delivered in sequence order to each connected client. Each client has a bounded queue. A client that fills its queue is disconnected so it cannot delay capture or other clients. Version 0.1 provides no replay; after reconnecting, the hello snapshot is the source of current state.

## Explicit exclusions

Version 0.1 does not include exclusive client ownership, foreground-app detection, OS accessibility handoff, debounce policy, hold events, per-client subscriptions, remote clients, or client-to-daemon control messages.

## Failsafes

Protocol 0.1 does **not** enforce the safety triggers defined in RFC §5: there is no escape-hatch detection (Trigger A, e.g. a sustained-hold revoke) and no software heartbeat (Trigger B, the client ack ping). A v0 daemon routes events passively and makes no claim of clinical lockout protection — a client must not assume it will be forcibly released from a stuck state by the broker. Failsafes belong to the future OS-handoff layer (RFC §4 `STATE_APP_CONTROL` and §5), not to this event-broker protocol.

