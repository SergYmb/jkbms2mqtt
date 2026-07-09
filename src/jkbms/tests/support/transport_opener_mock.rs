use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::jkbms::transport::{IJkBmsTransport, IJkBmsTransportOpener};

// ── Opener step definition ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TransportOpenerStep {
    /// `open()` returns `NotFound` for this duration of virtual Tokio time.
    PathDisappears { for_ms: u64 },
}

// ── JkBmsTransportStub ────────────────────────────────────────────────────────

/// No-op transport returned by `JkBmsTransportOpenerMock` on success.
/// The protocol mock intercepts all logical calls before they reach the transport,
/// so these methods should never be invoked. They panic to surface unexpected
/// call-throughs in tests.
pub struct JkBmsTransportStub;

#[async_trait]
impl IJkBmsTransport for JkBmsTransportStub {
    async fn write_all(&mut self, _bytes: &[u8]) -> io::Result<()> {
        panic!("JkBmsTransportStub::write_all called — the protocol mock should intercept this");
    }
    async fn read_exact(&mut self, _buf: &mut [u8]) -> io::Result<()> {
        panic!("JkBmsTransportStub::read_exact called — the protocol mock should intercept this");
    }
    async fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        panic!("JkBmsTransportStub::read called — the protocol mock should intercept this");
    }
}

// ── JkBmsTransportOpenerMock ──────────────────────────────────────────────────

struct OpenerState {
    steps: VecDeque<TransportOpenerStep>,
    disappeared_until: Option<tokio::time::Instant>,
    open_timestamps: Vec<tokio::time::Instant>,
}

/// Lightweight cloneable handle to the opener's shared state. Hold this before
/// passing the opener to `JkBmsConnection::new_with_injections` so tests can read
/// `open_timestamps` after the opener has been consumed by the actor.
#[derive(Clone)]
pub struct JkBmsTransportOpenerMockHandle {
    shared: Arc<Mutex<OpenerState>>,
}

impl JkBmsTransportOpenerMockHandle {
    pub fn open_timestamps(&self) -> Vec<tokio::time::Instant> {
        self.shared.lock().unwrap().open_timestamps.clone()
    }
}

pub struct JkBmsTransportOpenerMock {
    shared: Arc<Mutex<OpenerState>>,
}

impl JkBmsTransportOpenerMock {
    pub fn new(steps: Vec<TransportOpenerStep>) -> Self {
        Self {
            shared: Arc::new(Mutex::new(OpenerState {
                steps: VecDeque::from(steps),
                disappeared_until: None,
                open_timestamps: Vec::new(),
            })),
        }
    }

    pub fn handle(&self) -> JkBmsTransportOpenerMockHandle {
        JkBmsTransportOpenerMockHandle {
            shared: Arc::clone(&self.shared),
        }
    }
}

#[async_trait]
impl IJkBmsTransportOpener for JkBmsTransportOpenerMock {
    async fn open(&self) -> io::Result<Box<dyn IJkBmsTransport>> {
        let mut guard = self.shared.lock().unwrap();
        guard.open_timestamps.push(tokio::time::Instant::now());

        if let Some(until) = guard.disappeared_until {
            if tokio::time::Instant::now() < until {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            guard.disappeared_until = None;
        }

        if let Some(TransportOpenerStep::PathDisappears { for_ms }) = guard.steps.front() {
            let for_ms = *for_ms;
            guard.steps.pop_front();
            guard.disappeared_until =
                Some(tokio::time::Instant::now() + Duration::from_millis(for_ms));
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }

        Ok(Box::new(JkBmsTransportStub))
    }
}
