use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::healthcheck::HealthQuery;
use crate::jkbms::{IJkBmsConnection, JkBmsEvents};
use crate::mqtt::{IMqttConnection, MqttEvents};

use tokio::sync::mpsc;

use super::coordinator::Coordinator;

pub struct CoordinatorHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl CoordinatorHandle {
    pub fn new(
        mqtt: Box<dyn IMqttConnection>,
        jkbms: Box<dyn IJkBmsConnection>,
        jkbms_events_rx: mpsc::UnboundedReceiver<JkBmsEvents>,
        mqtt_events_rx: mpsc::UnboundedReceiver<MqttEvents>,
        health_query_rx: Option<mpsc::Receiver<HealthQuery>>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let coordinator = Coordinator::new(
            mqtt,
            jkbms,
            jkbms_events_rx,
            mqtt_events_rx,
            health_query_rx,
        );
        let task = tokio::spawn(coordinator.run(shutdown_rx));
        Self { shutdown_tx, task }
    }

    /// Signal coordinator shutdown and return the coordinator's `JoinHandle`.
    /// The coordinator's `run()` awaits its nested stop futures before returning.
    pub fn stop(self) -> JoinHandle<()> {
        let _ = self.shutdown_tx.send(true);
        drop(self.shutdown_tx);
        self.task
    }
}

/// Delegate `Future` polling to the inner coordinator task so the supervisor
/// can use `r = &mut coord_handle` in a `select!` to detect unexpected exits.
impl Future for CoordinatorHandle {
    type Output = Result<(), tokio::task::JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().task).poll(cx)
    }
}
