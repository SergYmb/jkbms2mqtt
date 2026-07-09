use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::task::JoinHandle;

use crate::jkbms::{IJkBmsConnection, RateLimitExceeded, WriteCommand};

pub struct JkBmsConnectionMock {
    writes: Mutex<Vec<(WriteCommand, u64)>>,
    stop_called: AtomicBool,
}

impl JkBmsConnectionMock {
    pub fn new() -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            stop_called: AtomicBool::new(false),
        }
    }

    pub fn writes(&self) -> Vec<(WriteCommand, u64)> {
        self.writes.lock().unwrap().clone()
    }

    pub fn stop_called(&self) -> bool {
        self.stop_called.load(Ordering::Relaxed)
    }
}

impl IJkBmsConnection for JkBmsConnectionMock {
    fn write(&self, command: WriteCommand, seq: u64) -> Result<(), RateLimitExceeded> {
        self.writes.lock().unwrap().push((command, seq));
        Ok(())
    }

    fn stop(self: Box<Self>) -> JoinHandle<()> {
        self.stop_called.store(true, Ordering::Relaxed);
        tokio::spawn(async {})
    }
}

impl IJkBmsConnection for std::sync::Arc<JkBmsConnectionMock> {
    fn write(&self, command: WriteCommand, seq: u64) -> Result<(), RateLimitExceeded> {
        self.as_ref().write(command, seq)
    }

    fn stop(self: Box<Self>) -> JoinHandle<()> {
        self.stop_called.store(true, Ordering::Relaxed);
        tokio::spawn(async {})
    }
}
