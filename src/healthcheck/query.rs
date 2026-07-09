use tokio::sync::oneshot;

use super::status::HealthStatus;

pub enum HealthQuery {
    Get(oneshot::Sender<HealthStatus>),
}
