use std::io;

use super::protocol::JkBmsParserError;
use super::types::{JkBmsConfigOptions, JkBmsData};

// ── Public control command ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteCommand {
    SetCharging(bool),
    SetBalancing(bool),
}

// ── Outgoing events (standalone channel: manager → consumer) ────────────────────

/// Outward serial-link status. Emitted as `JkBmsEvents::Connection(ConnectionState)`
/// whenever the manager either (re)establishes the link or completes the state
/// transition that follows a failed `opener.open()` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Link is up and the post-reconnect resync has completed.
    Connected,
    /// Link is down, still inside the fast-reconnect backoff window.
    Reconnecting,
    /// Link is down and the reconnect backoff schedule is exhausted (capped regime).
    Disconnected,
}

/// Everything the BMS connection manager emits to the outside world. Delivered on a
/// dedicated `mpsc::UnboundedReceiver<JkBmsEvents>` returned alongside the handle from
/// `JkBmsConnection::new`, so the manager never blocks on a slow or absent consumer.
/// There is no `seq` on `Data` because data flow is driven by the manager's own
/// internal polling, not by an external request that needs correlation.
#[derive(Debug)]
pub enum JkBmsEvents {
    /// Serial link state transition.
    Connection(ConnectionState),
    /// A freshly polled BMS data report (operational / config / device-info / alarms).
    Data(JkBmsData),
    /// A `JkBmsConnection::write` succeeded; `data` is the read-after-write ConfigOptions
    /// (Frame 0x01) carrying the committed switch-enable state. `seq` echoes the
    /// caller's write seq for switch-state freeze logic.
    WriteConfirmation {
        seq: u64,
        data: Box<JkBmsConfigOptions>,
    },
    /// A `JkBmsConnection::write` failed permanently (TTL expired or rejected). `seq`
    /// echoes the caller's write seq so an optimistic freeze can be cleared.
    WriteError { seq: u64 },
}

// ── Handle errors ───────────────────────────────────────────────────────────────

/// Returned by `JkBmsConnection` methods when the bounded command channel is full —
/// surfaces backpressure to the caller instead of blocking the manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitExceeded;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum JkBmsError {
    #[error("serial I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("frame parse error: {0}")]
    Parse(#[from] JkBmsParserError),
    #[error("connection manager not connected")]
    Disconnected,
}
