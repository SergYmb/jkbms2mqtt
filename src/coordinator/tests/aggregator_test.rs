use std::time::Duration;

use tokio::time::Instant;

use crate::coordinator::aggregator::StateAggregator;
use crate::coordinator::aggregator::internals::{
    compute_cell_aggregates, decode_alarms, format_iso8601, last_update_age_secs, total_power,
};
use crate::jkbms::{JkBmsConfigOptions, JkBmsDeviceInfo, JkBmsOperationalData};

fn dummy_device_info() -> JkBmsDeviceInfo {
    JkBmsDeviceInfo {
        model: "TestModel".into(),
        hardware_version: "1.0".into(),
        software_version: "1.0".into(),
        serial_number: "SN001".into(),
        power_cycle_count: 7,
    }
}

fn dummy_config(cell_count: u32) -> JkBmsConfigOptions {
    JkBmsConfigOptions {
        cell_count,
        charging_switch: true,
        balance_switch: false,
        battery_capacity_ah: 100.0,
        smart_sleep_voltage_v: 3.0,
        cell_undervoltage_protection_v: 2.8,
        cell_overvoltage_protection_v: 3.65,
        balance_trigger_voltage_v: 3.45,
    }
}

fn dummy_operational() -> JkBmsOperationalData {
    JkBmsOperationalData {
        cell_voltages_v: [3.3; 16],
        cell_resistances_ohm: [0.05; 16],
        total_voltage_v: 52.8,
        total_current_a: 5.0,
        soc_pct: 80,
        soh_pct: 100,
        capacity_remaining_ah: 80.0,
        total_cycle_capacity_ah: 1000.0,
        charging_cycles: 10,
        total_runtime_s: 3600,
        mos_temperature_c: 25.0,
        temperature_sensor_1_c: 22.0,
        temperature_sensor_2_c: 23.0,
        temperature_sensor_4_c: 21.0,
        temperature_sensor_5_c: 20.0,
        balancing_current_a: 0.0,
        balancing_active: false,
        charging_switch: true,
    }
}

#[tokio::test(start_paused = true)]
async fn switch_state_comes_from_config_not_operational() {
    let mut agg = StateAggregator::new();
    let mut cfg = dummy_config(8);
    cfg.balance_switch = true;
    cfg.charging_switch = false;
    agg.set_config_options(cfg);
    agg.set_operational(dummy_operational(), Instant::now());
    let snap = agg.snapshot().unwrap();
    assert!(snap.balance_switch, "balance_switch from config");
    assert!(!snap.charging_switch, "charging_switch from config");
}

#[tokio::test(start_paused = true)]
async fn returns_none_without_config_or_operational() {
    let mut agg = StateAggregator::new();
    assert!(agg.snapshot().is_none());

    agg.set_device_info(dummy_device_info());
    assert!(agg.snapshot().is_none(), "DeviceInfo alone is not enough");
}

#[tokio::test(start_paused = true)]
async fn returns_none_with_only_config() {
    let mut agg = StateAggregator::new();
    agg.set_config_options(dummy_config(8));
    assert!(
        agg.snapshot().is_none(),
        "ConfigOptions alone is not enough"
    );
}

#[tokio::test(start_paused = true)]
async fn merges_fragments_into_snapshot() {
    let mut agg = StateAggregator::new();
    agg.set_device_info(dummy_device_info());
    agg.set_config_options(dummy_config(8));
    agg.set_operational(dummy_operational(), Instant::now());

    let snap = agg.snapshot().expect("snapshot should be Some");
    assert_eq!(snap.cell_voltages_v.len(), 8);
    assert_eq!(snap.power_cycle_count, 7);
    assert!(snap.cell_aggregates.is_some());
    assert_eq!(snap.alarm_list, "");
}

#[tokio::test(start_paused = true)]
async fn cell_count_drives_cell_count_in_snapshot() {
    let mut agg = StateAggregator::new();
    agg.set_config_options(dummy_config(8));
    // Operational has 16 cells but config says 8 — aggregator should slice to 8
    agg.set_operational(dummy_operational(), Instant::now());

    let snap = agg.snapshot().unwrap();
    assert_eq!(snap.cell_voltages_v.len(), 8);
    assert_eq!(snap.cell_resistances_ohm.len(), 8);
}

#[tokio::test(start_paused = true)]
async fn alarms_threaded_through() {
    let mut agg = StateAggregator::new();
    agg.set_config_options(dummy_config(8));
    agg.set_operational(dummy_operational(), Instant::now());
    agg.set_alarms(0x0000_0010); // bit 4 → "Cell over-voltage protection"

    let snap = agg.snapshot().unwrap();
    assert_eq!(snap.alarm_raw, 0x0000_0010);
    assert!(
        snap.alarm_list.contains("Cell over-voltage protection"),
        "expected alarm description in list, got: {}",
        snap.alarm_list
    );
}

#[tokio::test(start_paused = true)]
async fn latest_fragment_wins() {
    let mut agg = StateAggregator::new();
    agg.set_config_options(dummy_config(8));

    let mut first = dummy_operational();
    first.total_voltage_v = 48.0;
    agg.set_operational(first, Instant::now());

    let mut second = dummy_operational();
    second.total_voltage_v = 52.8;
    agg.set_operational(second, Instant::now());

    let snap = agg.snapshot().unwrap();
    assert_eq!(snap.total_voltage_v, 52.8);
}

#[test]
fn has_device_info_and_config_flags() {
    let mut agg = StateAggregator::new();
    assert!(!agg.has_device_info());
    assert!(!agg.has_config_options());

    agg.set_device_info(dummy_device_info());
    assert!(agg.has_device_info());
    assert!(!agg.has_config_options());

    agg.set_config_options(dummy_config(8));
    assert!(agg.has_config_options());
}

#[tokio::test(start_paused = true)]
async fn alarm_update_resets_age() {
    let mut agg = StateAggregator::new();
    agg.set_config_options(dummy_config(8));
    agg.set_operational(dummy_operational(), Instant::now());

    tokio::time::advance(Duration::from_secs(10)).await;
    agg.set_alarms(0); // update at T=10
    tokio::time::advance(Duration::from_secs(3)).await; // now T=13

    let snap = agg.snapshot().unwrap();
    assert_eq!(
        snap.last_update_age_s, 3,
        "age should reflect alarm update, not operational"
    );
}

// ── decode_alarms ─────────────────────────────────────────────────────────────

#[test]
fn no_alarms_returns_empty() {
    assert_eq!(decode_alarms(0), "");
}

#[test]
fn single_bit_4() {
    assert_eq!(decode_alarms(1 << 4), "Cell over-voltage protection");
}

#[test]
fn multiple_bits() {
    let val = (1 << 4) | (1 << 6);
    assert_eq!(
        decode_alarms(val),
        "Cell over-voltage protection, Overcurrent charge protection"
    );
}

#[test]
fn unknown_bits_hex_fallback() {
    assert_eq!(decode_alarms(1 << 31), "unknown bits 0x80000000");
}

// ── format_iso8601 ────────────────────────────────────────────────────────────

#[test]
fn iso8601_doc_example() {
    assert_eq!(format_iso8601(34_906_707), "P404DT0H18M");
}

#[test]
fn iso8601_zero() {
    assert_eq!(format_iso8601(0), "P0DT0H0M");
}

#[test]
fn iso8601_exactly_one_day() {
    assert_eq!(format_iso8601(86_400), "P1DT0H0M");
}

#[test]
fn iso8601_hours_and_minutes() {
    assert_eq!(format_iso8601(9_000), "P0DT2H30M");
}

#[test]
fn iso8601_sub_minute_seconds_dropped() {
    assert_eq!(format_iso8601(61), "P0DT0H1M");
}

// ── compute_cell_aggregates ───────────────────────────────────────────────────

#[test]
fn cell_aggregates_exclude_inactive() {
    let voltages: Vec<f64> = vec![3.470, 3.472, 3.471, 3.471, 3.470, 3.470, 3.470, 3.470];
    let agg = compute_cell_aggregates(&voltages);
    assert_eq!(agg.min_v, 3.470);
    assert_eq!(agg.max_v, 3.472);
    let expected_delta = 3.472 - 3.470;
    assert!((agg.delta_v - expected_delta).abs() < 1e-9);
    let expected_avg = voltages.iter().sum::<f64>() / 8.0;
    assert!((agg.average_v - expected_avg).abs() < 1e-9);
}

#[test]
fn cell_voltage_min_max_cell_1_based() {
    let voltages = vec![3.470, 3.475, 3.471, 3.471, 3.470, 3.470, 3.470, 3.465];
    let agg = compute_cell_aggregates(&voltages);
    assert_eq!(agg.min_cell, 8, "min is cell 8, not 7");
    assert_eq!(agg.max_cell, 2, "max is cell 2");
}

#[test]
fn cell_voltage_delta() {
    let voltages = vec![3.328, 3.335, 3.330, 3.329];
    let agg = compute_cell_aggregates(&voltages);
    assert!((agg.delta_v - 0.007).abs() < 1e-9);
}

// ── total_power ───────────────────────────────────────────────────────────────

#[test]
fn power_signed() {
    let p = total_power(27.36, -8.800);
    let formatted = format!("{:.2}", p);
    assert_eq!(formatted, "-240.77");
}

// ── last_update_age_secs ──────────────────────────────────────────────────────

#[tokio::test]
async fn last_update_age_advances() {
    tokio::time::pause();
    let t = Instant::now();
    tokio::time::advance(Duration::from_secs(7)).await;
    assert_eq!(last_update_age_secs(t), 7);
}
