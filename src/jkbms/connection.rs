use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::commands::{JkBmsEvents, RateLimitExceeded, WriteCommand};
use super::config::JkBmsConfig;
use super::connection_manager::{JkBmsCommands, JkBmsConnectionManager};
use super::protocol::{IJkBmsProtocol, JkBmsProtocol};
use super::transport::{IJkBmsTransportOpener, JkBmsTransportOpener};

const COMMAND_CHANNEL_CAPACITY: usize = 32;

pub trait IJkBmsConnection: Send + Sync {
    fn write(&self, command: WriteCommand, seq: u64) -> Result<(), RateLimitExceeded>;
    /// Drop the command sender (closing the channel) and return the manager's
    /// `JoinHandle`. Caller awaits the handle to confirm the manager has exited.
    fn stop(self: Box<Self>) -> JoinHandle<()>;
}

/// The public handle to the BMS connection manager. Constructing it spawns the
/// manager task and returns the receiver end of the `JkBmsEvents` channel, which
/// the sole consumer (the coordinator) drains. The handle is not `Clone` — only
/// one task is expected to issue writes.
pub struct JkBmsConnection {
    commands_sender: mpsc::Sender<JkBmsCommands>,
    manager_task: JoinHandle<()>,
}

impl JkBmsConnection {
    /// Spawn the connection manager with production opener and protocol built from `config`.
    pub fn new(config: JkBmsConfig) -> (Self, mpsc::UnboundedReceiver<JkBmsEvents>) {
        let opener: Box<dyn IJkBmsTransportOpener> = Box::new(JkBmsTransportOpener {
            device_path: config.bms_device,
        });
        let protocol: Box<dyn IJkBmsProtocol> = Box::new(JkBmsProtocol::new(config.slave_id));
        Self::new_with_deps(opener, protocol)
    }

    /// Spawn the connection manager with injected opener and protocol. Test seam only.
    fn new_with_deps(
        opener: Box<dyn IJkBmsTransportOpener>,
        protocol: Box<dyn IJkBmsProtocol>,
    ) -> (Self, mpsc::UnboundedReceiver<JkBmsEvents>) {
        let (commands_sender, commands_receiver) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let (events_sender, events_receiver) = mpsc::unbounded_channel();
        let manager =
            JkBmsConnectionManager::new(commands_receiver, events_sender, opener, protocol);
        let manager_task = tokio::spawn(manager.run());
        (
            JkBmsConnection {
                commands_sender,
                manager_task,
            },
            events_receiver,
        )
    }

    /// Request a control write. `seq` is echoed back in the resulting
    /// `JkBmsEvents::WriteConfirmation`/`WriteError`. Non-blocking: returns
    /// `RateLimitExceeded` if the bounded command channel is full.
    pub fn write(&self, command: WriteCommand, seq: u64) -> Result<(), RateLimitExceeded> {
        self.commands_sender
            .try_send(JkBmsCommands::Write { command, seq })
            .map_err(|_| RateLimitExceeded)
    }
}

impl IJkBmsConnection for JkBmsConnection {
    fn write(&self, command: WriteCommand, seq: u64) -> Result<(), RateLimitExceeded> {
        self.write(command, seq)
    }
    fn stop(self: Box<Self>) -> JoinHandle<()> {
        let Self {
            commands_sender,
            manager_task,
        } = *self;
        drop(commands_sender); // closes channel → manager's recv() returns None
        manager_task
    }
}

#[cfg(test)]
pub(super) mod internals {
    use tokio::sync::mpsc;

    use super::super::commands::JkBmsEvents;
    use super::super::protocol::IJkBmsProtocol;
    use super::super::transport::IJkBmsTransportOpener;
    use super::JkBmsConnection;

    pub fn new_with_deps(
        opener: Box<dyn IJkBmsTransportOpener>,
        protocol: Box<dyn IJkBmsProtocol>,
    ) -> (JkBmsConnection, mpsc::UnboundedReceiver<JkBmsEvents>) {
        JkBmsConnection::new_with_deps(opener, protocol)
    }
}
