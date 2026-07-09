use crate::jkbms::JkBmsDeviceInfo;
use crate::mqtt::discovery::build_payloads;

const BMS: &str = "my_jk_bms";
const PREFIX: &str = "homeassistant";

fn make_device_info() -> JkBmsDeviceInfo {
    JkBmsDeviceInfo {
        model: "JK_PB2A16S20P".to_string(),
        hardware_version: "15A".to_string(),
        software_version: "15.41".to_string(),
        serial_number: "REDACTED".to_string(),
        power_cycle_count: 39,
    }
}

fn payload_for(pubs: &[(String, Vec<u8>)], component: &str, entity_id: &str) -> serde_json::Value {
    let topic = format!("{}/{}/{}/{}/config", PREFIX, component, BMS, entity_id);
    let bytes = pubs
        .iter()
        .find(|(t, _)| *t == topic)
        .map(|(_, p)| p.as_slice())
        .unwrap_or_else(|| panic!("no discovery payload for {}/{}", component, entity_id));
    serde_json::from_slice(bytes).unwrap()
}

fn has_topic(pubs: &[(String, Vec<u8>)], needle: &str) -> bool {
    pubs.iter().any(|(t, _)| t.contains(needle))
}

#[test]
fn sensor_total_voltage() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "sensor", "total_voltage");

    insta::assert_json_snapshot!(p, {
        ".device.serial_number" => "[redacted]"
    });
}

#[test]
fn binary_sensor_balancing_active() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "binary_sensor", "balancing_active");

    assert_eq!(p["payload_on"], "ON");
    assert_eq!(p["payload_off"], "OFF");
    assert_eq!(p["state_topic"], "my_jk_bms/balancing_active/state");
    assert_eq!(p["unique_id"], "my_jk_bms_balancing_active");
    assert_eq!(p["device"]["manufacturer"], "JIKONG");
}

#[test]
fn switch_charging() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "switch", "charging");

    assert_eq!(p["command_topic"], "my_jk_bms/charging/set");
    assert_eq!(p["state_topic"], "my_jk_bms/charging/state");
    assert_eq!(p["payload_on"], "ON");
    assert_eq!(p["payload_off"], "OFF");
    assert_eq!(p["unique_id"], "my_jk_bms_charging");
}

#[test]
fn power_cycle_count_not_diagnostic() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "sensor", "power_cycle_count");

    assert!(p.get("entity_category").is_none() || p["entity_category"].is_null());
    assert_eq!(p["state_class"], "total_increasing");
    assert!(p.get("device_class").is_none() || p["device_class"].is_null());
}

#[test]
fn diagnostic_last_update_age() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "sensor", "last_update_age");

    assert_eq!(p["entity_category"], "diagnostic");
    assert_eq!(p["device_class"], "duration");
    assert_eq!(p["state_class"], "measurement");
    assert_eq!(p["unit_of_measurement"], "s");
}

#[test]
fn diagnostic_jkbms_reconnect_count() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "sensor", "jkbms_reconnect_count");

    assert_eq!(p["entity_category"], "diagnostic");
    assert_eq!(p["state_class"], "total_increasing");
}

#[test]
fn diagnostic_mqtt_reconnect_count() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "sensor", "mqtt_reconnect_count");

    assert_eq!(p["entity_category"], "diagnostic");
    assert_eq!(p["state_class"], "total_increasing");
}

#[test]
fn cell_count_drives_cell_entities() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);

    // Exactly 8 cell_voltage + 8 cell_resistance discovery payloads
    let cell_voltage_count = pubs
        .iter()
        .filter(|(t, _)| {
            t.starts_with(&format!("{}/sensor/{}/cell_", PREFIX, BMS))
                && t.contains("_voltage/config")
        })
        .count();
    let cell_resistance_count = pubs
        .iter()
        .filter(|(t, _)| {
            t.starts_with(&format!("{}/sensor/{}/cell_", PREFIX, BMS))
                && t.contains("_resistance/config")
        })
        .count();

    assert_eq!(cell_voltage_count, 8, "expected 8 cell voltage entities");
    assert_eq!(
        cell_resistance_count, 8,
        "expected 8 cell resistance entities"
    );

    // Cell 9 must NOT be present
    assert!(!has_topic(&pubs, "cell_9_voltage"));
    assert!(!has_topic(&pubs, "cell_9_resistance"));
}

#[test]
fn cell_count_16_has_16_cells() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 16, BMS, PREFIX);

    let cell_voltage_count = pubs
        .iter()
        .filter(|(t, _)| {
            t.starts_with(&format!("{}/sensor/{}/cell_", PREFIX, BMS))
                && t.contains("_voltage/config")
        })
        .count();
    assert_eq!(cell_voltage_count, 16);
}

#[test]
fn device_block_fields() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);
    let p = payload_for(&pubs, "sensor", "total_voltage");

    assert_eq!(p["device"]["manufacturer"], "JIKONG");
    assert_eq!(p["device"]["model"], "JK_PB2A16S20P");
    assert_eq!(p["device"]["hw_version"], "15A");
    assert_eq!(p["device"]["sw_version"], "15.41");
    assert_eq!(p["device"]["identifiers"][0], BMS);
}

#[test]
fn availability_block_on_all_payloads() {
    let info = make_device_info();
    let pubs = build_payloads(&info, 8, BMS, PREFIX);

    for (topic, bytes) in &pubs {
        let p: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(
            p["availability_topic"], "my_jk_bms/availability",
            "missing availability_topic on {}",
            topic
        );
        assert_eq!(p["payload_available"], "online", "on {}", topic);
        assert_eq!(p["payload_not_available"], "offline", "on {}", topic);
    }
}
