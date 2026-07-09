use serde_json::{Value, json};

use crate::jkbms::JkBmsDeviceInfo;

use super::topics;

pub fn build_payloads(
    device_info: &JkBmsDeviceInfo,
    cell_count: u32,
    bms_name: &str,
    discovery_prefix: &str,
) -> Vec<(String, Vec<u8>)> {
    let device = device_block(device_info, bms_name);
    let avail = topics::availability_topic(bms_name);
    let mut out = Vec::new();

    let mut add = |component: &str, entity_id: &str, payload: Value| {
        let topic = topics::discovery_topic(discovery_prefix, component, bms_name, entity_id);
        out.push((topic, serde_json::to_vec(&payload).unwrap()));
    };

    // ── Pack sensors ────────────────────────────────────────────────────────
    add(
        "sensor",
        "total_voltage",
        sensor(
            "Total Voltage",
            bms_name,
            "total_voltage",
            &avail,
            &device,
            Some("voltage"),
            Some("measurement"),
            Some("V"),
            None,
            Some(2),
        ),
    );
    add(
        "sensor",
        "total_current",
        sensor(
            "Total Current",
            bms_name,
            "total_current",
            &avail,
            &device,
            Some("current"),
            Some("measurement"),
            Some("A"),
            None,
            Some(3),
        ),
    );
    add(
        "sensor",
        "total_power",
        sensor(
            "Total Power",
            bms_name,
            "total_power",
            &avail,
            &device,
            Some("power"),
            Some("measurement"),
            Some("W"),
            None,
            Some(2),
        ),
    );
    add(
        "sensor",
        "state_of_charge",
        sensor(
            "State of Charge",
            bms_name,
            "state_of_charge",
            &avail,
            &device,
            Some("battery"),
            Some("measurement"),
            Some("%"),
            None,
            None,
        ),
    );
    add(
        "sensor",
        "state_of_health",
        sensor(
            "State of Health",
            bms_name,
            "state_of_health",
            &avail,
            &device,
            Some("battery"),
            Some("measurement"),
            Some("%"),
            None,
            None,
        ),
    );
    add(
        "sensor",
        "capacity_remaining",
        sensor(
            "Capacity Remaining",
            bms_name,
            "capacity_remaining",
            &avail,
            &device,
            None,
            Some("measurement"),
            Some("Ah"),
            None,
            Some(1),
        ),
    );
    add(
        "sensor",
        "power_cycle_count",
        sensor(
            "Power Cycle Count",
            bms_name,
            "power_cycle_count",
            &avail,
            &device,
            None,
            Some("total_increasing"),
            None,
            None,
            None,
        ),
    );
    add(
        "sensor",
        "total_cycle_capacity",
        sensor(
            "Total Cycle Capacity",
            bms_name,
            "total_cycle_capacity",
            &avail,
            &device,
            None,
            Some("total_increasing"),
            Some("Ah"),
            None,
            Some(1),
        ),
    );
    add(
        "sensor",
        "battery_capacity_ah",
        sensor(
            "Battery Capacity",
            bms_name,
            "battery_capacity_ah",
            &avail,
            &device,
            None,
            Some("measurement"),
            Some("Ah"),
            None,
            Some(1),
        ),
    );
    add(
        "sensor",
        "charging_cycles",
        sensor(
            "Charging Cycles",
            bms_name,
            "charging_cycles",
            &avail,
            &device,
            None,
            Some("total_increasing"),
            None,
            None,
            None,
        ),
    );
    add(
        "sensor",
        "total_runtime_seconds",
        sensor(
            "Total Runtime Seconds",
            bms_name,
            "total_runtime_seconds",
            &avail,
            &device,
            Some("duration"),
            Some("total_increasing"),
            Some("s"),
            None,
            Some(0),
        ),
    );
    add(
        "sensor",
        "total_runtime",
        sensor(
            "Total Runtime",
            bms_name,
            "total_runtime",
            &avail,
            &device,
            None,
            None,
            None,
            None,
            None,
        ),
    );

    // ── Cell sensors ────────────────────────────────────────────────────────
    for n in 1..=cell_count {
        let entity_id = format!("cell_{}_voltage", n);
        let name = format!("Cell {} Voltage", n);
        add(
            "sensor",
            &entity_id,
            sensor(
                &name,
                bms_name,
                &entity_id,
                &avail,
                &device,
                Some("voltage"),
                Some("measurement"),
                Some("V"),
                None,
                Some(3),
            ),
        );

        let entity_id = format!("cell_{}_resistance", n);
        let name = format!("Cell {} Resistance", n);
        add(
            "sensor",
            &entity_id,
            sensor(
                &name,
                bms_name,
                &entity_id,
                &avail,
                &device,
                None,
                Some("measurement"),
                Some("Ω"),
                None,
                Some(3),
            ),
        );
    }

    // ── Cell aggregate sensors ───────────────────────────────────────────────
    add(
        "sensor",
        "cell_voltage_average",
        sensor(
            "Cell Voltage Average",
            bms_name,
            "cell_voltage_average",
            &avail,
            &device,
            Some("voltage"),
            Some("measurement"),
            Some("V"),
            None,
            Some(3),
        ),
    );
    add(
        "sensor",
        "cell_voltage_min",
        sensor(
            "Cell Voltage Min",
            bms_name,
            "cell_voltage_min",
            &avail,
            &device,
            Some("voltage"),
            Some("measurement"),
            Some("V"),
            None,
            Some(3),
        ),
    );
    add(
        "sensor",
        "cell_voltage_max",
        sensor(
            "Cell Voltage Max",
            bms_name,
            "cell_voltage_max",
            &avail,
            &device,
            Some("voltage"),
            Some("measurement"),
            Some("V"),
            None,
            Some(3),
        ),
    );
    add(
        "sensor",
        "cell_voltage_delta",
        sensor(
            "Cell Voltage Delta",
            bms_name,
            "cell_voltage_delta",
            &avail,
            &device,
            Some("voltage"),
            Some("measurement"),
            Some("V"),
            None,
            Some(3),
        ),
    );
    add(
        "sensor",
        "cell_voltage_min_cell",
        sensor(
            "Cell Voltage Min Cell",
            bms_name,
            "cell_voltage_min_cell",
            &avail,
            &device,
            None,
            None,
            None,
            None,
            None,
        ),
    );
    add(
        "sensor",
        "cell_voltage_max_cell",
        sensor(
            "Cell Voltage Max Cell",
            bms_name,
            "cell_voltage_max_cell",
            &avail,
            &device,
            None,
            None,
            None,
            None,
            None,
        ),
    );

    // ── Temperature sensors ──────────────────────────────────────────────────
    for (entity_id, name) in [
        ("mos_temperature", "MOS Temperature"),
        ("temperature_sensor_1", "Temperature Sensor 1"),
        ("temperature_sensor_2", "Temperature Sensor 2"),
        ("temperature_sensor_4", "Temperature Sensor 4"),
        ("temperature_sensor_5", "Temperature Sensor 5"),
    ] {
        add(
            "sensor",
            entity_id,
            sensor(
                name,
                bms_name,
                entity_id,
                &avail,
                &device,
                Some("temperature"),
                Some("measurement"),
                Some("°C"),
                None,
                Some(1),
            ),
        );
    }

    // ── Balancer ─────────────────────────────────────────────────────────────
    add(
        "sensor",
        "balancing_current",
        sensor(
            "Balancing Current",
            bms_name,
            "balancing_current",
            &avail,
            &device,
            Some("current"),
            Some("measurement"),
            Some("A"),
            None,
            Some(3),
        ),
    );
    add(
        "binary_sensor",
        "balancing_active",
        binary_sensor(
            "Balancing Active",
            bms_name,
            "balancing_active",
            &avail,
            &device,
        ),
    );

    // ── Alarm sensor ─────────────────────────────────────────────────────────
    add(
        "sensor",
        "alarm_list",
        sensor(
            "Alarm List",
            bms_name,
            "alarm_list",
            &avail,
            &device,
            None,
            None,
            None,
            None,
            None,
        ),
    );

    // ── Diagnostic sensors ───────────────────────────────────────────────────
    add(
        "sensor",
        "last_update_age",
        sensor(
            "Last Update Age",
            bms_name,
            "last_update_age",
            &avail,
            &device,
            Some("duration"),
            Some("measurement"),
            Some("s"),
            Some("diagnostic"),
            None,
        ),
    );
    add(
        "sensor",
        "jkbms_reconnect_count",
        sensor(
            "JK-BMS Reconnect Count",
            bms_name,
            "jkbms_reconnect_count",
            &avail,
            &device,
            None,
            Some("total_increasing"),
            None,
            Some("diagnostic"),
            None,
        ),
    );
    add(
        "sensor",
        "mqtt_reconnect_count",
        sensor(
            "MQTT Reconnect Count",
            bms_name,
            "mqtt_reconnect_count",
            &avail,
            &device,
            None,
            Some("total_increasing"),
            None,
            Some("diagnostic"),
            None,
        ),
    );

    // ── Switches ─────────────────────────────────────────────────────────────
    add(
        "switch",
        "charging",
        switch_payload("Charging", bms_name, "charging", &avail, &device),
    );
    add(
        "switch",
        "balancing",
        switch_payload("Balancing", bms_name, "balancing", &avail, &device),
    );

    out
}

fn device_block(info: &JkBmsDeviceInfo, bms_name: &str) -> Value {
    json!({
        "name": bms_name,
        "manufacturer": "JIKONG",
        "model": info.model,
        "hw_version": info.hardware_version,
        "sw_version": info.software_version,
        "serial_number": info.serial_number,
        "identifiers": [bms_name],
    })
}

fn availability_block(avail_topic: &str) -> Value {
    json!({
        "availability_topic": avail_topic,
        "payload_available": "online",
        "payload_not_available": "offline",
    })
}

#[allow(clippy::too_many_arguments)]
fn sensor(
    name: &str,
    bms_name: &str,
    entity_id: &str,
    avail_topic: &str,
    device: &Value,
    device_class: Option<&str>,
    state_class: Option<&str>,
    unit: Option<&str>,
    entity_category: Option<&str>,
    suggested_display_precision: Option<u8>,
) -> Value {
    let avail = availability_block(avail_topic);
    let mut payload = json!({
        "name": name,
        "object_id": format!("{}_{}", bms_name, entity_id),
        "unique_id": format!("{}_{}", bms_name, entity_id),
        "state_topic": topics::state_topic(bms_name, entity_id),
        "availability_topic": avail["availability_topic"],
        "payload_available": avail["payload_available"],
        "payload_not_available": avail["payload_not_available"],
        "device": device,
    });
    if let Some(dc) = device_class {
        payload["device_class"] = json!(dc);
    }
    if let Some(sc) = state_class {
        payload["state_class"] = json!(sc);
    }
    if let Some(u) = unit {
        payload["unit_of_measurement"] = json!(u);
    }
    if let Some(ec) = entity_category {
        payload["entity_category"] = json!(ec);
    }
    if let Some(sdp) = suggested_display_precision {
        payload["suggested_display_precision"] = json!(sdp);
    }
    payload
}

fn binary_sensor(
    name: &str,
    bms_name: &str,
    entity_id: &str,
    avail_topic: &str,
    device: &Value,
) -> Value {
    let avail = availability_block(avail_topic);
    json!({
        "name": name,
        "object_id": format!("{}_{}", bms_name, entity_id),
        "unique_id": format!("{}_{}", bms_name, entity_id),
        "state_topic": topics::state_topic(bms_name, entity_id),
        "payload_on": "ON",
        "payload_off": "OFF",
        "availability_topic": avail["availability_topic"],
        "payload_available": avail["payload_available"],
        "payload_not_available": avail["payload_not_available"],
        "device": device,
    })
}

fn switch_payload(
    name: &str,
    bms_name: &str,
    entity_id: &str,
    avail_topic: &str,
    device: &Value,
) -> Value {
    let avail = availability_block(avail_topic);
    json!({
        "name": name,
        "object_id": format!("{}_{}", bms_name, entity_id),
        "unique_id": format!("{}_{}", bms_name, entity_id),
        "state_topic": topics::state_topic(bms_name, entity_id),
        "command_topic": topics::set_topic(bms_name, entity_id),
        "payload_on": "ON",
        "payload_off": "OFF",
        "state_on": "ON",
        "state_off": "OFF",
        "availability_topic": avail["availability_topic"],
        "payload_available": avail["payload_available"],
        "payload_not_available": avail["payload_not_available"],
        "device": device,
    })
}
