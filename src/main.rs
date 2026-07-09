use anyhow::Context;

use jkbms2mqtt::config::Config;
use jkbms2mqtt::healthcheck::HealthcheckClient;
use jkbms2mqtt::supervisor;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    eprintln!("jkbms2mqtt v{}", env!("CARGO_PKG_VERSION"));

    let config = match Config::parse().context("failed to load configuration") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e:#}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if config.healthcheck {
        let client =
            HealthcheckClient::new(&config.mqtt.bms_name, config.healthcheck_socket.clone());
        return match client.check().await {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => std::process::ExitCode::FAILURE,
        };
    }

    let filter = tracing_subscriber::EnvFilter::try_new(&config.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    tracing::info!(
        version    = env!("CARGO_PKG_VERSION"),
        log_level  = %config.log_level,
        bms_device = %config.jkbms.bms_device,
        slave_id   = config.jkbms.slave_id,
        bms_name   = %config.mqtt.bms_name,
        mqtt_host  = %config.mqtt.host,
        mqtt_port  = config.mqtt.port,
        mqtt_tls   = config.mqtt.tls,
        "jkbms2mqtt starting"
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        await_shutdown_signal().await;
        tracing::info!("shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    supervisor::run(config, shutdown_rx).await;
    tracing::info!("jkbms2mqtt stopped");
    std::process::ExitCode::SUCCESS
}

async fn await_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
