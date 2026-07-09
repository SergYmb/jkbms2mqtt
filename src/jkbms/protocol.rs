use std::io;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::time::Instant;

use super::commands::{JkBmsError, WriteCommand};
use super::transport::IJkBmsTransport;
use super::types::{JkBmsConfigOptions, JkBmsDeviceInfo, JkBmsOperationalData, TOTAL_CELL_SLOTS};

// ── Wire-level constants ──────────────────────────────────────────────────────

const FRAME_LEN: usize = 300;
const ACK_LEN: usize = 8;
const ALARM_RSP_LEN: usize = 9;
/// Minimum silence between consecutive master→slave transmissions (application-level gap;
/// satisfies the Modbus 3.5-char floor automatically). Virtual-clock-safe (tokio::time).
const INTERFRAME_GAP: Duration = Duration::from_millis(5);
/// Per-read timeout; a `TimedOut` I/O error from this is treated as a soft error
/// (stuck-state counter) rather than a hard reconnect trigger.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// After a soft error, wait this long for the wire to go quiet: if a `read()` call
/// inside `drain_stale` returns no bytes within this window, the port is considered
/// idle and the drain loop exits.
const DRAIN_QUIET: Duration = Duration::from_millis(100);
/// Safety cap so a stuck-broadcasting BMS can never trap the actor in drain.
const DRAIN_MAX_BYTES: usize = 4096;

const MAGIC: [u8; 4] = [0x55, 0xAA, 0xEB, 0x90];

// ── Public error type ─────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JkBmsParserError {
    #[error("wrong length: expected 300, got {0}")]
    Length(usize),
    #[error("bad magic header")]
    Magic,
    #[error("checksum mismatch")]
    Checksum,
}

// ── Protocol trait ────────────────────────────────────────────────────────────

/// Logical BMS operations, one method per frame type. The actor depends on this
/// trait; the production impl owns the wire choreography (gap enforcement, encoding,
/// timeouts, decoding). Tests inject `JkBmsProtocolMock` which scripts results at
/// the logical level without any byte vectors (ARCHITECTURE.md §7.2).
#[async_trait]
pub trait IJkBmsProtocol: Send + Sync {
    async fn poll_device_info(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsDeviceInfo, JkBmsError>;

    async fn poll_config(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsConfigOptions, JkBmsError>;

    async fn poll_operational(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsOperationalData, JkBmsError>;

    async fn poll_alarms(&mut self, t: &mut dyn IJkBmsTransport) -> Result<u32, JkBmsError>;

    /// FC 0x10 switch write + 8-byte ack.
    async fn write(
        &mut self,
        t: &mut dyn IJkBmsTransport,
        command: WriteCommand,
    ) -> Result<(), JkBmsError>;
}

// ── Production impl ───────────────────────────────────────────────────────────

/// Production `IJkBmsProtocol` impl. Owns the wire choreography: inter-frame gap
/// enforcement, request encoding, `read_exact` with timeout, ack draining, and frame
/// decoding. Does not own the transport — the actor passes it per call so it can
/// drop+reopen on hard I/O without coordinating with the protocol layer.
pub struct JkBmsProtocol {
    slave_id: u8,
    /// Updated after every TX/RX exchange; drives `await_gap` before the next TX.
    last_serial_activity: Option<Instant>,
    /// Set true when a previous protocol call returned a soft error (timeout /
    /// parse). Drives a lazy `drain_stale` call at the top of the next
    /// `trigger_frame` / `write_inner`, so any late bytes from the failed
    /// exchange cannot desynchronize the next request.
    needs_drain: bool,
}

impl JkBmsProtocol {
    pub fn new(slave_id: u8) -> Self {
        JkBmsProtocol {
            slave_id,
            last_serial_activity: None,
            needs_drain: false,
        }
    }
}

#[async_trait]
impl IJkBmsProtocol for JkBmsProtocol {
    async fn poll_device_info(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsDeviceInfo, JkBmsError> {
        let result = self.poll_device_info_inner(t).await;
        if matches!(&result, Err(e) if is_soft(e)) {
            self.needs_drain = true;
        }
        result
    }

    async fn poll_config(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsConfigOptions, JkBmsError> {
        let result = self.poll_config_inner(t).await;
        if matches!(&result, Err(e) if is_soft(e)) {
            self.needs_drain = true;
        }
        result
    }

    async fn poll_operational(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsOperationalData, JkBmsError> {
        let result = self.poll_operational_inner(t).await;
        if matches!(&result, Err(e) if is_soft(e)) {
            self.needs_drain = true;
        }
        result
    }

    async fn poll_alarms(&mut self, t: &mut dyn IJkBmsTransport) -> Result<u32, JkBmsError> {
        let result = self.poll_alarms_inner(t).await;
        if matches!(&result, Err(e) if is_soft(e)) {
            self.needs_drain = true;
        }
        result
    }

    async fn write(
        &mut self,
        t: &mut dyn IJkBmsTransport,
        command: WriteCommand,
    ) -> Result<(), JkBmsError> {
        let result = self.write_inner(t, command).await;
        if matches!(&result, Err(e) if is_soft(e)) {
            self.needs_drain = true;
        }
        result
    }
}

// ── Private impl helpers ──────────────────────────────────────────────────────

impl JkBmsProtocol {
    /// Sleep until at least `INTERFRAME_GAP` has elapsed since the last serial activity.
    /// No-op when no activity has been recorded yet (e.g. first call after reopen).
    async fn await_gap(&self) {
        if let Some(last) = self.last_serial_activity {
            let waited = last.elapsed();
            if waited < INTERFRAME_GAP {
                tokio::time::sleep(INTERFRAME_GAP - waited).await;
            }
        }
    }

    /// One request/response honoring the inter-frame gap before TX.
    async fn exchange(
        &mut self,
        t: &mut dyn IJkBmsTransport,
        req: &[u8],
        rsp: &mut [u8],
    ) -> Result<(), JkBmsError> {
        self.await_gap().await;
        t.write_all(req).await?;
        match tokio::time::timeout(READ_TIMEOUT, t.read_exact(rsp)).await {
            Ok(inner) => inner?,
            Err(_) => {
                tracing::trace!(
                    step = "exchange",
                    expected_len = rsp.len(),
                    timeout_ms = READ_TIMEOUT.as_millis(),
                    "serial read timeout"
                );
                return Err(io::Error::new(io::ErrorKind::TimedOut, "serial read timeout").into());
            }
        }
        self.last_serial_activity = Some(Instant::now());
        Ok(())
    }

    /// Read that immediately follows a previous response — no extra gap, just timeout.
    async fn read_into(
        &mut self,
        t: &mut dyn IJkBmsTransport,
        buf: &mut [u8],
    ) -> Result<(), JkBmsError> {
        match tokio::time::timeout(READ_TIMEOUT, t.read_exact(buf)).await {
            Ok(inner) => inner?,
            Err(_) => {
                tracing::trace!(
                    step = "read_into",
                    expected_len = buf.len(),
                    timeout_ms = READ_TIMEOUT.as_millis(),
                    "serial read timeout"
                );
                return Err(io::Error::new(io::ErrorKind::TimedOut, "serial read timeout").into());
            }
        }
        self.last_serial_activity = Some(Instant::now());
        Ok(())
    }

    /// Called at the top of `trigger_frame` and `write_inner`. If the previous
    /// call set `needs_drain`, runs `drain_stale` and clears the flag before the
    /// next TX so late bytes from the failed exchange are discarded.
    async fn drain_if_needed(&mut self, t: &mut dyn IJkBmsTransport) {
        if self.needs_drain {
            self.drain_stale(t).await;
            self.needs_drain = false;
        }
    }

    /// Drain any bytes still on the wire after a soft error (timeout / parse) so
    /// they cannot desynchronize the next request. Loops on best-effort `read()`
    /// until either the port has been quiet for `DRAIN_QUIET` or the safety cap
    /// `DRAIN_MAX_BYTES` is hit.
    async fn drain_stale(&mut self, t: &mut dyn IJkBmsTransport) {
        let mut buf = [0u8; 64];
        let mut total = 0usize;
        while total < DRAIN_MAX_BYTES {
            match tokio::time::timeout(DRAIN_QUIET, t.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => total += n,
                _ => break,
            }
        }
        if total > 0 {
            tracing::warn!(bytes = total, "drained stale bytes after soft error");
            self.last_serial_activity = Some(Instant::now());
        }
    }

    /// Trigger a 300-byte JK frame: write the FC 0x10 trigger, read the frame, then
    /// drain the trailing 8-byte FC 0x10 ack the BMS emits after every trigger.
    async fn trigger_frame(
        &mut self,
        t: &mut dyn IJkBmsTransport,
        req: [u8; 11],
    ) -> Result<[u8; FRAME_LEN], JkBmsError> {
        self.drain_if_needed(t).await;
        let mut frame = [0u8; FRAME_LEN];
        self.exchange(t, &req, &mut frame).await?;
        let mut ack = [0u8; ACK_LEN];
        self.read_into(t, &mut ack).await?;
        Ok(frame)
    }

    async fn poll_device_info_inner(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsDeviceInfo, JkBmsError> {
        let frame = self
            .trigger_frame(t, encode_trigger_device_info(self.slave_id))
            .await?;
        Ok(decode_device_info(&frame)?)
    }

    async fn poll_config_inner(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsConfigOptions, JkBmsError> {
        let frame = self
            .trigger_frame(t, encode_trigger_config(self.slave_id))
            .await?;
        Ok(decode_config_options(&frame)?)
    }

    async fn poll_operational_inner(
        &mut self,
        t: &mut dyn IJkBmsTransport,
    ) -> Result<JkBmsOperationalData, JkBmsError> {
        let frame = self
            .trigger_frame(t, encode_trigger_operational(self.slave_id))
            .await?;
        Ok(decode_operational_data(&frame)?)
    }

    async fn poll_alarms_inner(&mut self, t: &mut dyn IJkBmsTransport) -> Result<u32, JkBmsError> {
        let req = encode_alarm_read(self.slave_id);
        let mut rsp = [0u8; ALARM_RSP_LEN];
        self.exchange(t, &req, &mut rsp).await?;
        decode_alarm_response(self.slave_id, &rsp)
            .ok_or(JkBmsError::Parse(JkBmsParserError::Checksum))
    }

    async fn write_inner(
        &mut self,
        t: &mut dyn IJkBmsTransport,
        command: WriteCommand,
    ) -> Result<(), JkBmsError> {
        self.drain_if_needed(t).await;
        tracing::trace!(?command, "write: switch write");
        let slave = self.slave_id;
        let req = match command {
            WriteCommand::SetCharging(v) => encode_charge_switch(slave, v),
            WriteCommand::SetBalancing(v) => encode_balance_switch(slave, v),
        };
        let mut ack = [0u8; ACK_LEN];
        self.exchange(t, &req, &mut ack).await?;
        if !decode_write_ack(slave, &ack) {
            return Err(JkBmsError::Parse(JkBmsParserError::Magic));
        }
        tracing::trace!("write: switch ACK validated");
        Ok(())
    }
}

// ── Modbus RTU helpers ────────────────────────────────────────────────────────

/// CRC16 Modbus (polynomial 0xA001, init 0xFFFF, appended LE).
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            if crc & 0x0001 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn append_crc(buf: &mut Vec<u8>) {
    let crc = crc16(buf);
    buf.push((crc & 0xFF) as u8);
    buf.push((crc >> 8) as u8);
}

fn build_trigger(slave: u8, reg: u16) -> [u8; 11] {
    let rh = (reg >> 8) as u8;
    let rl = (reg & 0xFF) as u8;
    let mut buf = vec![slave, 0x10, rh, rl, 0x00, 0x01, 0x02, 0x00, 0x00];
    append_crc(&mut buf);
    buf.try_into().unwrap()
}

fn build_write(slave: u8, reg: u16, value: u32) -> [u8; 13] {
    let rh = (reg >> 8) as u8;
    let rl = (reg & 0xFF) as u8;
    let b = value.to_be_bytes();
    let mut buf = vec![
        slave, 0x10, rh, rl, 0x00, 0x02, 0x04, b[0], b[1], b[2], b[3],
    ];
    append_crc(&mut buf);
    buf.try_into().unwrap()
}

fn encode_trigger_device_info(slave: u8) -> [u8; 11] {
    build_trigger(slave, 0x161C)
}

fn encode_trigger_config(slave: u8) -> [u8; 11] {
    build_trigger(slave, 0x161E)
}

fn encode_trigger_operational(slave: u8) -> [u8; 11] {
    build_trigger(slave, 0x1620)
}

fn encode_charge_switch(slave: u8, enabled: bool) -> [u8; 13] {
    build_write(slave, 0x1070, enabled as u32)
}

fn encode_balance_switch(slave: u8, enabled: bool) -> [u8; 13] {
    build_write(slave, 0x1078, enabled as u32)
}

fn encode_alarm_read(slave: u8) -> [u8; 8] {
    let mut buf = vec![slave, 0x03, 0x12, 0xA0, 0x00, 0x02];
    append_crc(&mut buf);
    buf.try_into().unwrap()
}

fn decode_write_ack(slave: u8, buf: &[u8; 8]) -> bool {
    buf[0] == slave && buf[1] == 0x10
}

fn decode_alarm_response(slave: u8, buf: &[u8; 9]) -> Option<u32> {
    if buf[0] != slave || buf[1] != 0x03 || buf[2] != 0x04 {
        return None;
    }
    let expected_crc = crc16(&buf[..7]);
    let got_crc = u16::from(buf[7]) | (u16::from(buf[8]) << 8);
    if expected_crc != got_crc {
        return None;
    }
    Some(u32::from_be_bytes([buf[3], buf[4], buf[5], buf[6]]))
}

// ── JK Frame parsing helpers ──────────────────────────────────────────────────

fn validate(frame: &[u8]) -> Result<(), JkBmsParserError> {
    if frame.len() != FRAME_LEN {
        return Err(JkBmsParserError::Length(frame.len()));
    }
    if frame[..4] != MAGIC {
        return Err(JkBmsParserError::Magic);
    }
    let sum: u8 = frame[..299].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    if frame[299] != sum {
        return Err(JkBmsParserError::Checksum);
    }
    Ok(())
}

fn ascii_str(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

fn read_u16_le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
fn read_i16_le(b: &[u8], off: usize) -> i16 {
    i16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
fn read_i32_le(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn read_u32_le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn decode_device_info(raw: &[u8]) -> Result<JkBmsDeviceInfo, JkBmsParserError> {
    validate(raw)?;
    Ok(JkBmsDeviceInfo {
        model: ascii_str(&raw[6..19]),
        hardware_version: ascii_str(&raw[22..25]),
        software_version: ascii_str(&raw[30..35]),
        power_cycle_count: read_u32_le(raw, 42),
        serial_number: ascii_str(&raw[46..59]),
    })
}

fn decode_config_options(raw: &[u8]) -> Result<JkBmsConfigOptions, JkBmsParserError> {
    validate(raw)?;
    Ok(JkBmsConfigOptions {
        smart_sleep_voltage_v: read_i32_le(raw, 6) as f64 / 1000.0,
        cell_undervoltage_protection_v: read_i32_le(raw, 10) as f64 / 1000.0,
        cell_overvoltage_protection_v: read_i32_le(raw, 18) as f64 / 1000.0,
        balance_trigger_voltage_v: read_i32_le(raw, 26) as f64 / 1000.0,
        cell_count: read_i32_le(raw, 114) as u32,
        charging_switch: read_i32_le(raw, 118) != 0,
        balance_switch: read_i32_le(raw, 126) != 0,
        battery_capacity_ah: read_i32_le(raw, 130) as f64 / 1000.0,
    })
}

fn decode_operational_data(raw: &[u8]) -> Result<JkBmsOperationalData, JkBmsParserError> {
    validate(raw)?;
    let mut cell_voltages_v = [0.0f64; TOTAL_CELL_SLOTS];
    let mut cell_resistances_ohm = [0.0f64; TOTAL_CELL_SLOTS];
    for i in 0..TOTAL_CELL_SLOTS {
        cell_voltages_v[i] = read_u16_le(raw, 6 + i * 2) as f64 / 1000.0;
        cell_resistances_ohm[i] = read_i16_le(raw, 80 + i * 2).max(0) as f64 / 1000.0;
    }
    Ok(JkBmsOperationalData {
        cell_voltages_v,
        cell_resistances_ohm,
        total_voltage_v: read_u16_le(raw, 234) as f64 / 100.0,
        total_current_a: read_i32_le(raw, 158) as f64 / 1000.0,
        soc_pct: raw[173],
        soh_pct: raw[190],
        capacity_remaining_ah: read_i32_le(raw, 174) as f64 / 1000.0,
        total_cycle_capacity_ah: read_i32_le(raw, 186) as f64 / 1000.0,
        charging_cycles: read_i32_le(raw, 182),
        total_runtime_s: read_u32_le(raw, 194),
        mos_temperature_c: read_i16_le(raw, 144) as f64 / 10.0,
        temperature_sensor_1_c: read_i16_le(raw, 162) as f64 / 10.0,
        temperature_sensor_2_c: read_i16_le(raw, 164) as f64 / 10.0,
        temperature_sensor_4_c: read_i16_le(raw, 256) as f64 / 10.0,
        temperature_sensor_5_c: read_i16_le(raw, 258) as f64 / 10.0,
        balancing_current_a: read_i16_le(raw, 170) as f64 / 1000.0,
        balancing_active: raw[172] != 0,
        charging_switch: raw[198] != 0,
    })
}

// ── Soft-error predicate ──────────────────────────────────────────────────────

/// A soft error — timeout on read or a parse error on a fully received frame.
/// Distinguished from hard I/O so the drain path is skipped on hard errors
/// (the transport is about to be dropped and reopened).
fn is_soft(e: &JkBmsError) -> bool {
    matches!(e, JkBmsError::Io(io) if io.kind() == io::ErrorKind::TimedOut)
        || matches!(e, JkBmsError::Parse(_))
}

#[cfg(test)]
pub(super) mod internals {
    use super::JkBmsParserError;
    use crate::jkbms::{JkBmsConfigOptions, JkBmsDeviceInfo, JkBmsOperationalData};

    pub const MAGIC: [u8; 4] = super::MAGIC;

    pub fn crc16(data: &[u8]) -> u16 {
        super::crc16(data)
    }
    pub fn validate(frame: &[u8]) -> Result<(), JkBmsParserError> {
        super::validate(frame)
    }
    pub fn encode_trigger_device_info(slave: u8) -> [u8; 11] {
        super::encode_trigger_device_info(slave)
    }
    pub fn encode_trigger_config(slave: u8) -> [u8; 11] {
        super::encode_trigger_config(slave)
    }
    pub fn encode_trigger_operational(slave: u8) -> [u8; 11] {
        super::encode_trigger_operational(slave)
    }
    pub fn encode_alarm_read(slave: u8) -> [u8; 8] {
        super::encode_alarm_read(slave)
    }
    pub fn encode_charge_switch(slave: u8, enabled: bool) -> [u8; 13] {
        super::encode_charge_switch(slave, enabled)
    }
    pub fn encode_balance_switch(slave: u8, enabled: bool) -> [u8; 13] {
        super::encode_balance_switch(slave, enabled)
    }
    pub fn decode_alarm_response(slave: u8, buf: &[u8; 9]) -> Option<u32> {
        super::decode_alarm_response(slave, buf)
    }
    pub fn decode_device_info(raw: &[u8]) -> Result<JkBmsDeviceInfo, JkBmsParserError> {
        super::decode_device_info(raw)
    }
    pub fn decode_config_options(raw: &[u8]) -> Result<JkBmsConfigOptions, JkBmsParserError> {
        super::decode_config_options(raw)
    }
    pub fn decode_operational_data(raw: &[u8]) -> Result<JkBmsOperationalData, JkBmsParserError> {
        super::decode_operational_data(raw)
    }
}
