use std::collections::VecDeque;

use async_trait::async_trait;

use crate::jkbms::transport::IJkBmsTransport;
use crate::jkbms::{
    IJkBmsProtocol, JkBmsConfigOptions, JkBmsDeviceInfo, JkBmsError, JkBmsOperationalData,
    WriteCommand,
};

// ── Protocol mock ─────────────────────────────────────────────────────────────

/// Scripted step for `JkBmsProtocolMock`. Each variant carries the `Result` that
/// `poll_*` / `write` should return. Panics with a descriptive message if the
/// wrong variant is consumed (e.g. the actor calls `poll_config` but the next step
/// is `ProtocolStep::Operational`).
// Test-only mock; boxing the large variant would only churn call sites for no runtime benefit.
#[allow(clippy::large_enum_variant)]
pub enum ProtocolStep {
    DeviceInfo(Result<JkBmsDeviceInfo, JkBmsError>),
    Config(Result<JkBmsConfigOptions, JkBmsError>),
    Operational(Result<JkBmsOperationalData, JkBmsError>),
    Alarms(Result<u32, JkBmsError>),
    Write {
        command: WriteCommand,
        result: Result<(), JkBmsError>,
    },
}

pub struct JkBmsProtocolMock {
    steps: VecDeque<ProtocolStep>,
}

impl JkBmsProtocolMock {
    pub fn new(steps: Vec<ProtocolStep>) -> Self {
        Self {
            steps: VecDeque::from(steps),
        }
    }
}

#[async_trait]
impl IJkBmsProtocol for JkBmsProtocolMock {
    async fn poll_device_info(
        &mut self,
        _t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsDeviceInfo, JkBmsError> {
        match self.steps.pop_front() {
            Some(ProtocolStep::DeviceInfo(r)) => r,
            other => panic!(
                "JkBmsProtocolMock: expected ProtocolStep::DeviceInfo, got {}",
                step_name(&other)
            ),
        }
    }

    async fn poll_config(
        &mut self,
        _t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsConfigOptions, JkBmsError> {
        match self.steps.pop_front() {
            Some(ProtocolStep::Config(r)) => r,
            other => panic!(
                "JkBmsProtocolMock: expected ProtocolStep::Config, got {}",
                step_name(&other)
            ),
        }
    }

    async fn poll_operational(
        &mut self,
        _t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsOperationalData, JkBmsError> {
        match self.steps.pop_front() {
            Some(ProtocolStep::Operational(r)) => r,
            other => panic!(
                "JkBmsProtocolMock: expected ProtocolStep::Operational, got {}",
                step_name(&other)
            ),
        }
    }

    async fn poll_alarms(&mut self, _t: &mut dyn IJkBmsTransport) -> Result<u32, JkBmsError> {
        match self.steps.pop_front() {
            Some(ProtocolStep::Alarms(r)) => r,
            other => panic!(
                "JkBmsProtocolMock: expected ProtocolStep::Alarms, got {}",
                step_name(&other)
            ),
        }
    }

    async fn write(
        &mut self,
        _t: &mut dyn IJkBmsTransport,
        command: WriteCommand,
    ) -> Result<(), JkBmsError> {
        match self.steps.pop_front() {
            Some(ProtocolStep::Write {
                command: expected,
                result,
            }) => {
                assert_eq!(
                    command, expected,
                    "JkBmsProtocolMock::write: command mismatch"
                );
                result
            }
            other => panic!(
                "JkBmsProtocolMock: expected ProtocolStep::Write, got {}",
                step_name(&other)
            ),
        }
    }
}

fn step_name(step: &Option<ProtocolStep>) -> &'static str {
    match step {
        Some(ProtocolStep::DeviceInfo(_)) => "DeviceInfo",
        Some(ProtocolStep::Config(_)) => "Config",
        Some(ProtocolStep::Operational(_)) => "Operational",
        Some(ProtocolStep::Alarms(_)) => "Alarms",
        Some(ProtocolStep::Write { .. }) => "Write",
        None => "<empty queue>",
    }
}
