use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::status::HealthStatus;

const WIRE_HEALTHY: u8 = 0x01;
const WIRE_UNHEALTHY: u8 = 0x00;

pub(super) async fn read_status<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<HealthStatus> {
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf).await?;
    Ok(if buf[0] == WIRE_HEALTHY {
        HealthStatus::Healthy
    } else {
        HealthStatus::Unhealthy
    })
}

pub(super) async fn write_status<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    status: HealthStatus,
) -> std::io::Result<()> {
    let byte = match status {
        HealthStatus::Healthy => WIRE_HEALTHY,
        HealthStatus::Unhealthy => WIRE_UNHEALTHY,
    };
    writer.write_all(&[byte]).await
}
