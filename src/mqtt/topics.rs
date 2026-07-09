pub fn state_topic(bms_name: &str, entity_id: &str) -> String {
    format!("{}/{}/state", bms_name, entity_id)
}

pub fn set_topic(bms_name: &str, entity_id: &str) -> String {
    format!("{}/{}/set", bms_name, entity_id)
}

pub fn availability_topic(bms_name: &str) -> String {
    format!("{}/availability", bms_name)
}

pub fn discovery_topic(
    discovery_prefix: &str,
    component: &str,
    bms_name: &str,
    object_id: &str,
) -> String {
    format!(
        "{}/{}/{}/{}/config",
        discovery_prefix, component, bms_name, object_id
    )
}
