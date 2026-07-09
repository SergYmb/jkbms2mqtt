//! The idiomatic Tokio "actor" pattern (Alice Ryhl, "Actors with Tokio").
//!
//! `JkBmsConnectionManager` owns the serial link and is the sole mutator of all
//! connection state — no locks. It is reached only through `JkBmsConnection` (public
//! commands, bounded channel) and drives its own lifecycle through
//! `JkBmsInternalCommands` (self-posted, unbounded — the actor is the only drainer,
//! so a bounded self-send could deadlock). Timed work goes through a `DelayQueue`.
//! Everything it produces leaves on the standalone `JkBmsEvents` channel.

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant; // virtual clock under tokio::time::pause()/advance() (testability, ARCHITECTURE.md §8)
use tokio_stream::StreamExt;
use tokio_util::time::DelayQueue;

use super::commands::{ConnectionState, JkBmsError, JkBmsEvents, WriteCommand};
use super::protocol::IJkBmsProtocol;
use super::transport::{IJkBmsTransport, IJkBmsTransportOpener};
use super::types::{JkBmsData, JkBmsDataType};

const PENDING_WRITES_CAP: usize = 5;
/// How long a queued write may wait for the link before it is evicted with `WriteError`.
const WRITE_TTL: Duration = Duration::from_secs(10);
/// Settle time between a control-register write ACK and the follow-up Frame 0x01
/// readback. Empirically the BMS ACKs a switch write in ~10–16 ms but is still
/// busy internally afterwards — a config trigger sent 5 ms after the ACK gets no
/// reply. 300 ms is safely past the observed busy window and imperceptible on the
/// HA switch UX.
const POST_WRITE_SETTLE: Duration = Duration::from_millis(300);
/// Delay before RetryWrites after a soft write failure — gives the wire (and the
/// BMS) time to settle before the retry TX, and prevents a tight loop when the
/// stuck-state counter is climbing towards `RECONNECT_THRESHOLD`.
const RETRY_WRITE_SOFT_DELAY: Duration = Duration::from_millis(250);

// ── Polling / timing tunables ─────────────────────────────────────────────────
const OPERATIONAL_POLL_INTERVAL: Duration = Duration::from_secs(5);
const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(20);
const DEVICE_INFO_POLL_INTERVAL: Duration = Duration::from_secs(30);
const ALARM_POLL_INTERVAL: Duration = Duration::from_secs(5);
const MIN_POLL_INTERVAL: Duration = OPERATIONAL_POLL_INTERVAL;
/// Cap on the exponential reconnect backoff (the `Capped` stage delay).
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_millis(5000);
/// Consecutive soft poll failures (parse error or timeout, any frame type) that force a synthetic disconnect.
const RECONNECT_THRESHOLD: u32 = 5;

/// Deterministic backoff schedule for serial reopen attempts (ARCHITECTURE.md §10).
/// Indexed by reconnect-attempt count; attempts past the end use `RECONNECT_BACKOFF_MAX`.
/// The listed delays sum to exactly 10s, so once they are exhausted (the "capped"
/// regime) writes start being rejected — 10s after the first failed reconnect.
const RECONNECT_BACKOFF: [Duration; 5] = [
    Duration::from_millis(250),
    Duration::from_millis(1000),
    Duration::from_millis(1750),
    Duration::from_millis(2500),
    Duration::from_millis(4500),
];

// ── Handle → actor protocol (visible to `client.rs`, not outside the module) ──

pub(super) enum JkBmsCommands {
    Write { command: WriteCommand, seq: u64 },
}

// ── Self-posted lifecycle commands (fully private to the actor) ───────────────

enum JkBmsInternalCommands {
    Connect,
    RunDataPolling,
    RetryWrites,
}

pub(super) struct JkBmsConnectionManager {
    opener: Box<dyn IJkBmsTransportOpener>,
    protocol: Box<dyn IJkBmsProtocol>,

    // Channels
    commands_receiver: mpsc::Receiver<JkBmsCommands>,
    events_sender: mpsc::UnboundedSender<JkBmsEvents>,
    internal_commands_sender: mpsc::UnboundedSender<JkBmsInternalCommands>,
    internal_commands_receiver: mpsc::UnboundedReceiver<JkBmsInternalCommands>,
    delayed_commands: DelayQueue<JkBmsInternalCommands>,

    // State, synchronized by this task alone
    transport: Option<Box<dyn IJkBmsTransport>>,
    reconnect_attempt: usize,
    data_polling_scheduled: bool,
    /// True when a `RetryWrites` command is queued (immediate or delayed).
    /// Guards against duplicate scheduling — cleared at the top of
    /// `retry_writes` so the handler can re-arm if more work remains.
    retry_writes_scheduled: bool,
    consecutive_failures: u32,
    pending_writes: VecDeque<(WriteCommand, u64, Instant)>,
    last_device_info: Option<Instant>,
    last_config: Option<Instant>,
    last_operational: Option<Instant>,
    last_alarms: Option<Instant>,
}

impl JkBmsConnectionManager {
    pub(super) fn new(
        commands_receiver: mpsc::Receiver<JkBmsCommands>,
        events_sender: mpsc::UnboundedSender<JkBmsEvents>,
        opener: Box<dyn IJkBmsTransportOpener>,
        protocol: Box<dyn IJkBmsProtocol>,
    ) -> Self {
        let (internal_commands_sender, internal_commands_receiver) = mpsc::unbounded_channel();
        JkBmsConnectionManager {
            opener,
            protocol,
            commands_receiver,
            events_sender,
            internal_commands_sender,
            internal_commands_receiver,
            delayed_commands: DelayQueue::new(),
            transport: None,
            reconnect_attempt: 0,
            data_polling_scheduled: false,
            retry_writes_scheduled: false,
            consecutive_failures: 0,
            pending_writes: VecDeque::new(),
            last_device_info: None,
            last_config: None,
            last_operational: None,
            last_alarms: None,
        }
    }

    /// Runs until `JkBmsConnection` is dropped and the command channel closes. Each message
    /// is handled to completion before the next is read, so mutations are serialized.
    pub(super) async fn run(mut self) {
        self.connect().await;

        loop {
            tokio::select! {
                maybe = self.commands_receiver.recv() => match maybe {
                    Some(cmd) => self.handle_command(cmd).await,
                    None => break, // handle dropped
                },
                Some(internal) = self.internal_commands_receiver.recv() => {
                    self.handle_internal(internal).await;
                }
                Some(expired) = self.delayed_commands.next() => {
                    self.handle_internal(expired.into_inner()).await;
                }
            }
        }

        tracing::info!("JkBmsConnectionManager run loop finished");
    }

    async fn handle_command(&mut self, cmd: JkBmsCommands) {
        match cmd {
            JkBmsCommands::Write { command, seq } => self.write_command(command, seq).await,
        }
    }

    async fn handle_internal(&mut self, cmd: JkBmsInternalCommands) {
        match cmd {
            JkBmsInternalCommands::Connect => self.connect().await,
            JkBmsInternalCommands::RunDataPolling => self.run_data_polling().await,
            JkBmsInternalCommands::RetryWrites => self.retry_writes().await,
        }
    }

    // ── Connection lifecycle ──────────────────────────────────────────────────

    async fn connect(&mut self) {
        if self.transport.is_some() {
            return;
        }
        match self.opener.open().await {
            Ok(transport) => {
                self.transport = Some(transport);
                self.consecutive_failures = 0;
                tracing::info!("serial connected");

                if let Err(e) = self.resync().await {
                    tracing::warn!("resync failed after connect: {e}");
                    self.handle_disconnect(e);
                    return;
                }

                // Only a full connect+resync clears the reconnect backoff schedule.
                self.reconnect_attempt = 0;
                self.emit(JkBmsEvents::Connection(ConnectionState::Connected));

                self.schedule_data_polling_now();
                if !self.pending_writes.is_empty() {
                    self.schedule_retry_writes_now();
                }
            }
            Err(e) => {
                tracing::warn!(errno = e.raw_os_error(), "serial open failed: {e}");
                self.schedule_reconnect();
            }
        }
    }

    /// Post-reopen / startup resync: device info, then config (sets cell_count),
    /// then operational. Errors propagate to the caller, which triggers reconnect.
    async fn resync(&mut self) -> Result<(), JkBmsError> {
        tracing::debug!("resync: polling device info");
        self.poll_data(JkBmsDataType::DeviceInfo).await?;
        tracing::debug!("resync: polling config options");
        self.poll_data(JkBmsDataType::ConfigOptions).await?;
        tracing::debug!("resync: polling operational data");
        self.poll_data(JkBmsDataType::OperationalData).await?;
        tracing::debug!("resync complete");
        Ok(())
    }

    fn handle_disconnect(&mut self, err: JkBmsError) {
        if self.transport.is_none() {
            return;
        }
        if let JkBmsError::Io(e) = &err {
            tracing::warn!(errno = e.raw_os_error(), "serial disconnect: {e}");
        } else {
            tracing::warn!("serial disconnect: {err}");
        }
        self.transport = None;
        self.consecutive_failures = 0;
        // `reconnect_attempt` is deliberately not reset here.
        self.schedule_reconnect();
    }

    /// True once the backoff schedule is exhausted (the capped regime).
    fn reconnect_capped(&self) -> bool {
        self.reconnect_attempt > RECONNECT_BACKOFF.len()
    }

    fn schedule_reconnect(&mut self) {
        self.reconnect_attempt += 1;
        if self.reconnect_capped() {
            self.emit(JkBmsEvents::Connection(ConnectionState::Disconnected));
        } else {
            self.emit(JkBmsEvents::Connection(ConnectionState::Reconnecting));
        };

        let delay = RECONNECT_BACKOFF
            .get(self.reconnect_attempt - 1)
            .copied()
            .unwrap_or(RECONNECT_BACKOFF_MAX);

        self.delayed_commands
            .insert(JkBmsInternalCommands::Connect, delay);
    }

    // ── Polling ───────────────────────────────────────────────────────────────

    async fn run_data_polling(&mut self) {
        self.data_polling_scheduled = false;
        if self.transport.is_none() {
            return;
        }
        let now = Instant::now();
        let due = |last: Option<Instant>, interval: Duration| -> bool {
            last.is_none_or(|t| now.duration_since(t) >= interval)
        };

        if due(self.last_device_info, DEVICE_INFO_POLL_INTERVAL) {
            tracing::debug!("polling device info");
            if self
                .poll_with_escalation(JkBmsDataType::DeviceInfo)
                .await
                .is_err()
            {
                return;
            }
        }
        if due(self.last_config, CONFIG_POLL_INTERVAL) {
            tracing::debug!("polling config options");
            if self
                .poll_with_escalation(JkBmsDataType::ConfigOptions)
                .await
                .is_err()
            {
                return;
            }
        }
        if due(self.last_operational, OPERATIONAL_POLL_INTERVAL) {
            tracing::debug!("polling operational data");
            if self
                .poll_with_escalation(JkBmsDataType::OperationalData)
                .await
                .is_err()
            {
                return;
            }
        }
        if due(self.last_alarms, ALARM_POLL_INTERVAL) {
            tracing::debug!("polling alarms");
            if self
                .poll_with_escalation(JkBmsDataType::Alarms)
                .await
                .is_err()
            {
                return;
            }
        }

        self.schedule_data_polling_delayed(MIN_POLL_INTERVAL);
    }

    /// Poll one data type. Hard I/O errors trigger an immediate reconnect. Soft errors
    /// (parse failure or timeout) increment the stuck-state counter; once it reaches
    /// RECONNECT_THRESHOLD a synthetic disconnect is forced. Any successful poll resets
    /// the counter.
    async fn poll_with_escalation(&mut self, dtype: JkBmsDataType) -> Result<(), ()> {
        match self.poll_data(dtype).await {
            Ok(()) => {
                self.consecutive_failures = 0;
                Ok(())
            }
            Err(e) if is_hard_io(&e) => {
                self.handle_disconnect(e);
                Err(())
            }
            Err(e) => {
                self.consecutive_failures += 1;
                tracing::warn!(
                    "{dtype:?} poll failed ({}/{}): {e}",
                    self.consecutive_failures,
                    RECONNECT_THRESHOLD
                );
                if self.consecutive_failures >= RECONNECT_THRESHOLD {
                    self.handle_disconnect(JkBmsError::Disconnected);
                    return Err(());
                }
                Ok(())
            }
        }
    }

    async fn poll_data(&mut self, dtype: JkBmsDataType) -> Result<(), JkBmsError> {
        match dtype {
            JkBmsDataType::DeviceInfo => {
                let t = self
                    .transport
                    .as_mut()
                    .ok_or(JkBmsError::Disconnected)?
                    .as_mut();
                let info = self.protocol.poll_device_info(t).await?;
                tracing::debug!("device info poll ok");
                self.emit(JkBmsEvents::Data(JkBmsData::DeviceInfo(info)));
                self.last_device_info = Some(Instant::now());
            }
            JkBmsDataType::ConfigOptions => {
                let t = self
                    .transport
                    .as_mut()
                    .ok_or(JkBmsError::Disconnected)?
                    .as_mut();
                let cfg = self.protocol.poll_config(t).await?;
                tracing::debug!("config options poll ok");
                self.emit(JkBmsEvents::Data(JkBmsData::ConfigOptions(cfg)));
                self.last_config = Some(Instant::now());
            }
            JkBmsDataType::OperationalData => {
                let t = self
                    .transport
                    .as_mut()
                    .ok_or(JkBmsError::Disconnected)?
                    .as_mut();
                let data = self.protocol.poll_operational(t).await?;
                tracing::debug!("operational data poll ok");
                self.emit(JkBmsEvents::Data(JkBmsData::OperationalData(Box::new(
                    data,
                ))));
                self.last_operational = Some(Instant::now());
            }
            JkBmsDataType::Alarms => {
                let t = self
                    .transport
                    .as_mut()
                    .ok_or(JkBmsError::Disconnected)?
                    .as_mut();
                let value = self.protocol.poll_alarms(t).await?;
                tracing::debug!("alarms poll ok");
                self.emit(JkBmsEvents::Data(JkBmsData::Alarms(value)));
                self.last_alarms = Some(Instant::now());
            }
        }
        Ok(())
    }

    // ── Writes ────────────────────────────────────────────────────────────────

    async fn write_command(&mut self, command: WriteCommand, seq: u64) {
        // FIFO: queue while disconnected or behind earlier pending writes.
        if self.transport.is_none() || !self.pending_writes.is_empty() {
            // Outage persisted past the fast-reconnect window — stop accepting writes.
            if self.reconnect_capped() {
                tracing::warn!(seq, "reconnection capped; rejecting write");
                self.emit(JkBmsEvents::WriteError { seq });
                return;
            }
            if self.pending_writes.len() >= PENDING_WRITES_CAP {
                tracing::warn!(
                    seq,
                    cap = PENDING_WRITES_CAP,
                    "pending_writes queue full; rejecting write"
                );
                self.emit(JkBmsEvents::WriteError { seq });
                return;
            }
            self.pending_writes
                .push_back((command, seq, Instant::now()));
            if self.transport.is_some() {
                self.schedule_retry_writes_now();
            }
            return;
        }
        if let Err(e) = self.do_write(command, seq).await {
            self.pending_writes
                .push_back((command, seq, Instant::now()));
            self.handle_write_error(e);
        }
    }

    /// One pending write per invocation; self-reschedules if more remain. Head-of-queue
    /// TTL-expired writes are evicted first without counting as the attempt.
    async fn retry_writes(&mut self) {
        // Clear the guard before doing any work: further scheduling requests
        // during this handler (e.g. from `handle_write_error`) will re-arm.
        self.retry_writes_scheduled = false;
        if self.transport.is_none() {
            return;
        }
        while let Some(&(_, seq, queued_at)) = self.pending_writes.front() {
            if queued_at.elapsed() > WRITE_TTL {
                self.pending_writes.pop_front();
                self.emit(JkBmsEvents::WriteError { seq });
            } else {
                break;
            }
        }
        let Some(&(command, seq, _)) = self.pending_writes.front() else {
            return;
        };
        match self.do_write(command, seq).await {
            Ok(()) => {
                self.pending_writes.pop_front();
                if !self.pending_writes.is_empty() {
                    self.schedule_retry_writes_now();
                }
            }
            Err(e) => {
                // Leave the write at the front; handle_write_error schedules the next
                // RetryWrites on soft failure (or disconnects on hard / threshold hit).
                self.handle_write_error(e);
            }
        }
    }

    /// FC 0x10 switch write + POST_WRITE_SETTLE + ConfigOptions readback as one atomic
    /// handler (§9). Wire choreography lives in `self.protocol`; the two calls are
    /// atomic here because the actor does not `select!` between them.
    async fn do_write(&mut self, command: WriteCommand, seq: u64) -> Result<(), JkBmsError> {
        let t = self
            .transport
            .as_mut()
            .ok_or(JkBmsError::Disconnected)?
            .as_mut();
        tracing::info!(seq, command = ?command, "writing BMS command");
        self.protocol.write(t, command).await?;
        tokio::time::sleep(POST_WRITE_SETTLE).await;
        let t = self
            .transport
            .as_mut()
            .ok_or(JkBmsError::Disconnected)?
            .as_mut();
        let data = self.protocol.poll_config(t).await?;
        self.last_config = Some(Instant::now());
        self.consecutive_failures = 0;
        self.emit(JkBmsEvents::WriteConfirmation {
            seq,
            data: Box::new(data),
        });
        Ok(())
    }

    /// Shared escalation for write-path errors (`write_command` and `retry_writes`).
    /// Hard I/O → immediate reconnect. Soft error → increment `consecutive_failures`;
    /// at threshold force a synthetic disconnect, otherwise postpone `RetryWrites`
    /// by `RETRY_WRITE_SOFT_DELAY` to let the wire and BMS settle before retrying.
    fn handle_write_error(&mut self, e: JkBmsError) {
        if is_hard_io(&e) {
            self.handle_disconnect(e);
            return;
        }
        self.consecutive_failures += 1;
        tracing::warn!(
            "write failed ({}/{}): {e}",
            self.consecutive_failures,
            RECONNECT_THRESHOLD
        );
        if self.consecutive_failures >= RECONNECT_THRESHOLD {
            self.handle_disconnect(JkBmsError::Disconnected);
        } else {
            self.schedule_retry_writes_delayed(RETRY_WRITE_SOFT_DELAY);
        }
    }

    /// Enqueue an immediate `RunDataPolling` command, deduped by `data_polling_scheduled`.
    fn schedule_data_polling_now(&mut self) {
        if !self.data_polling_scheduled {
            self.data_polling_scheduled = true;
            self.schedule_internal(JkBmsInternalCommands::RunDataPolling);
        }
    }

    /// Enqueue a `RunDataPolling` command after `delay`, deduped by `data_polling_scheduled`.
    fn schedule_data_polling_delayed(&mut self, delay: Duration) {
        if !self.data_polling_scheduled {
            self.data_polling_scheduled = true;
            self.delayed_commands
                .insert(JkBmsInternalCommands::RunDataPolling, delay);
        }
    }

    /// Enqueue an immediate `RetryWrites` command, deduped by `retry_writes_scheduled`.
    fn schedule_retry_writes_now(&mut self) {
        if !self.retry_writes_scheduled {
            self.retry_writes_scheduled = true;
            self.schedule_internal(JkBmsInternalCommands::RetryWrites);
        }
    }

    /// Enqueue a `RetryWrites` command after `delay`, deduped by `retry_writes_scheduled`.
    fn schedule_retry_writes_delayed(&mut self, delay: Duration) {
        if !self.retry_writes_scheduled {
            self.retry_writes_scheduled = true;
            self.delayed_commands
                .insert(JkBmsInternalCommands::RetryWrites, delay);
        }
    }

    // ── Channel helpers ───────────────────────────────────────────────────────

    fn emit(&self, event: JkBmsEvents) {
        let _ = self.events_sender.send(event);
    }

    fn schedule_internal(&self, cmd: JkBmsInternalCommands) {
        let _ = self.internal_commands_sender.send(cmd);
    }
}

/// A hard I/O error (closed/absent fd) warrants an immediate close+reopen; read
/// timeouts and parse errors are soft and feed the stuck-state counter instead.
fn is_hard_io(e: &JkBmsError) -> bool {
    matches!(e, JkBmsError::Io(io) if io.kind() != io::ErrorKind::TimedOut)
}

#[cfg(test)]
pub(super) mod internals {
    use std::time::Duration;
    pub const WRITE_TTL: Duration = super::WRITE_TTL;
    pub const OPERATIONAL_POLL_INTERVAL: Duration = super::OPERATIONAL_POLL_INTERVAL;
    pub const RECONNECT_THRESHOLD: u32 = super::RECONNECT_THRESHOLD;
    pub const RECONNECT_BACKOFF: [Duration; 5] = super::RECONNECT_BACKOFF;
    pub const RECONNECT_BACKOFF_MAX: Duration = super::RECONNECT_BACKOFF_MAX;
}
