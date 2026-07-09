use crate::domain::{CellAggregates, Snapshot};
use crate::mqtt::formatter::{per_entity_publications, snapshot_json};

const BMS: &str = "my_jk_bms";

fn make_snapshot() -> Snapshot {
    Snapshot {
        total_voltage_v: 27.36,
        total_current_a: -8.800,
        total_power_w: -240.768,
        soc_pct: 86,
        soh_pct: 100,
        capacity_remaining_ah: 240.780,
        total_cycle_capacity_ah: 25292.242,
        battery_capacity_ah: 314.0,
        charging_cycles: 90,
        total_runtime_s: 34_906_707,
        total_runtime: "P404DT0H18M".to_string(),
        cell_voltages_v: vec![3.467, 3.465],
        cell_resistances_ohm: vec![0.003, 0.004],
        cell_aggregates: Some(CellAggregates {
            average_v: 3.466,
            min_v: 3.465,
            max_v: 3.467,
            delta_v: 0.002,
            min_cell: 2,
            max_cell: 1,
        }),
        mos_temperature_c: 19.3,
        temperature_sensor_1_c: 21.7,
        temperature_sensor_2_c: 21.4,
        temperature_sensor_4_c: 20.8,
        temperature_sensor_5_c: 20.5,
        balancing_current_a: 0.000,
        balancing_active: false,
        charging_switch: true,
        balance_switch: false,
        alarm_raw: 0,
        alarm_list: String::new(),
        power_cycle_count: 47,
        last_update_age_s: 2,
        jkbms_reconnect_count: 0,
        mqtt_reconnect_count: 0,
    }
}

fn payload_for<'a>(pubs: &'a [(String, Vec<u8>)], entity_id: &str) -> &'a str {
    let topic = format!("{}/{}/state", BMS, entity_id);
    pubs.iter()
        .find(|(t, _)| *t == topic)
        .map(|(_, p)| std::str::from_utf8(p).unwrap())
        .unwrap_or_else(|| panic!("no publication for {}", entity_id))
}

#[test]
fn per_entity_precision() {
    let snap = make_snapshot();
    let pubs = per_entity_publications(&snap, BMS);

    assert_eq!(payload_for(&pubs, "total_voltage"), "27.36");
    assert_eq!(payload_for(&pubs, "total_current"), "-8.800");
    assert_eq!(payload_for(&pubs, "cell_1_voltage"), "3.467");
    assert_eq!(payload_for(&pubs, "cell_2_voltage"), "3.465");
    assert_eq!(payload_for(&pubs, "cell_1_resistance"), "0.003");
    assert_eq!(payload_for(&pubs, "capacity_remaining"), "240.8");
    assert_eq!(payload_for(&pubs, "total_cycle_capacity"), "25292.2");
    assert_eq!(payload_for(&pubs, "battery_capacity_ah"), "314.0");
    assert_eq!(payload_for(&pubs, "mos_temperature"), "19.3");
    assert_eq!(payload_for(&pubs, "temperature_sensor_1"), "21.7");
    assert_eq!(payload_for(&pubs, "cell_voltage_average"), "3.466");
    assert_eq!(payload_for(&pubs, "cell_voltage_delta"), "0.002");
    assert_eq!(payload_for(&pubs, "cell_voltage_min_cell"), "2");
}

#[test]
fn signed_power_published() {
    // From mqtt-topics.md: 27.36 V × -8.800 A = -240.768 W → "-240.77" at 2 dp
    let mut snap = make_snapshot();
    snap.total_power_w = 27.36 * -8.800;
    let pubs = per_entity_publications(&snap, BMS);
    assert_eq!(payload_for(&pubs, "total_power"), "-240.77");
}

#[test]
fn binary_sensor_on_off() {
    let mut snap = make_snapshot();
    snap.balancing_active = true;
    let pubs = per_entity_publications(&snap, BMS);
    assert_eq!(payload_for(&pubs, "balancing_active"), "ON");

    snap.balancing_active = false;
    let pubs = per_entity_publications(&snap, BMS);
    assert_eq!(payload_for(&pubs, "balancing_active"), "OFF");
}

#[test]
fn switch_on_off() {
    let mut snap = make_snapshot();
    snap.charging_switch = false;
    let pubs = per_entity_publications(&snap, BMS);
    // switch entity_id is "charging", not "charging_switch"
    assert_eq!(payload_for(&pubs, "charging"), "OFF");

    snap.balance_switch = true;
    let pubs = per_entity_publications(&snap, BMS);
    assert_eq!(payload_for(&pubs, "balancing"), "ON");
}

#[test]
fn iso8601_in_total_runtime() {
    let snap = make_snapshot();
    let pubs = per_entity_publications(&snap, BMS);
    assert_eq!(payload_for(&pubs, "total_runtime"), "P404DT0H18M");
    assert_eq!(payload_for(&pubs, "total_runtime_seconds"), "34906707");
}

#[test]
fn json_snapshot_flat_and_keyed() {
    let snap = make_snapshot();
    let bytes = snapshot_json(&snap);
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let obj = v.as_object().unwrap();

    assert_eq!(obj["total_voltage"], "27.36");
    assert_eq!(obj["total_current"], "-8.800");
    assert_eq!(obj["alarm_list"], "");
    assert_eq!(obj["total_runtime"], "P404DT0H18M");
    assert_eq!(obj["total_runtime_seconds"], 34_906_707u32);
    assert_eq!(obj["cell_1_voltage"], "3.467");
    assert!(
        obj.get("soc").is_none(),
        "key 'soc' should not exist; correct key is 'state_of_charge'"
    );
    assert!(obj.contains_key("state_of_charge"));
    // No nesting — all keys are top-level
    assert!(obj.values().all(|v| !v.is_object()));
}
