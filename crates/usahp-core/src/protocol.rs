use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: &str = "0.1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SwitchState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwitchSnapshot {
    pub switch_id: String,
    pub state: SwitchState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    pub protocol_version: String,
    pub switches: Vec<SwitchSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwitchEvent {
    pub protocol_version: String,
    pub sequence: u64,
    pub monotonic_us: u64,
    pub switch_id: String,
    pub action: Action,
    /// Analog activation confidence in the range 0.0–100.0. Binary switches
    /// emit `100.0` on `Pressed` and `0.0` on `Released`; analog sources
    /// (BCI, facial-gesture, pressure) stream a raw probability instead.
    ///
    /// `serde(default)` keeps this field optional on the wire: clients built
    /// against this struct tolerate older servers that omit it (defaulting to
    /// `0.0`), and older clients simply ignore the extra field.
    #[serde(default)]
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello(Hello),
    SwitchEvent(SwitchEvent),
    HandshakeResponse(HandshakeResponse),
    SessionRevoked(SessionRevoked),
}

// ---- Session / heartbeat (Stage 1, backward-compatible layer on v0.1) ----

/// A handshake request from a client that wants active switch control.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handshake {
    pub protocol_version: String,
    pub app_id: String,
    pub requested_mode: String,
    pub heartbeat_interval_ms: u32,
    pub missed_heartbeat_limit: u32,
}

/// The daemon's response to a handshake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeResponse {
    #[serde(rename = "status")]
    pub status: HandshakeStatus,
    pub session_id: Option<String>,
    pub heartbeat_interval_ms: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HandshakeStatus {
    Accepted,
    Rejected,
}

/// Sent to a client when its session is revoked (heartbeat timeout or superseded).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRevoked {
    pub reason: String,
}

/// Messages a client sends TO the daemon (v0.1 is one-way; this is additive).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Handshake(Handshake),
    Heartbeat,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_serializes_to_versioned_tagged_json() {
        let message = ServerMessage::SwitchEvent(SwitchEvent {
            protocol_version: PROTOCOL_VERSION.into(),
            sequence: 42,
            monotonic_us: 1_843_201,
            switch_id: "switch_1".into(),
            action: Action::Pressed,
            confidence: 100.0,
        });
        let value = serde_json::to_value(message).unwrap();
        assert_eq!(value["type"], "switch_event");
        assert_eq!(value["protocol_version"], "0.1");
        assert_eq!(value["action"], "pressed");
        assert_eq!(value["confidence"], 100.0);
    }

    #[test]
    fn event_without_confidence_defaults_to_zero() {
        // An older server that omits `confidence` must still deserialize.
        let json = r#"{"type":"switch_event","protocol_version":"0.1","sequence":1,"monotonic_us":0,"switch_id":"switch_1","action":"released"}"#;
        let parsed: ServerMessage = serde_json::from_str(json).unwrap();
        let ServerMessage::SwitchEvent(event) = parsed else {
            panic!("expected switch_event");
        };
        assert_eq!(event.confidence, 0.0);
    }
}
