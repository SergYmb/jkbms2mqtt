use super::inbound::IncomingRequest;

#[derive(Debug)]
pub enum MqttEvents {
    BrokerConnected,
    BrokerDisconnected,
    Incoming(IncomingRequest),
}
