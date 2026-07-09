use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::domain::Snapshot;
use crate::jkbms::JkBmsDeviceInfo;

use super::config::MqttConfig;
use super::connection_manager::{MqttCommand, MqttConnectionManager};
use super::events::MqttEvents;
use super::mqttc_wrapper::{IMqttClientFactory, MqttClientFactory};

const COMMAND_CHANNEL_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitExceeded;

/// The public handle to the MQTT connection manager. Constructing it spawns the
/// manager task and returns the `MqttEvents` receiver, which the sole consumer
/// (the coordinator) drains. The trait is non-async: methods `try_send` onto a
/// bounded command channel and return `RateLimitExceeded` on backpressure —
/// mirrors `IJkBmsConnection`.
pub trait IMqttConnection: Send + Sync {
    fn publish_snapshot(&self, snapshot: &Snapshot) -> Result<(), RateLimitExceeded>;
    fn publish_discovery(
        &self,
        device_info: &JkBmsDeviceInfo,
        cell_count: u32,
    ) -> Result<(), RateLimitExceeded>;
    fn publish_availability(&self, online: bool) -> Result<(), RateLimitExceeded>;
    fn subscribe_to_commands(&self) -> Result<(), RateLimitExceeded>;
    /// Drop the command sender (closing the channel) and return the manager's
    /// `JoinHandle`. Caller awaits the handle to confirm the manager has exited.
    fn stop(self: Box<Self>) -> JoinHandle<()>;
}

pub struct MqttConnection {
    commands_tx: mpsc::Sender<MqttCommand>,
    manager_task: JoinHandle<()>,
}

impl MqttConnection {
    /// Build channels, spawn the manager task, return the handle and the events
    /// receiver. The manager owns the `(AsyncClient, EventLoop)` pair and drives
    /// its own reconnect loop with backoff.
    pub fn new(config: MqttConfig) -> (Self, mpsc::UnboundedReceiver<MqttEvents>) {
        Self::new_with_factory(config, Box::new(MqttClientFactory))
    }

    fn new_with_factory(
        config: MqttConfig,
        client_factory: Box<dyn IMqttClientFactory>,
    ) -> (Self, mpsc::UnboundedReceiver<MqttEvents>) {
        let (commands_tx, commands_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let manager = MqttConnectionManager::new(config, commands_rx, events_tx, client_factory);
        let manager_task = tokio::spawn(manager.run());
        (
            MqttConnection {
                commands_tx,
                manager_task,
            },
            events_rx,
        )
    }
}

impl IMqttConnection for MqttConnection {
    fn publish_snapshot(&self, snapshot: &Snapshot) -> Result<(), RateLimitExceeded> {
        self.commands_tx
            .try_send(MqttCommand::PublishSnapshot(Box::new(snapshot.clone())))
            .map_err(|_| RateLimitExceeded)
    }

    fn publish_discovery(
        &self,
        device_info: &JkBmsDeviceInfo,
        cell_count: u32,
    ) -> Result<(), RateLimitExceeded> {
        self.commands_tx
            .try_send(MqttCommand::PublishDiscovery {
                device_info: Box::new(device_info.clone()),
                cell_count,
            })
            .map_err(|_| RateLimitExceeded)
    }

    fn publish_availability(&self, online: bool) -> Result<(), RateLimitExceeded> {
        self.commands_tx
            .try_send(MqttCommand::PublishAvailability(online))
            .map_err(|_| RateLimitExceeded)
    }

    fn subscribe_to_commands(&self) -> Result<(), RateLimitExceeded> {
        self.commands_tx
            .try_send(MqttCommand::SubscribeToCommands)
            .map_err(|_| RateLimitExceeded)
    }

    fn stop(self: Box<Self>) -> JoinHandle<()> {
        let Self {
            commands_tx,
            manager_task,
        } = *self;
        drop(commands_tx); // closes channel → manager's recv() returns None
        manager_task
    }
}

#[cfg(test)]
pub(super) mod internals {
    pub fn new_with_factory(
        config: super::MqttConfig,
        client_factory: Box<dyn super::IMqttClientFactory>,
    ) -> (
        super::MqttConnection,
        tokio::sync::mpsc::UnboundedReceiver<super::MqttEvents>,
    ) {
        super::MqttConnection::new_with_factory(config, client_factory)
    }
}
