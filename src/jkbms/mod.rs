// Public surface: the handle, the event/command vocabulary, the parsed data types,
// the transport seam, and the parser error type. The actor itself
// (`connection_manager`) and the protocol encode/decode functions are private to the
// module — they are reached only internally via `super::protocol::` and are not
// reachable from outside.
mod commands;
mod config;
mod connection;
mod connection_manager;
mod protocol;
mod transport;
mod types;

pub use commands::{ConnectionState, JkBmsError, JkBmsEvents, RateLimitExceeded, WriteCommand};
pub use config::JkBmsConfig;
pub use connection::{IJkBmsConnection, JkBmsConnection};
pub use protocol::{IJkBmsProtocol, JkBmsParserError, JkBmsProtocol};
pub use types::{
    ALARM_DESCRIPTIONS, JkBmsConfigOptions, JkBmsData, JkBmsDataType, JkBmsDeviceInfo,
    JkBmsOperationalData, TOTAL_CELL_SLOTS,
};

#[cfg(test)]
mod tests;
