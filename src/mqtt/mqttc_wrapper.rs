use async_trait::async_trait;
use rumqttc::{AsyncClient, ClientError, EventLoop, MqttOptions, QoS};

// ── Thin async traits (testability seams) ─────────────────────────────────────

#[async_trait]
pub(super) trait IMqttEventLoop: Send {
    async fn poll(&mut self) -> Result<rumqttc::Event, rumqttc::ConnectionError>;
}

#[async_trait]
pub(super) trait IMqttClient: Send + Sync {
    async fn publish(
        &self,
        topic: &str,
        qos: QoS,
        retain: bool,
        payload: Vec<u8>,
    ) -> Result<(), ClientError>;
    async fn subscribe(&self, topic: &str, qos: QoS) -> Result<(), ClientError>;
}

pub(super) trait IMqttClientFactory: Send + Sync {
    fn new_client(
        &self,
        opts: MqttOptions,
        channel_capacity: usize,
    ) -> (Box<dyn IMqttClient>, Box<dyn IMqttEventLoop>);
}

// ── Production implementations ────────────────────────────────────────────────

#[async_trait]
impl IMqttEventLoop for EventLoop {
    async fn poll(&mut self) -> Result<rumqttc::Event, rumqttc::ConnectionError> {
        EventLoop::poll(self).await
    }
}

#[async_trait]
impl IMqttClient for AsyncClient {
    async fn publish(
        &self,
        topic: &str,
        qos: QoS,
        retain: bool,
        payload: Vec<u8>,
    ) -> Result<(), ClientError> {
        AsyncClient::publish(self, topic, qos, retain, payload).await
    }

    async fn subscribe(&self, topic: &str, qos: QoS) -> Result<(), ClientError> {
        AsyncClient::subscribe(self, topic, qos).await
    }
}

pub(super) struct MqttClientFactory;

impl IMqttClientFactory for MqttClientFactory {
    fn new_client(
        &self,
        opts: MqttOptions,
        channel_capacity: usize,
    ) -> (Box<dyn IMqttClient>, Box<dyn IMqttEventLoop>) {
        let (client, eventloop) = AsyncClient::new(opts, channel_capacity);
        (Box::new(client), Box::new(eventloop))
    }
}
