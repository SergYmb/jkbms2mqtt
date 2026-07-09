use std::time::Duration;

use tokio::time::timeout;

use super::codec;
use super::socket;
use super::status::HealthStatus;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(2);

pub struct HealthcheckClient {
    bms_name: String,
    socket_override: Option<String>,
}

impl HealthcheckClient {
    pub fn new(bms_name: &str, socket_override: Option<String>) -> Self {
        Self {
            bms_name: bms_name.to_string(),
            socket_override,
        }
    }

    pub async fn check(&self) -> anyhow::Result<()> {
        let mut stream = timeout(
            CONNECT_TIMEOUT,
            socket::connect_stream(&self.bms_name, self.socket_override.as_deref()),
        )
        .await??;
        let status = timeout(READ_TIMEOUT, codec::read_status(&mut stream)).await??;
        match status {
            HealthStatus::Healthy => Ok(()),
            HealthStatus::Unhealthy => Err(anyhow::anyhow!("BMS is unhealthy")),
        }
    }
}
