# USAHP in Plain English

A plain-language companion to the [USAHP specification](USAHP-RFC.md). This document explains the protocol without the formal RFC language. If you build software that uses switches, or if you help switch users, this document tells you what USAHP does and why it matters.

---

## The Problem

Switch users face a problem when they move between the operating system and an application.

For example, a user scans the macOS dock with Apple Switch Control. Then they open a scanning app. The app also wants the switch signal. But the operating system and the app both try to read the same switch at the same time. The result is confusion. Keys fire twice, or not at all.

This problem exists on every platform. Windows, macOS, Linux, iOS, and Android all have system-level switch scanning. AAC apps have their own scanning. There is no standard way to hand control from one to the other.

USAHP solves this problem.

---

## What USAHP Does

USAHP is a protocol that hands switch control between the operating system and an application.

When a switch-aware app comes to the foreground, the app tells the system that it wants the switches. The system stops its own scanning and sends the raw switch events to the app. When the app is done, or when the user switches away, the system takes control again.

The user never gets stuck. The protocol has built-in safety mechanisms. If the app freezes or crashes, the system takes back control immediately.

---

## The Four States

USAHP uses four states to manage who gets the switches.

1. **OS routing (default).** The operating system scans. The app gets no switch events. This is what Apple Switch Control and Windows Eye Control do today.

2. **Handshake.** A switch-aware app comes to the foreground. The app sends a request to the system. The system checks the request and accepts or rejects it.

3. **App control.** The system stops scanning. It sends the raw switch events to the app. The app does its own scanning. The system watches for safety triggers at the same time.

4. **Recovery.** If the app crashes or stops responding, the system takes back control instantly. The user is never trapped inside the app.

---

## Confidence Scores

Most switches are binary. The switch is pressed, or it is released. For these switches, the confidence score is always 100.0 (pressed) or 0.0 (released).

But modern switch inputs are not binary. A brain-computer interface sends a probability. A facial-gesture tracker sends a confidence level. A pressure sensor sends a continuous value.

USAHP carries this analog data in a `confidence` field. The field is a float from 0.0 to 100.0. Binary switches send 100.0 or 0.0. Analog sources send the raw probability.

The app decides what counts as a press. For example, the app can set a threshold at 85.0. Below 85.0, nothing happens. Above 85.0, the app treats it as a switch press. This lets apps do predictive scanning from noisy inputs.

---

## Safety

A switch user must never be trapped inside an application. USAHP enforces two safety mechanisms.

**Escape hatch (user-initiated).** The user can trigger a hardware pattern (for example, hold a switch for four seconds). The system intercepts this pattern before the app sees it. The system revokes app control and shows a system-level overlay. The user is back in OS scanning.

**Heartbeat (software-initiated).** While the app has control, it sends a regular ping to the system (every 500 ms by default). If the system misses three pings in a row, it assumes that the app is frozen. The system revokes control and returns to OS scanning.

---

## Who Gets the Switches

Multiple switch-aware apps can run at the same time. USAHP defines three tiers.

**Exclusive foreground.** The app gets switches only when it has window focus. If the user clicks away, the app loses the switches. The switches go back to the operating system. This is the default tier for most apps.

**Primary controller.** The app gets switches all the time, even when it is in the background. This tier is for "computer control" apps (for example, Grid 3 or VoiceGarden). Only one app can hold this tier at a time.

**Passive observer.** The app gets a read-only copy of switch events. It cannot consume or block them. Multiple apps can hold this tier at the same time.

When two apps ask for the same tier, the system shows a high-contrast arbitration overlay. The user uses standard OS scanning to choose which app gets control. If the user does not respond within a timeout (default 15 seconds), the system denies the new request and keeps the current app.

---

## How It Works on Each Platform

**macOS, Windows, Linux.** A background daemon captures switch hardware globally. It routes events to the foreground app over a local WebSocket. The daemon needs OS-level input permission (Accessibility on macOS, low-level hook approval on Windows, `input` group on Linux).

**iOS and iPadOS.** Apple does not allow third-party background daemons. A software-only daemon is not possible. The solution is a small hardware bridge (a Bluetooth dongle). The dongle acts as both a HID keyboard (for Apple Switch Control) and a custom GATT service (for the app). The dongle does the handoff in firmware. If the app crashes, the Bluetooth link drops. The dongle reverts to HID keyboard mode. Apple Switch Control takes over.

**Android.** Android's security model does not allow global input hooks from a standalone binary. The daemon runs as a registered AccessibilityService. Communication with the app uses Android's native AIDL bound services. When the app loses focus or crashes, Android severs the connection. The daemon detects this and returns to OS routing.

---

## What Exists Today (v0)

The v0 broker is built and working. It does the narrow event-broker layer:

- It captures switch keys (keyboard on all platforms, gamepad on Linux).
- It normalizes them to `pressed` and `released` edges.
- It broadcasts those edges to every connected local app over WebSocket (`ws://127.0.0.1:7312`).
- It supports a runtime capture flag. A host app can pause and resume capture (for example, when the window loses focus). This implements the `exclusive_foreground` tier.

The v0 broker does **not** do the full handoff state machine. There is no OS-level arbitration, no multi-app conflict resolution, no heartbeat, and no escape hatch. Those are future work.

A `confidence` field is on the wire (binary values only today). Streaming real analog confidence from capture through to the app is a follow-up.

---

## For Developers

If you build a switch-aware app:

1. Connect to `ws://127.0.0.1:7312`.
2. Read the `hello` frame to learn which switches are configured.
3. Listen for `switch_event` frames. Each frame has a `switch_id`, an `action` (`pressed` or `released`), and a `confidence` (0.0 to 100.0).
4. Map the switch events to your app's internal actions (for example, `switch_1` = select, `switch_2` = step).
5. You do not send anything back. The protocol is passive in v0. Your app receives events but never sends commands to the daemon.

If the daemon is not running, your app falls back to standard keyboard input. This is the expected behaviour. The daemon is an enhancement, not a requirement.
