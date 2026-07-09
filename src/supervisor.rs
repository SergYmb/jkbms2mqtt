use std::time::Duration;

use tokio::sync::watch;

use crate::config::Config;
use crate::coordinator::CoordinatorHandle;
use crate::healthcheck::HealthcheckServer;
use crate::jkbms::JkBmsConnection;
use crate::mqtt::MqttConnection;

const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Run the service under a simple restart supervisor.
///
/// Watches for the coordinator task exiting/panicking and restarts it (with backoff on panic)
/// until the shutdown watch fires. The MQTT event-loop task is spawned inside
/// `MqttConnection::new` and does not need external supervision.
pub async fn run(config: Config, mut shutdown_rx: watch::Receiver<bool>) {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        if *shutdown_rx.borrow() {
            return;
        }

        let result = run_once(&config, &mut shutdown_rx).await;
        match result {
            RunResult::Shutdown => return,
            RunResult::Restart { because_panic } => {
                let msg = if because_panic {
                    "task panicked; restarting after backoff"
                } else {
                    "task exited unexpectedly; restarting after backoff"
                };
                tracing::warn!(delay_ms = backoff.as_millis(), "{msg}");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() { return; }
                    }
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

enum RunResult {
    Shutdown,
    Restart { because_panic: bool },
}

async fn run_once(config: &Config, shutdown_rx: &mut watch::Receiver<bool>) -> RunResult {
    let (mqtt_conn, mqtt_events_rx) = MqttConnection::new(config.mqtt.clone());
    let (jk_conn, jk_events_rx) = JkBmsConnection::new(config.jkbms.clone());

    let (hc_server, health_query_rx) = if config.healthcheck_server {
        let (srv, rx) =
            HealthcheckServer::new(&config.mqtt.bms_name, config.healthcheck_socket.as_deref());
        (Some(srv), Some(rx))
    } else {
        (None, None)
    };

    let mut coord_handle = CoordinatorHandle::new(
        Box::new(mqtt_conn),
        Box::new(jk_conn),
        jk_events_rx,
        mqtt_events_rx,
        health_query_rx,
    );

    let result = loop {
        tokio::select! {
            r = &mut coord_handle => {
                if *shutdown_rx.borrow() {
                    break RunResult::Shutdown;
                }
                break match r {
                    Ok(()) => RunResult::Restart { because_panic: false },
                    Err(e) if e.is_panic() => RunResult::Restart { because_panic: true },
                    Err(_) => RunResult::Shutdown,
                };
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break RunResult::Shutdown;
                }
            }
        }
    };
    let _ = coord_handle.stop().await;
    if let Some(hc) = hc_server {
        let _ = hc.stop().await;
    }
    result
}
