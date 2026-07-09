#[derive(Debug, Clone)]
pub struct MqttConfig {
    /// Logical name of this BMS — used as MQTT topic prefix and HA entity ID prefix.
    pub bms_name: String,

    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub tls: bool,
    /// When None, `MqttConnection::new` derives `jkbms2mqtt-<bms_name>`.
    pub client_id: Option<String>,

    /// HA MQTT discovery prefix (usually "homeassistant").
    pub discovery_prefix: String,
}
