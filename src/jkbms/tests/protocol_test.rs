use std::io;
use std::time::Duration;

use crate::jkbms::{IJkBmsProtocol, JkBmsError, JkBmsParserError, JkBmsProtocol, WriteCommand};

use super::super::protocol::internals::*;
use super::support::build_fixture::{build_frame02, parse_hex_fixture, stamp_checksum};
use super::support::transport_mock::{JkBmsTransportMock, TransportStep};

// ── Modbus encode ─────────────────────────────────────────────────────────────

#[test]
fn encode_trigger_device_info_bytes() {
    assert_eq!(
        encode_trigger_device_info(1),
        [
            0x01, 0x10, 0x16, 0x1C, 0x00, 0x01, 0x02, 0x00, 0x00, 0xD3, 0xCD
        ]
    );
}

#[test]
fn encode_trigger_config_bytes() {
    assert_eq!(
        encode_trigger_config(1),
        [
            0x01, 0x10, 0x16, 0x1E, 0x00, 0x01, 0x02, 0x00, 0x00, 0xD2, 0x2F
        ]
    );
}

#[test]
fn encode_trigger_operational_bytes() {
    assert_eq!(
        encode_trigger_operational(1),
        [
            0x01, 0x10, 0x16, 0x20, 0x00, 0x01, 0x02, 0x00, 0x00, 0xD6, 0xF1
        ]
    );
}

#[test]
fn encode_alarm_read_bytes() {
    assert_eq!(
        encode_alarm_read(1),
        [0x01, 0x03, 0x12, 0xA0, 0x00, 0x02, 0xC1, 0x51]
    );
}

#[test]
fn encode_charge_switch_enable() {
    assert_eq!(
        encode_charge_switch(1, true),
        [
            0x01, 0x10, 0x10, 0x70, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00, 0x01, 0xF8, 0x8B
        ]
    );
}

#[test]
fn encode_charge_switch_disable() {
    assert_eq!(
        encode_charge_switch(1, false),
        [
            0x01, 0x10, 0x10, 0x70, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00, 0x00, 0x39, 0x4B
        ]
    );
}

#[test]
fn encode_balance_switch_enable() {
    assert_eq!(
        encode_balance_switch(1, true),
        [
            0x01, 0x10, 0x10, 0x78, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00, 0x01, 0xF9, 0x2D
        ]
    );
}

#[test]
fn encode_balance_switch_disable() {
    assert_eq!(
        encode_balance_switch(1, false),
        [
            0x01, 0x10, 0x10, 0x78, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00, 0x00, 0x38, 0xED
        ]
    );
}

#[test]
fn decode_alarm_response_no_active() {
    let buf: [u8; 9] = [0x01, 0x03, 0x04, 0x00, 0x00, 0x00, 0x00, 0xFA, 0x33];
    assert_eq!(decode_alarm_response(1, &buf), Some(0));
}

#[test]
fn decode_alarm_response_crc_mismatch() {
    let mut buf: [u8; 9] = [0x01, 0x03, 0x04, 0x00, 0x00, 0x00, 0x00, 0xFA, 0x33];
    buf[7] ^= 0xFF;
    assert_eq!(decode_alarm_response(1, &buf), None);
}

#[test]
fn crc16_roundtrip() {
    let frame = encode_trigger_operational(1);
    let crc = crc16(&frame[..9]);
    assert_eq!(crc, u16::from(frame[9]) | (u16::from(frame[10]) << 8));
}

// ── JK Frame Validation ───────────────────────────────────────────────────────

#[test]
fn rejects_short_frame() {
    let frame = vec![0u8; 299];
    assert_eq!(validate(&frame), Err(JkBmsParserError::Length(299)));
}

#[test]
fn rejects_bad_magic() {
    let mut frame = [0u8; 300];
    frame[0] = 0xFF;
    frame[1] = 0xAA;
    frame[2] = 0xEB;
    frame[3] = 0x90;
    stamp_checksum(&mut frame);
    assert_eq!(validate(&frame), Err(JkBmsParserError::Magic));
}

#[test]
fn rejects_bad_checksum() {
    let mut frame = [0u8; 300];
    frame[..4].copy_from_slice(&MAGIC);
    stamp_checksum(&mut frame);
    frame[10] ^= 0xFF;
    assert_eq!(validate(&frame), Err(JkBmsParserError::Checksum));
}

// ── Device Info (Frame 0x03) ──────────────────────────────────────────────────

#[test]
fn decode_device_info_fixture() {
    let raw = parse_hex_fixture(include_str!("fixtures/frame_03_device_info.hex"));
    let f = decode_device_info(&raw).unwrap();
    assert_eq!(f.model, "JK_PB2A16S20P");
    assert_eq!(f.hardware_version, "15A");
    assert_eq!(f.software_version, "15.41");
    assert_eq!(f.power_cycle_count, 39);
    // serial_number field is redacted per PII rules — not asserted
}

// ── Configuration Options (Frame 0x01) ────────────────────────────────────────

#[test]
fn decode_config_options_fixture() {
    let raw = parse_hex_fixture(include_str!("fixtures/frame_01_config.hex"));
    let f = decode_config_options(&raw).unwrap();
    assert_eq!(f.cell_count, 8);
    assert!((f.cell_undervoltage_protection_v - 2.630).abs() < 0.001);
    assert!((f.cell_overvoltage_protection_v - 3.650).abs() < 0.001);
    assert!((f.balance_trigger_voltage_v - 0.005).abs() < 0.0001);
    assert!((f.smart_sleep_voltage_v - 3.400).abs() < 0.001);
    assert!((f.battery_capacity_ah - 314.0).abs() < 0.001);
}

// ── Operational Data (Frame 0x02) ─────────────────────────────────────────────

#[test]
fn decode_operational_data_eight_cells() {
    let raw = parse_hex_fixture(include_str!("fixtures/frame_02_operational.hex"));
    let f = decode_operational_data(&raw).unwrap();
    assert_eq!(f.cell_voltages_v.len(), 16);
    assert_eq!(f.cell_resistances_ohm.len(), 16);

    assert!((f.cell_voltages_v[0] - 3.470).abs() < 0.001, "cell 1");
    assert!((f.cell_voltages_v[1] - 3.472).abs() < 0.001, "cell 2");
    assert!((f.cell_voltages_v[7] - 3.470).abs() < 0.001, "cell 8");

    assert!((f.total_voltage_v - 27.76).abs() < 0.01);
    assert!((f.total_current_a - (-0.386)).abs() < 0.001);
    assert!((f.mos_temperature_c - 26.7).abs() < 0.1);
    assert!(f.charging_switch, "charging switch should be on");
    assert!(!f.balancing_active, "not actively balancing");
}

#[test]
fn operational_data_signed_current() {
    let current_raw: i32 = -8800;
    let bytes = current_raw.to_le_bytes();
    let f = build_frame02(&[(158, &bytes), (234, &2736u16.to_le_bytes())]);
    let frame = decode_operational_data(&f).unwrap();
    assert!((frame.total_current_a - (-8.800)).abs() < 0.001);
}

#[test]
fn operational_data_negative_resistance_clamped() {
    let neg: i16 = -1;
    let f = build_frame02(&[(80, &neg.to_le_bytes())]);
    let frame = decode_operational_data(&f).unwrap();
    assert_eq!(
        frame.cell_resistances_ohm[0], 0.0,
        "negative resistance must clamp to 0"
    );
}

#[test]
fn operational_data_soc_and_soh() {
    let f = build_frame02(&[(173, &[99u8]), (190, &[100u8])]);
    let frame = decode_operational_data(&f).unwrap();
    assert_eq!(frame.soc_pct, 99);
    assert_eq!(frame.soh_pct, 100);
}

// ── Alarms ────────────────────────────────────────────────────────────────────

#[test]
fn decode_alarm_response_be_uint32() {
    let mut buf = [0u8; 9];
    buf[0] = 1;
    buf[1] = 0x03;
    buf[2] = 0x04;
    buf[3] = 0xD3;
    buf[4] = 0xD2;
    buf[5] = 0xD1;
    buf[6] = 0xD0;
    let crc = crc16(&buf[..7]);
    buf[7] = (crc & 0xFF) as u8;
    buf[8] = (crc >> 8) as u8;
    assert_eq!(decode_alarm_response(1, &buf), Some(0xD3D2D1D0));
}

#[test]
fn decode_alarm_response_bad_crc() {
    let mut buf: [u8; 9] = [0x01, 0x03, 0x04, 0x00, 0x00, 0x00, 0x00, 0xFA, 0x33];
    buf[7] ^= 0xFF;
    assert_eq!(decode_alarm_response(1, &buf), None);
}

// ── Wire constants (values verified by encode tests above) ────────────────────

const TRIGGER_DEVICE_INFO: [u8; 11] = [
    0x01, 0x10, 0x16, 0x1C, 0x00, 0x01, 0x02, 0x00, 0x00, 0xD3, 0xCD,
];
const TRIGGER_CONFIG: [u8; 11] = [
    0x01, 0x10, 0x16, 0x1E, 0x00, 0x01, 0x02, 0x00, 0x00, 0xD2, 0x2F,
];
const TRIGGER_OPERATIONAL: [u8; 11] = [
    0x01, 0x10, 0x16, 0x20, 0x00, 0x01, 0x02, 0x00, 0x00, 0xD6, 0xF1,
];
const ALARM_READ: [u8; 8] = [0x01, 0x03, 0x12, 0xA0, 0x00, 0x02, 0xC1, 0x51];
const ALARM_NO_ACTIVE: [u8; 9] = [0x01, 0x03, 0x04, 0x00, 0x00, 0x00, 0x00, 0xFA, 0x33];
const WRITE_CHARGE_ENABLE: [u8; 13] = [
    0x01, 0x10, 0x10, 0x70, 0x00, 0x02, 0x04, 0x00, 0x00, 0x00, 0x01, 0xF8, 0x8B,
];
/// Any FC 0x10 ack — decode_write_ack only checks bytes [0] and [1].
const WRITE_ACK: [u8; 8] = [0x01, 0x10, 0x10, 0x70, 0x00, 0x02, 0x44, 0xD3];
/// 8-byte ack drained after every trigger-frame — content ignored by trigger_frame.
const TRIGGER_ACK: [u8; 8] = [0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

const INTERFRAME_GAP: Duration = Duration::from_millis(5);

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn frame_device_info() -> Vec<u8> {
    parse_hex_fixture(include_str!("fixtures/frame_03_device_info.hex"))
}
fn frame_config() -> Vec<u8> {
    parse_hex_fixture(include_str!("fixtures/frame_01_config.hex"))
}
fn frame_operational() -> Vec<u8> {
    parse_hex_fixture(include_str!("fixtures/frame_02_operational.hex"))
}

// ── Protocol integration tests ────────────────────────────────────────────────

/// Trigger → 300-byte device-info frame → 8-byte ack; result round-trips.
#[tokio::test(start_paused = true)]
async fn poll_device_info_round_trip() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(TRIGGER_DEVICE_INFO.to_vec()),
        TransportStep::Reply(frame_device_info()),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let info = p.poll_device_info(&mut t).await.unwrap();
    assert!(!info.model.is_empty());
}

/// Trigger → 300-byte config frame → 8-byte ack; cell_count is within valid range.
#[tokio::test(start_paused = true)]
async fn poll_config_round_trip() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(TRIGGER_CONFIG.to_vec()),
        TransportStep::Reply(frame_config()),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let cfg = p.poll_config(&mut t).await.unwrap();
    assert!(cfg.cell_count > 0 && cfg.cell_count <= 32);
}

/// Trigger → 300-byte frame → 8-byte ack fully consumed; cell_voltages length matches.
#[tokio::test(start_paused = true)]
async fn poll_operational_round_trip_drains_ack() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Reply(frame_operational()),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let data = p.poll_operational(&mut t).await.unwrap();
    assert_eq!(data.cell_voltages_v.len(), 16);
}

/// Alarm read → 9-byte response with no active alarms → returns 0.
#[tokio::test(start_paused = true)]
async fn poll_alarms_no_active_round_trip() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(ALARM_READ.to_vec()),
        TransportStep::Reply(ALARM_NO_ACTIVE.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let alarms = p.poll_alarms(&mut t).await.unwrap();
    assert_eq!(alarms, 0);
}

/// FC 0x10 write → 8-byte ack. The ConfigOptions readback now lives in
/// `connection_manager::do_write`, not in `protocol.write`.
#[tokio::test(start_paused = true)]
async fn write_charge_then_ack() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(WRITE_CHARGE_ENABLE.to_vec()),
        TransportStep::Reply(WRITE_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    p.write(&mut t, WriteCommand::SetCharging(true))
        .await
        .unwrap();
}

/// Second TX is at least INTERFRAME_GAP after the previous RX completed.
#[tokio::test(start_paused = true)]
async fn inter_frame_gap_enforced() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(TRIGGER_DEVICE_INFO.to_vec()),
        TransportStep::Reply(frame_device_info()),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
        TransportStep::Expect(TRIGGER_CONFIG.to_vec()),
        TransportStep::Reply(frame_config()),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);

    p.poll_device_info(&mut t).await.unwrap();
    p.poll_config(&mut t).await.unwrap();

    let ts = t.write_timestamps();
    assert_eq!(ts.len(), 2, "expected exactly 2 TX calls");
    assert!(
        ts[1] >= ts[0] + INTERFRAME_GAP,
        "second TX at {:?} is too soon after first TX at {:?} (gap < {:?})",
        ts[1],
        ts[0],
        INTERFRAME_GAP,
    );
}

/// Transport returns TimedOut on read → JkBmsError::Io(TimedOut) (soft error, not hard).
#[tokio::test(start_paused = true)]
async fn read_timeout_is_soft_error() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Disconnect(io::ErrorKind::TimedOut),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let err = p.poll_operational(&mut t).await.unwrap_err();
    assert!(
        matches!(&err, JkBmsError::Io(e) if e.kind() == io::ErrorKind::TimedOut),
        "expected TimedOut IO error, got: {err:?}",
    );
}

/// Transport returns Other on read → JkBmsError::Io(Other) (hard error, triggers reconnect).
#[tokio::test(start_paused = true)]
async fn enodev_is_hard_error() {
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Disconnect(io::ErrorKind::Other),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let err = p.poll_operational(&mut t).await.unwrap_err();
    assert!(
        matches!(&err, JkBmsError::Io(e) if e.kind() != io::ErrorKind::TimedOut),
        "expected hard IO error, got: {err:?}",
    );
}

/// Corrupted checksum byte in the 300-byte frame → JkBmsError::Parse.
#[tokio::test(start_paused = true)]
async fn bad_checksum_is_parse_error() {
    let mut frame = frame_operational();
    frame[299] ^= 0xFF;
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Reply(frame),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let err = p.poll_operational(&mut t).await.unwrap_err();
    assert!(
        matches!(err, JkBmsError::Parse(_)),
        "expected Parse error, got: {err:?}"
    );
}

/// Read timeout on operational poll → `needs_drain` flag set. Drain runs at
/// the top of the *next* `trigger_frame`, consuming stale bytes before the
/// retry's TX.
#[tokio::test(start_paused = true)]
async fn soft_timeout_drains_stale_bytes_before_next_trigger() {
    let mut t = JkBmsTransportMock::new(vec![
        // First poll: TX ok, RX times out.
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Disconnect(io::ErrorKind::TimedOut),
        // Stale bytes now sit on the wire.
        TransportStep::DrainBytes(vec![0xAA; 32]),
        // Second poll: drain runs first (consumes DrainBytes), then TX + RX succeed.
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Reply(frame_operational()),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let err = p.poll_operational(&mut t).await.unwrap_err();
    assert!(
        matches!(&err, JkBmsError::Io(e) if e.kind() == io::ErrorKind::TimedOut),
        "expected TimedOut IO error, got: {err:?}",
    );
    // Advance past the inter-frame gap so the retry's TX is not delayed by await_gap.
    tokio::time::advance(INTERFRAME_GAP).await;
    p.poll_operational(&mut t).await.unwrap();
    assert_eq!(
        t.remaining_steps(),
        0,
        "expected all script steps to be consumed (drain + retry)"
    );
}

/// Parse error on the frame → `needs_drain` flag set. Drain runs at the top
/// of the next `trigger_frame`, consuming stale bytes before the retry's TX.
#[tokio::test(start_paused = true)]
async fn parse_error_drains_stale_bytes_before_next_trigger() {
    let mut bad_frame = frame_operational();
    bad_frame[299] ^= 0xFF;
    let mut t = JkBmsTransportMock::new(vec![
        // First poll: TX ok, RX returns a frame with bad checksum + ACK, decode fails.
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Reply(bad_frame),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
        // Stale bytes now sit on the wire.
        TransportStep::DrainBytes(vec![0xBB; 16]),
        // Second poll: drain runs first, then TX + RX succeed.
        TransportStep::Expect(TRIGGER_OPERATIONAL.to_vec()),
        TransportStep::Reply(frame_operational()),
        TransportStep::Reply(TRIGGER_ACK.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let err = p.poll_operational(&mut t).await.unwrap_err();
    assert!(
        matches!(err, JkBmsError::Parse(_)),
        "expected Parse error, got: {err:?}"
    );
    tokio::time::advance(INTERFRAME_GAP).await;
    p.poll_operational(&mut t).await.unwrap();
    assert_eq!(
        t.remaining_steps(),
        0,
        "expected all script steps to be consumed (drain + retry)"
    );
}

/// Alarm response with wrong slave ID → decode_alarm_response returns None → JkBmsError::Parse.
#[tokio::test(start_paused = true)]
async fn wrong_slave_in_alarm_response_returns_parse_error() {
    let mut rsp = ALARM_NO_ACTIVE;
    rsp[0] = 0x02;
    let mut t = JkBmsTransportMock::new(vec![
        TransportStep::Expect(ALARM_READ.to_vec()),
        TransportStep::Reply(rsp.to_vec()),
    ]);
    let mut p = JkBmsProtocol::new(1);
    let err = p.poll_alarms(&mut t).await.unwrap_err();
    assert!(
        matches!(err, JkBmsError::Parse(_)),
        "expected Parse error, got: {err:?}"
    );
}
