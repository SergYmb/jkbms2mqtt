use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::task::JoinHandle;

use crate::domain::Snapshot;
use crate::jkbms::JkBmsDeviceInfo;
use crate::mqtt::connection::{IMqttConnection, RateLimitExceeded};
use crate::mqtt::events::MqttEvents;

pub struct MqttConnectionMock {
    inner: Mutex<Inner>,
    pub events_tx: tokio::sync::mpsc::UnboundedSender<MqttEvents>,
    stop_called: AtomicBool,
}

struct Inner {
    snapshots: Vec<Snapshot>,
    availability: Vec<bool>,
    discovery_cell_counts: Vec<u32>,
    subscribe_commands_count: u32,
}

impl MqttConnectionMock {
    pub fn new() -> (Self, tokio::sync::mpsc::UnboundedReceiver<MqttEvents>) {
        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        let mock = Self {
            inner: Mutex::new(Inner {
                snapshots: Vec::new(),
                availability: Vec::new(),
                discovery_cell_counts: Vec::new(),
                subscribe_commands_count: 0,
            }),
            events_tx,
            stop_called: AtomicBool::new(false),
        };
        (mock, events_rx)
    }

    pub fn snapshots(&self) -> Vec<Snapshot> {
        self.inner.lock().unwrap().snapshots.clone()
    }

    pub fn last_snapshot(&self) -> Option<Snapshot> {
        self.inner.lock().unwrap().snapshots.last().cloned()
    }

    pub fn availability_calls(&self) -> Vec<bool> {
        self.inner.lock().unwrap().availability.clone()
    }

    pub fn discovery_cell_counts(&self) -> Vec<u32> {
        self.inner.lock().unwrap().discovery_cell_counts.clone()
    }

    pub fn subscribe_commands_count(&self) -> u32 {
        self.inner.lock().unwrap().subscribe_commands_count
    }

    pub fn stop_called(&self) -> bool {
        self.stop_called.load(Ordering::Relaxed)
    }
}

impl IMqttConnection for MqttConnectionMock {
    fn publish_snapshot(&self, snapshot: &Snapshot) -> Result<(), RateLimitExceeded> {
        self.inner.lock().unwrap().snapshots.push(snapshot.clone());
        Ok(())
    }

    fn publish_discovery(
        &self,
        _device_info: &JkBmsDeviceInfo,
        cell_count: u32,
    ) -> Result<(), RateLimitExceeded> {
        self.inner
            .lock()
            .unwrap()
            .discovery_cell_counts
            .push(cell_count);
        Ok(())
    }

    fn publish_availability(&self, online: bool) -> Result<(), RateLimitExceeded> {
        self.inner.lock().unwrap().availability.push(online);
        Ok(())
    }

    fn subscribe_to_commands(&self) -> Result<(), RateLimitExceeded> {
        self.inner.lock().unwrap().subscribe_commands_count += 1;
        Ok(())
    }

    fn stop(self: Box<Self>) -> JoinHandle<()> {
        self.stop_called.store(true, Ordering::Relaxed);
        tokio::spawn(async {})
    }
}

impl IMqttConnection for Arc<MqttConnectionMock> {
    fn publish_snapshot(&self, snapshot: &Snapshot) -> Result<(), RateLimitExceeded> {
        self.as_ref().publish_snapshot(snapshot)
    }

    fn publish_discovery(
        &self,
        device_info: &JkBmsDeviceInfo,
        cell_count: u32,
    ) -> Result<(), RateLimitExceeded> {
        self.as_ref().publish_discovery(device_info, cell_count)
    }

    fn publish_availability(&self, online: bool) -> Result<(), RateLimitExceeded> {
        self.as_ref().publish_availability(online)
    }

    fn subscribe_to_commands(&self) -> Result<(), RateLimitExceeded> {
        self.as_ref().subscribe_to_commands()
    }

    fn stop(self: Box<Self>) -> JoinHandle<()> {
        self.stop_called.store(true, Ordering::Relaxed);
        tokio::spawn(async {})
    }
}
