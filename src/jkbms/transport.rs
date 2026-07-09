use std::io;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;
use tokio_serial::SerialPortBuilderExt;

const BAUD_RATE: u32 = 115_200;

#[async_trait]
pub trait IJkBmsTransport: Send + Sync {
    async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()>;
    /// Read whatever bytes are currently available, up to `buf.len()`. Returns 0
    /// on EOF. Used by the protocol layer's post-soft-error drain to consume any
    /// late bytes still on the wire before the next TX.
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

// `Send + Sync` (not just `Send`): the connection manager runs as a spawned task, so
// its `run()` future must be `Send`. `async_trait` desugars `open(&self)` to a boxed
// future that borrows `&self` across the await; that future is `Send` only if the
// trait object is `Sync`. The production impls (`JkBmsTransportOpener`, `JkBmsTransport`) are
// both `Send + Sync`, so this costs nothing.
#[async_trait]
pub trait IJkBmsTransportOpener: Send + Sync {
    async fn open(&self) -> io::Result<Box<dyn IJkBmsTransport>>;
}

struct JkBmsTransport {
    inner: tokio_serial::SerialStream,
    last_activity: Option<Instant>,
}

#[async_trait]
impl IJkBmsTransport for JkBmsTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let elapsed_ms = self.last_activity.map(|t| t.elapsed().as_millis());
        tracing::trace!(
            dir = "TX",
            len = bytes.len(),
            elapsed_ms = ?elapsed_ms,
            bytes = %hex_display(bytes),
            "serial write"
        );
        let result = AsyncWriteExt::write_all(&mut self.inner, bytes).await;
        self.last_activity = Some(Instant::now());
        result
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        let elapsed_ms = self.last_activity.map(|t| t.elapsed().as_millis());
        let result = AsyncReadExt::read_exact(&mut self.inner, buf)
            .await
            .map(|_| ());
        self.last_activity = Some(Instant::now());
        if result.is_ok() {
            tracing::trace!(
                dir = "RX",
                len = buf.len(),
                elapsed_ms = ?elapsed_ms,
                bytes = %hex_display(buf),
                "serial read"
            );
        }
        result
    }

    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let result = AsyncReadExt::read(&mut self.inner, buf).await;
        if let Ok(n) = &result {
            if *n > 0 {
                self.last_activity = Some(Instant::now());
            }
        }
        result
    }
}

pub(super) struct JkBmsTransportOpener {
    pub(super) device_path: String,
}

#[async_trait]
impl IJkBmsTransportOpener for JkBmsTransportOpener {
    async fn open(&self) -> io::Result<Box<dyn IJkBmsTransport>> {
        let stream = tokio_serial::new(&self.device_path, BAUD_RATE)
            .open_native_async()
            .map_err(io::Error::from)?;
        Ok(Box::new(JkBmsTransport {
            inner: stream,
            last_activity: None,
        }))
    }
}

// ── Hex formatting with Frame 0x03 password redaction ────────────────────────
//
// Trace-level byte dumps of the full buffer (all JK frames are ≤ 300 bytes; a
// single long log line is fine and lets us cross-reference every offset).
//
// Frame 0x03 (Device Info) carries two 8-byte ASCII password fields at offsets
// 0x3E and 0x76. These MUST NOT be logged — mask them with ASCII 'X' before hex
// encoding. The original buffer is never mutated; a scratch copy is used.

const FRAME_03_MAGIC: [u8; 4] = [0x55, 0xAA, 0xEB, 0x90];
const PW1_OFFSET: usize = 0x3E;
const PW2_OFFSET: usize = 0x76;
const PW_LEN: usize = 8;

fn is_frame_03(buf: &[u8]) -> bool {
    buf.len() >= PW2_OFFSET + PW_LEN && buf[0..4] == FRAME_03_MAGIC && buf[4] == 0x03
}

fn hex_display(buf: &[u8]) -> String {
    let bytes: std::borrow::Cow<'_, [u8]> = if is_frame_03(buf) {
        let mut owned = buf.to_vec();
        owned[PW1_OFFSET..PW1_OFFSET + PW_LEN].copy_from_slice(&[b'X'; PW_LEN]);
        owned[PW2_OFFSET..PW2_OFFSET + PW_LEN].copy_from_slice(&[b'X'; PW_LEN]);
        std::borrow::Cow::Owned(owned)
    } else {
        std::borrow::Cow::Borrowed(buf)
    };

    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
pub(super) mod internals {
    pub fn hex_display(buf: &[u8]) -> String {
        super::hex_display(buf)
    }
}
