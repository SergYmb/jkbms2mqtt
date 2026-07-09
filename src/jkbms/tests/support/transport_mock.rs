use std::collections::VecDeque;
use std::io;

use async_trait::async_trait;

use crate::jkbms::transport::IJkBmsTransport;

// ── Script definition ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TransportStep {
    /// Assert the protocol writes exactly these bytes.
    Expect(Vec<u8>),
    /// Deliver these bytes to the next `read_exact` call.
    Reply(Vec<u8>),
    /// The next I/O call returns this error kind.
    Disconnect(io::ErrorKind),
    /// Bytes that will be delivered piecewise to subsequent `read()` calls
    /// (not `read_exact`). Simulates stale bytes still on the wire after a
    /// soft error so drain-recovery paths can be exercised.
    DrainBytes(Vec<u8>),
}

// ── JkBmsTransportMock ────────────────────────────────────────────────────────

pub struct JkBmsTransportMock {
    steps: VecDeque<TransportStep>,
    write_timestamps: Vec<tokio::time::Instant>,
}

impl JkBmsTransportMock {
    pub fn new(steps: Vec<TransportStep>) -> Self {
        Self {
            steps: VecDeque::from(steps),
            write_timestamps: Vec::new(),
        }
    }

    pub fn write_timestamps(&self) -> Vec<tokio::time::Instant> {
        self.write_timestamps.clone()
    }

    /// Number of script steps that have not yet been consumed. Lets drain-recovery
    /// tests assert that a `DrainBytes` step was actually consumed by `read()`.
    pub fn remaining_steps(&self) -> usize {
        self.steps.len()
    }
}

#[async_trait]
impl IJkBmsTransport for JkBmsTransportMock {
    async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_timestamps.push(tokio::time::Instant::now());
        match self.steps.pop_front() {
            Some(TransportStep::Expect(expected)) => {
                assert_eq!(
                    bytes,
                    expected.as_slice(),
                    "JkBmsTransportMock::write_all mismatch\n  got:      {:02X?}\n  expected: {:02X?}",
                    bytes,
                    expected,
                );
                Ok(())
            }
            Some(TransportStep::Disconnect(kind)) => Err(io::Error::from(kind)),
            other => panic!("JkBmsTransportMock::write_all: unexpected step {other:?}"),
        }
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        match self.steps.pop_front() {
            Some(TransportStep::Reply(data)) => {
                assert_eq!(
                    data.len(),
                    buf.len(),
                    "JkBmsTransportMock::read_exact: script has {} bytes but caller wants {}",
                    data.len(),
                    buf.len(),
                );
                buf.copy_from_slice(&data);
                Ok(())
            }
            Some(TransportStep::Disconnect(kind)) => Err(io::Error::from(kind)),
            other => panic!("JkBmsTransportMock::read_exact: unexpected step {other:?}"),
        }
    }

    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Only `DrainBytes` steps are consumed by `read`. Any other step (or
        // an empty queue) means "port is quiet" — return 0 without touching
        // the script so existing tests remain unaffected by the drain path.
        let Some(TransportStep::DrainBytes(_)) = self.steps.front() else {
            return Ok(0);
        };
        let Some(TransportStep::DrainBytes(mut data)) = self.steps.pop_front() else {
            unreachable!()
        };
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        if n < data.len() {
            data.drain(..n);
            self.steps.push_front(TransportStep::DrainBytes(data));
        }
        Ok(n)
    }
}
