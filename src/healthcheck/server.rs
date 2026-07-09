use std::time::Duration;

use interprocess::local_socket::{tokio::Stream, traits::tokio::Listener as _};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

use super::codec;
use super::query::HealthQuery;
use super::socket;
use super::status::HealthStatus;

const CHANNEL_CAPACITY: usize = 4;
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

pub struct HealthcheckServer {
    task: JoinHandle<()>,
}

impl HealthcheckServer {
    pub fn new(
        bms_name: &str,
        socket_override: Option<&str>,
    ) -> (Self, mpsc::Receiver<HealthQuery>) {
        let (query_tx, query_rx) = mpsc::channel::<HealthQuery>(CHANNEL_CAPACITY);
        let bms_name = bms_name.to_string();
        let socket_override = socket_override.map(str::to_owned);
        let task = tokio::spawn(async move {
            if let Err(e) = accept_loop(bms_name, socket_override, query_tx).await {
                tracing::warn!(error = %e, "healthcheck server exited");
            }
        });
        (Self { task }, query_rx)
    }

    pub fn stop(self) -> JoinHandle<()> {
        self.task.abort(); // no graceful path for the accept loop
        self.task // caller awaits to know the listener has been released
    }
}

async fn accept_loop(
    bms_name: String,
    socket_override: Option<String>,
    query_tx: mpsc::Sender<HealthQuery>,
) -> anyhow::Result<()> {
    let listener = socket::create_listener(&bms_name, socket_override.as_deref())?;
    let mut conn_tasks: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            result = listener.accept() => {
                let conn = result?;
                let tx = query_tx.clone();
                conn_tasks.spawn(handle_connection(conn, tx));
            }
            Some(_) = conn_tasks.join_next() => {}
        }
    }
}

async fn handle_connection(mut conn: Stream, tx: mpsc::Sender<HealthQuery>) {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if tx.send(HealthQuery::Get(reply_tx)).await.is_err() {
        return;
    }
    let status = match timeout(QUERY_TIMEOUT, reply_rx).await {
        Ok(Ok(s)) => s,
        _ => HealthStatus::Unhealthy,
    };
    let _ = codec::write_status(&mut conn, status).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stopping a `HealthcheckServer` must release the UDS listener so the same
    /// `bms_name` can be reused immediately.
    #[tokio::test]
    async fn stopping_server_releases_listener() {
        let bms_name = "jkbms2mqtt-stop-test";

        let (server1, _rx1) = HealthcheckServer::new(bms_name, None);
        // Yield so the accept loop actually reaches `listener.accept().await`
        // and takes ownership of the listener before we abort.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        let _ = server1.stop().await;

        // Rebind directly — this is what accept_loop does inside the task, and
        // is where a leaked listener would surface as `Address in use`.
        let _listener = socket::create_listener(bms_name, None).expect("rebind after stop");
    }
}
