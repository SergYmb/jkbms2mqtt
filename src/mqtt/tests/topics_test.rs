use crate::mqtt::topics;

const BMS: &str = "my_jk_bms";
const PREFIX: &str = "homeassistant";

#[test]
fn state_topic() {
    assert_eq!(
        topics::state_topic(BMS, "total_voltage"),
        "my_jk_bms/total_voltage/state"
    );
}

#[test]
fn set_topic() {
    assert_eq!(topics::set_topic(BMS, "charging"), "my_jk_bms/charging/set");
}

#[test]
fn availability_topic() {
    assert_eq!(topics::availability_topic(BMS), "my_jk_bms/availability");
}

#[test]
fn discovery_topic() {
    assert_eq!(
        topics::discovery_topic(PREFIX, "sensor", BMS, "total_voltage"),
        "homeassistant/sensor/my_jk_bms/total_voltage/config"
    );
    assert_eq!(
        topics::discovery_topic(PREFIX, "switch", BMS, "charging"),
        "homeassistant/switch/my_jk_bms/charging/config"
    );
    assert_eq!(
        topics::discovery_topic(PREFIX, "binary_sensor", BMS, "balancing_active"),
        "homeassistant/binary_sensor/my_jk_bms/balancing_active/config"
    );
}
