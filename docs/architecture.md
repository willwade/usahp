# Architecture

USAHP keeps hardware handling separate from application behaviour. The daemon is the single input owner. Passive clients can observe legacy broadcasts, or one managed client can receive exclusive routing.

```text
keyboard / Linux evdev
          │
          ▼
 capture + suppression
          │ physical edges
          ▼
 logical state machine ◄── stdin simulator
          │ logical transitions
          ▼
 broker + global sequence
          │
          ├── bounded client queue ── WebSocket client A
          └── bounded client queue ── WebSocket client B
```

## Capture and suppression

Every configured mapping must use a backend that can guarantee suppression. Keyboard mappings use a global grab on all supported platforms. Linux gamepad and dedicated switch inputs use an exclusive evdev device grab. A gamepad mapping on Windows or macOS is rejected at startup because v0 cannot guarantee suppression there.

## Logical aggregation

Each physical mapping has a unique `id` and points to a logical `switch_id`. Several mappings may point to the same logical switch:

- the first physical press emits one logical `pressed` event;
- further presses for that switch change no logical state;
- releases emit nothing while another mapped input remains held;
- the final physical release emits one logical `released` event.

Duplicate physical transitions are invalid and are ignored by the broker.

## Delivery

The broker assigns one global sequence number and a monotonic timestamp to every logical transition. Each client gets its own bounded queue. When a client cannot keep up and fills that queue, USAHP disconnects it rather than delaying capture or other listeners.

New and reconnected clients first receive a [`hello` snapshot](/protocol-v0#transport-and-legacy-listeners) containing every configured logical switch. Protocol 0.2 does not replay missed events.

## Session and capture lifecycle

The daemon starts with capture enabled and legacy broadcast routing. The first valid handshake clears any held legacy state before establishing an exclusive session. Heartbeat timeout, disconnect, queue overflow, or explicit library revocation disables capture, emits ordered releases, clears physical state, and pauses routing. A later handshake succeeds only after every backend is reacquired.

## Responsibility boundary

| USAHP daemon | Client application |
| --- | --- |
| Capture configured inputs | Interpret an interaction |
| Guarantee suppression | Debounce if required |
| Aggregate physical state | Detect holds and repeats |
| Order logical transitions | Apply confidence or policy |
| Route to passive or managed clients | Trigger an application action |

This boundary lets several applications observe a common, low-level event stream without embedding hardware-specific code in each app.
