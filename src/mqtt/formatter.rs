use serde_json::{Map, Value};

use crate::domain::Snapshot;

use super::topics;

pub fn per_entity_publications(snapshot: &Snapshot, bms_name: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pub_str = |entity_id: &str, value: String| {
        let topic = topics::state_topic(bms_name, entity_id);
        out.push((topic, value.into_bytes()));
    };

    // Pack sensors
    pub_str("total_voltage", format!("{:.2}", snapshot.total_voltage_v));
    pub_str("total_current", format!("{:.3}", snapshot.total_current_a));
    pub_str("total_power", format!("{:.2}", snapshot.total_power_w));
    pub_str("state_of_charge", snapshot.soc_pct.to_string());
    pub_str("state_of_health", snapshot.soh_pct.to_string());
    pub_str(
        "capacity_remaining",
        format!("{:.1}", snapshot.capacity_remaining_ah),
    );
    pub_str(
        "total_cycle_capacity",
        format!("{:.1}", snapshot.total_cycle_capacity_ah),
    );
    pub_str(
        "battery_capacity_ah",
        format!("{:.1}", snapshot.battery_capacity_ah),
    );
    pub_str("charging_cycles", snapshot.charging_cycles.to_string());
    pub_str(
        "total_runtime_seconds",
        snapshot.total_runtime_s.to_string(),
    );
    pub_str("total_runtime", snapshot.total_runtime.clone());

    // Cell sensors
    for (i, &v) in snapshot.cell_voltages_v.iter().enumerate() {
        pub_str(&format!("cell_{}_voltage", i + 1), format!("{:.3}", v));
    }
    for (i, &r) in snapshot.cell_resistances_ohm.iter().enumerate() {
        pub_str(&format!("cell_{}_resistance", i + 1), format!("{:.3}", r));
    }

    // Cell aggregates
    if let Some(ref agg) = snapshot.cell_aggregates {
        pub_str("cell_voltage_average", format!("{:.3}", agg.average_v));
        pub_str("cell_voltage_min", format!("{:.3}", agg.min_v));
        pub_str("cell_voltage_max", format!("{:.3}", agg.max_v));
        pub_str("cell_voltage_delta", format!("{:.3}", agg.delta_v));
        pub_str("cell_voltage_min_cell", agg.min_cell.to_string());
        pub_str("cell_voltage_max_cell", agg.max_cell.to_string());
    }

    // Temperature sensors
    pub_str(
        "mos_temperature",
        format!("{:.1}", snapshot.mos_temperature_c),
    );
    pub_str(
        "temperature_sensor_1",
        format!("{:.1}", snapshot.temperature_sensor_1_c),
    );
    pub_str(
        "temperature_sensor_2",
        format!("{:.1}", snapshot.temperature_sensor_2_c),
    );
    pub_str(
        "temperature_sensor_4",
        format!("{:.1}", snapshot.temperature_sensor_4_c),
    );
    pub_str(
        "temperature_sensor_5",
        format!("{:.1}", snapshot.temperature_sensor_5_c),
    );

    // Balancer
    pub_str(
        "balancing_current",
        format!("{:.3}", snapshot.balancing_current_a),
    );
    pub_str(
        "balancing_active",
        on_off(snapshot.balancing_active).to_string(),
    );

    // Switches (entity_id matches the switch name, not the field name)
    pub_str("charging", on_off(snapshot.charging_switch).to_string());
    pub_str("balancing", on_off(snapshot.balance_switch).to_string());

    // Alarm
    pub_str("alarm_list", snapshot.alarm_list.clone());

    // Diagnostics
    pub_str("power_cycle_count", snapshot.power_cycle_count.to_string());
    pub_str("last_update_age", snapshot.last_update_age_s.to_string());
    pub_str(
        "jkbms_reconnect_count",
        snapshot.jkbms_reconnect_count.to_string(),
    );
    pub_str(
        "mqtt_reconnect_count",
        snapshot.mqtt_reconnect_count.to_string(),
    );

    out
}

pub fn snapshot_json(snapshot: &Snapshot) -> Vec<u8> {
    let mut map = Map::new();

    let ins = |map: &mut Map<String, Value>, k: &str, v: Value| {
        map.insert(k.to_string(), v);
    };

    ins(&mut map, "total_voltage", json_f2(snapshot.total_voltage_v));
    ins(&mut map, "total_current", json_f3(snapshot.total_current_a));
    ins(&mut map, "total_power", json_f2(snapshot.total_power_w));
    ins(
        &mut map,
        "state_of_charge",
        Value::Number(snapshot.soc_pct.into()),
    );
    ins(
        &mut map,
        "state_of_health",
        Value::Number(snapshot.soh_pct.into()),
    );
    ins(
        &mut map,
        "capacity_remaining",
        json_f1(snapshot.capacity_remaining_ah),
    );
    ins(
        &mut map,
        "total_cycle_capacity",
        json_f1(snapshot.total_cycle_capacity_ah),
    );
    ins(
        &mut map,
        "battery_capacity_ah",
        json_f1(snapshot.battery_capacity_ah),
    );
    ins(
        &mut map,
        "charging_cycles",
        Value::Number(snapshot.charging_cycles.into()),
    );
    ins(
        &mut map,
        "total_runtime_seconds",
        Value::Number(snapshot.total_runtime_s.into()),
    );
    ins(
        &mut map,
        "total_runtime",
        Value::String(snapshot.total_runtime.clone()),
    );

    for (i, &v) in snapshot.cell_voltages_v.iter().enumerate() {
        ins(&mut map, &format!("cell_{}_voltage", i + 1), json_f3(v));
    }
    for (i, &r) in snapshot.cell_resistances_ohm.iter().enumerate() {
        ins(&mut map, &format!("cell_{}_resistance", i + 1), json_f3(r));
    }

    if let Some(ref agg) = snapshot.cell_aggregates {
        ins(&mut map, "cell_voltage_average", json_f3(agg.average_v));
        ins(&mut map, "cell_voltage_min", json_f3(agg.min_v));
        ins(&mut map, "cell_voltage_max", json_f3(agg.max_v));
        ins(&mut map, "cell_voltage_delta", json_f3(agg.delta_v));
        ins(
            &mut map,
            "cell_voltage_min_cell",
            Value::Number(agg.min_cell.into()),
        );
        ins(
            &mut map,
            "cell_voltage_max_cell",
            Value::Number(agg.max_cell.into()),
        );
    }

    ins(
        &mut map,
        "mos_temperature",
        json_f1(snapshot.mos_temperature_c),
    );
    ins(
        &mut map,
        "temperature_sensor_1",
        json_f1(snapshot.temperature_sensor_1_c),
    );
    ins(
        &mut map,
        "temperature_sensor_2",
        json_f1(snapshot.temperature_sensor_2_c),
    );
    ins(
        &mut map,
        "temperature_sensor_4",
        json_f1(snapshot.temperature_sensor_4_c),
    );
    ins(
        &mut map,
        "temperature_sensor_5",
        json_f1(snapshot.temperature_sensor_5_c),
    );

    ins(
        &mut map,
        "balancing_current",
        json_f3(snapshot.balancing_current_a),
    );
    ins(
        &mut map,
        "balancing_active",
        Value::String(on_off(snapshot.balancing_active).to_string()),
    );
    ins(
        &mut map,
        "charging",
        Value::String(on_off(snapshot.charging_switch).to_string()),
    );
    ins(
        &mut map,
        "balancing",
        Value::String(on_off(snapshot.balance_switch).to_string()),
    );
    ins(
        &mut map,
        "alarm_list",
        Value::String(snapshot.alarm_list.clone()),
    );
    ins(
        &mut map,
        "power_cycle_count",
        Value::Number(snapshot.power_cycle_count.into()),
    );
    ins(
        &mut map,
        "last_update_age",
        Value::Number(snapshot.last_update_age_s.into()),
    );
    ins(
        &mut map,
        "jkbms_reconnect_count",
        Value::Number(snapshot.jkbms_reconnect_count.into()),
    );
    ins(
        &mut map,
        "mqtt_reconnect_count",
        Value::Number(snapshot.mqtt_reconnect_count.into()),
    );

    serde_json::to_vec(&Value::Object(map)).unwrap()
}

fn on_off(v: bool) -> &'static str {
    if v { "ON" } else { "OFF" }
}

fn json_f1(v: f64) -> Value {
    Value::String(format!("{:.1}", v))
}

fn json_f2(v: f64) -> Value {
    Value::String(format!("{:.2}", v))
}

fn json_f3(v: f64) -> Value {
    Value::String(format!("{:.3}", v))
}
