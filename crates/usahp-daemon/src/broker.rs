use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use usahp_core::{
    Action, HandshakeResponse, HandshakeStatus, Hello, Mapping, PROTOCOL_VERSION, ServerMessage,
    SessionRevoked, SwitchEvent, SwitchStateMachine,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalEvent {
    pub mapping_id: String,
    pub action: Action,
}

#[derive(Debug)]
pub enum BrokerCommand {
    Input(PhysicalEvent),
    Register {
        sender: mpsc::Sender<Arc<ServerMessage>>,
        reply: oneshot::Sender<u64>,
    },
    Unregister(u64),
    Handshake {
        client_id: u64,
        heartbeat_interval_ms: u32,
        missed_heartbeat_limit: u32,
    },
    Heartbeat {
        client_id: u64,
    },
}

struct Session {
    client_id: u64,
    heartbeat_interval: Duration,
    heartbeat_limit: u32,
    last_heartbeat: Instant,
}

pub fn spawn(mappings: Vec<Mapping>) -> mpsc::Sender<BrokerCommand> {
    let (sender, receiver) = mpsc::channel(1024);
    tokio::spawn(run(receiver, mappings));
    sender
}

async fn run(mut receiver: mpsc::Receiver<BrokerCommand>, mappings: Vec<Mapping>) {
    let mut state = SwitchStateMachine::new(&mappings);
    let started = Instant::now();
    let mut sequence = 0_u64;
    let mut next_client = 1_u64;
    let mut clients: HashMap<u64, mpsc::Sender<Arc<ServerMessage>>> = HashMap::new();
    let mut session: Option<Session> = None;
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            command = receiver.recv() => {
                let Some(command) = command else { break; };
                match command {
                    BrokerCommand::Register { sender, reply } => {
                        let client_id = next_client;
                        next_client += 1;
                        let hello = Arc::new(ServerMessage::Hello(Hello {
                            protocol_version: PROTOCOL_VERSION.into(),
                            switches: state.snapshots(),
                        }));
                        if sender.try_send(hello).is_ok() {
                            clients.insert(client_id, sender);
                            let _ = reply.send(client_id);
                        }
                    }
                    BrokerCommand::Unregister(client_id) => {
                        clients.remove(&client_id);
                        if session.as_ref().is_some_and(|s| s.client_id == client_id) {
                            session = None;
                            info!(client_id, "session client disconnected");
                        }
                    }
                    BrokerCommand::Input(event) => match state.apply(&event.mapping_id, event.action) {
                        Ok(Some(transition)) => {
                            sequence += 1;
                            let message = Arc::new(ServerMessage::SwitchEvent(SwitchEvent {
                                protocol_version: PROTOCOL_VERSION.into(),
                                sequence,
                                monotonic_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
                                switch_id: transition.switch_id,
                                action: transition.action,
                                confidence: match transition.action {
                                    Action::Pressed => 100.0,
                                    Action::Released => 0.0,
                                },
                            }));
                            if let Some(sess) = session.as_ref() {
                                // Exclusive: send only to the session client.
                                if let Some(sender) = clients.get(&sess.client_id) {
                                    if sender.try_send(message).is_err() {
                                        warn!(client_id = sess.client_id, "session queue full — dropping session");
                                        clients.remove(&sess.client_id);
                                        session = None;
                                    }
                                }
                            } else {
                                // No session: broadcast to all (backward compatible).
                                clients.retain(|cid, sender| match sender.try_send(message.clone()) {
                                    Ok(()) => true,
                                    Err(error) => {
                                        warn!(client_id = cid, %error, "disconnecting slow or closed client");
                                        false
                                    }
                                });
                            }
                        }
                        Ok(None) => {}
                        Err(error) => debug!(%error, "ignored invalid physical transition"),
                    },
                    BrokerCommand::Handshake { client_id, heartbeat_interval_ms, missed_heartbeat_limit } => {
                        // Revoke the previous session if one exists.
                        if let Some(old) = session.take() {
                            if let Some(sender) = clients.get(&old.client_id) {
                                let _ = sender.try_send(Arc::new(ServerMessage::SessionRevoked(
                                    SessionRevoked { reason: "SUPERSEDED".into() },
                                )));
                            }
                            info!(old_client = old.client_id, "session superseded");
                        }
                        session = Some(Session {
                            client_id,
                            heartbeat_interval: Duration::from_millis(heartbeat_interval_ms as u64),
                            heartbeat_limit: missed_heartbeat_limit,
                            last_heartbeat: Instant::now(),
                        });
                        if let Some(sender) = clients.get(&client_id) {
                            let _ = sender.try_send(Arc::new(ServerMessage::HandshakeResponse(
                                HandshakeResponse {
                                    status: HandshakeStatus::Accepted,
                                    session_id: Some(format!("sess_{client_id}")),
                                    heartbeat_interval_ms,
                                },
                            )));
                        }
                        info!(client_id, "handshake accepted");
                    }
                    BrokerCommand::Heartbeat { client_id } => {
                        if let Some(sess) = session.as_mut() {
                            if sess.client_id == client_id {
                                sess.last_heartbeat = Instant::now();
                            }
                        }
                    }
                }
            }
            _ = ticker.tick() => {
                // Check heartbeat timeout.
                if let Some(sess) = session.as_ref() {
                    let timeout = sess.heartbeat_interval * sess.heartbeat_limit;
                    if sess.last_heartbeat.elapsed() > timeout {
                        let cid = sess.client_id;
                        if let Some(sender) = clients.get(&cid) {
                            let _ = sender.try_send(Arc::new(ServerMessage::SessionRevoked(
                                SessionRevoked { reason: "HEARTBEAT_TIMEOUT".into() },
                            )));
                        }
                        session = None;
                        warn!(client_id = cid, "session revoked — heartbeat timeout");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::error::TryRecvError;
    use usahp_core::{InputKind, SwitchState};

    use super::*;

    fn mapping(id: &str, switch_id: &str) -> Mapping {
        Mapping {
            id: id.into(),
            switch_id: switch_id.into(),
            input: InputKind::Keyboard,
            code: "Space".into(),
            device: None,
        }
    }

    async fn register(
        broker: &mpsc::Sender<BrokerCommand>,
        capacity: usize,
    ) -> (u64, mpsc::Receiver<Arc<ServerMessage>>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let (reply, result) = oneshot::channel();
        broker
            .send(BrokerCommand::Register { sender, reply })
            .await
            .unwrap();
        (result.await.unwrap(), receiver)
    }

    #[tokio::test]
    async fn new_client_gets_current_state() {
        let broker = spawn(vec![mapping("a", "switch_1")]);
        broker
            .send(BrokerCommand::Input(PhysicalEvent {
                mapping_id: "a".into(),
                action: Action::Pressed,
            }))
            .await
            .unwrap();
        let (_, mut receiver) = register(&broker, 4).await;
        let message = receiver.recv().await.unwrap();
        let ServerMessage::Hello(hello) = &*message else {
            panic!("expected hello")
        };
        assert_eq!(hello.switches[0].state, SwitchState::Pressed);
    }

    #[tokio::test]
    async fn broadcasts_identical_ordered_events() {
        let broker = spawn(vec![mapping("a", "switch_1")]);
        let (_, mut first) = register(&broker, 4).await;
        let (_, mut second) = register(&broker, 4).await;
        first.recv().await.unwrap();
        second.recv().await.unwrap();

        for action in [Action::Pressed, Action::Released] {
            broker
                .send(BrokerCommand::Input(PhysicalEvent {
                    mapping_id: "a".into(),
                    action,
                }))
                .await
                .unwrap();
        }

        for expected in 1..=2 {
            for receiver in [&mut first, &mut second] {
                let ServerMessage::SwitchEvent(event) = &*receiver.recv().await.unwrap() else {
                    panic!("expected event")
                };
                assert_eq!(event.sequence, expected);
                // Binary switches emit 100.0 on press (seq 1) and 0.0 on release (seq 2).
                let expected_confidence: f32 = if expected == 1 { 100.0 } else { 0.0 };
                assert!(
                    (event.confidence - expected_confidence).abs() <= f32::EPSILON,
                    "confidence was {}",
                    event.confidence
                );
            }
        }
    }

    #[tokio::test]
    async fn disconnects_client_when_queue_is_full() {
        let broker = spawn(vec![mapping("a", "switch_1")]);
        let (_, mut receiver) = register(&broker, 1).await;
        // Leave hello queued so the first event overflows the client queue.
        broker
            .send(BrokerCommand::Input(PhysicalEvent {
                mapping_id: "a".into(),
                action: Action::Pressed,
            }))
            .await
            .unwrap();
        tokio::task::yield_now().await;
        assert!(receiver.try_recv().is_ok());
        tokio::task::yield_now().await;
        assert!(matches!(
            receiver.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn in_process_delivery_p95_is_below_twenty_milliseconds() {
        let broker = spawn(vec![mapping("a", "switch_1")]);
        let (_, mut receiver) = register(&broker, 8).await;
        receiver.recv().await.unwrap();
        let mut samples = Vec::new();

        for index in 0..100 {
            let action = if index % 2 == 0 {
                Action::Pressed
            } else {
                Action::Released
            };
            let started = std::time::Instant::now();
            broker
                .send(BrokerCommand::Input(PhysicalEvent {
                    mapping_id: "a".into(),
                    action,
                }))
                .await
                .unwrap();
            receiver.recv().await.unwrap();
            samples.push(started.elapsed());
        }

        samples.sort_unstable();
        let p95 = samples[94];
        assert!(p95 < std::time::Duration::from_millis(20), "p95={p95:?}");
    }
}
