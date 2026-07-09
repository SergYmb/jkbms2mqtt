use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rumqttc::{ClientError, ConnectionError, Event, MqttOptions, QoS};
use tokio::sync::Notify;
use tokio::time::Instant;

use super::super::super::config::MqttConfig;
use super::super::super::mqttc_wrapper::{IMqttClient, IMqttClientFactory, IMqttEventLoop};

// ── PublishCall / SubscribeCall ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PublishCall {
    pub topic: String,
    pub qos: QoS,
    pub retain: bool,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SubscribeCall {
    pub topic: String,
    pub qos: QoS,
}

// ── EventLoopStep ─────────────────────────────────────────────────────────────

/// One step in a scripted `MqttEventLoopMock`.
pub enum EventLoopStep {
    /// Return this result from the next `poll()` call.
    Event(Result<Event, ConnectionError>),
    /// Sleep for `d` before processing the next step (allows paused-time tests
    /// to advance time between eventloop events, e.g. to make a session "stable").
    Delay(Duration),
}

// ── MqttClientMock ────────────────────────────────────────────────────────────

struct ClientInner {
    publishes: Vec<PublishCall>,
    subscribes: Vec<SubscribeCall>,
}

/// Mock MQTT client. Records all publish/subscribe calls.
/// On drop it notifies the paired `MqttEventLoopMock` so the eventloop
/// task can exit cleanly rather than hanging forever.
pub struct MqttClientMock {
    inner: Arc<Mutex<ClientInner>>,
    drop_notify: Arc<Notify>,
}

/// Test-side handle to a `MqttClientMock`. Kept by the test to inspect calls.
pub struct MqttClientMockHandle {
    inner: Arc<Mutex<ClientInner>>,
}

impl MqttClientMock {
    pub fn new(drop_notify: Arc<Notify>) -> (Self, MqttClientMockHandle) {
        let inner = Arc::new(Mutex::new(ClientInner {
            publishes: Vec::new(),
            subscribes: Vec::new(),
        }));
        let mock = MqttClientMock {
            inner: inner.clone(),
            drop_notify,
        };
        let handle = MqttClientMockHandle { inner };
        (mock, handle)
    }
}

impl Drop for MqttClientMock {
    fn drop(&mut self) {
        self.drop_notify.notify_one();
    }
}

#[async_trait]
impl IMqttClient for MqttClientMock {
    async fn publish(
        &self,
        topic: &str,
        qos: QoS,
        retain: bool,
        payload: Vec<u8>,
    ) -> Result<(), ClientError> {
        self.inner.lock().unwrap().publishes.push(PublishCall {
            topic: topic.to_string(),
            qos,
            retain,
            payload,
        });
        Ok(())
    }

    async fn subscribe(&self, topic: &str, qos: QoS) -> Result<(), ClientError> {
        self.inner.lock().unwrap().subscribes.push(SubscribeCall {
            topic: topic.to_string(),
            qos,
        });
        Ok(())
    }
}

impl MqttClientMockHandle {
    pub fn publishes(&self) -> Vec<PublishCall> {
        self.inner.lock().unwrap().publishes.clone()
    }

    pub fn subscribes(&self) -> Vec<SubscribeCall> {
        self.inner.lock().unwrap().subscribes.clone()
    }
}

// ── MqttEventLoopMock ─────────────────────────────────────────────────────────

/// Scripted eventloop mock. Delivers `EventLoopStep`s in order. When the
/// script is exhausted it parks on `drop_notify` — the manager fires this
/// when it drops the client on graceful shutdown.
pub struct MqttEventLoopMock {
    steps: VecDeque<EventLoopStep>,
    drop_notify: Arc<Notify>,
}

impl MqttEventLoopMock {
    pub fn new(steps: Vec<EventLoopStep>, drop_notify: Arc<Notify>) -> Self {
        MqttEventLoopMock {
            steps: steps.into(),
            drop_notify,
        }
    }
}

#[async_trait]
impl IMqttEventLoop for MqttEventLoopMock {
    async fn poll(&mut self) -> Result<Event, ConnectionError> {
        loop {
            match self.steps.pop_front() {
                Some(EventLoopStep::Event(result)) => return result,
                Some(EventLoopStep::Delay(d)) => tokio::time::sleep(d).await,
                None => {
                    self.drop_notify.notified().await;
                    return Err(ConnectionError::Io(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "mock client dropped",
                    )));
                }
            }
        }
    }
}

// ── MqttClientFactoryMock ─────────────────────────────────────────────────────

struct FactoryInner {
    sessions: VecDeque<(MqttClientMock, MqttEventLoopMock)>,
    session_timestamps: Vec<Instant>,
}

/// Scripted client factory. Each call to `new_client()` pops the next
/// prepared `(client, eventloop)` pair and records the call timestamp.
pub struct MqttClientFactoryMock {
    inner: Arc<Mutex<FactoryInner>>,
}

/// Test-side handle to inspect client creation timestamps (for backoff tests).
pub struct MqttClientFactoryMockHandle {
    inner: Arc<Mutex<FactoryInner>>,
}

impl MqttClientFactoryMock {
    pub fn new(
        sessions: Vec<(MqttClientMock, MqttEventLoopMock)>,
    ) -> (Self, MqttClientFactoryMockHandle) {
        let inner = Arc::new(Mutex::new(FactoryInner {
            sessions: sessions.into(),
            session_timestamps: Vec::new(),
        }));
        let factory = MqttClientFactoryMock {
            inner: inner.clone(),
        };
        let handle = MqttClientFactoryMockHandle { inner };
        (factory, handle)
    }
}

impl IMqttClientFactory for MqttClientFactoryMock {
    fn new_client(
        &self,
        _opts: MqttOptions,
        _channel_capacity: usize,
    ) -> (Box<dyn IMqttClient>, Box<dyn IMqttEventLoop>) {
        let mut g = self.inner.lock().unwrap();
        g.session_timestamps.push(Instant::now());
        let (client, eventloop) = g
            .sessions
            .pop_front()
            .expect("MqttClientFactoryMock: no more sessions scripted");
        (Box::new(client), Box::new(eventloop))
    }
}

impl MqttClientFactoryMockHandle {
    /// Timestamps of each `new_client()` call, in order.
    pub fn session_timestamps(&self) -> Vec<Instant> {
        self.inner.lock().unwrap().session_timestamps.clone()
    }
}

// ── Builder helpers ───────────────────────────────────────────────────────────

/// Build a matched `(client, eventloop, client_handle)` triple for one session.
pub fn make_session(
    steps: Vec<EventLoopStep>,
) -> (MqttClientMock, MqttEventLoopMock, MqttClientMockHandle) {
    let drop_notify = Arc::new(Notify::new());
    let (client, client_handle) = MqttClientMock::new(Arc::clone(&drop_notify));
    let eventloop = MqttEventLoopMock::new(steps, drop_notify);
    (client, eventloop, client_handle)
}

/// Construct a `MqttConfig` suitable for tests.
pub fn test_config() -> MqttConfig {
    MqttConfig {
        bms_name: "testbms".into(),
        host: "localhost".into(),
        port: 1883,
        user: None,
        pass: None,
        tls: false,
        client_id: None,
        discovery_prefix: "homeassistant".into(),
    }
}

// ── EventLoopStep constructors ────────────────────────────────────────────────

pub fn connack_step() -> EventLoopStep {
    use rumqttc::{ConnAck, ConnectReturnCode, Packet};
    EventLoopStep::Event(Ok(Event::Incoming(Packet::ConnAck(ConnAck::new(
        ConnectReturnCode::Success,
        false,
    )))))
}

pub fn disconnect_step() -> EventLoopStep {
    use rumqttc::Packet;
    EventLoopStep::Event(Ok(Event::Incoming(Packet::Disconnect)))
}

pub fn publish_step(topic: &str, payload: &[u8]) -> EventLoopStep {
    use rumqttc::{Packet, Publish};
    EventLoopStep::Event(Ok(Event::Incoming(Packet::Publish(Publish::new(
        topic,
        QoS::AtLeastOnce,
        payload.to_vec(),
    )))))
}

pub fn io_error_step() -> EventLoopStep {
    EventLoopStep::Event(Err(ConnectionError::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        "mock I/O error",
    ))))
}

pub fn delay_step(d: Duration) -> EventLoopStep {
    EventLoopStep::Delay(d)
}
